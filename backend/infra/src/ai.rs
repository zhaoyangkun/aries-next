//! AI Provider 协议实现：OpenAI 兼容（`/chat/completions`）与 Anthropic Messages API。
//! api_key 只允许进入鉴权 Header，禁止写入日志与错误消息。
//! `DispatchingAiProvider` 按 settings.protocol 分发，调用方无需关心协议差异。

use std::time::Duration;

use aries_core::ai::{AiChatRequest, AiError, AiProvider, AiSettings, AiStreamEvent, AiUsage};
use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::mpsc;

/// 非流式请求整体超时；评论审核等场景由调用方再包更短的超时。
const CHAT_TIMEOUT: Duration = Duration::from_secs(60);
/// 流式首字节超时：Provider 建连后迟迟不出 token 视为不可用。
const STREAM_FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(30);
/// Anthropic Messages API 版本头（固定，随协议升级再提）。
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Anthropic 官方端点（base_url 缺省时使用）。
const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Anthropic 强制要求 max_tokens，调用方未指定时的兜底值。
const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 1024;
/// 拉取模型列表的短超时：管理端手动触发，Provider 不应让用户久等。
const LIST_MODELS_TIMEOUT: Duration = Duration::from_secs(15);
/// 模型列表上限：防止异常 Provider 返回超长列表撑爆响应。
const LIST_MODELS_MAX: usize = 500;

fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(CHAT_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 把 reqwest 错误映射为领域错误；不携带 URL/Body，避免 Secret 泄露。
fn map_reqwest(error: reqwest::Error) -> AiError {
    if error.is_timeout() {
        return AiError::Timeout;
    }
    if error.is_status() {
        if error.status() == Some(reqwest::StatusCode::TOO_MANY_REQUESTS) {
            return AiError::RateLimited;
        }
        return AiError::ProviderFailed;
    }
    tracing::warn!(error = %error, "ai provider request failed");
    AiError::ProviderFailed
}

/// 有限重试：仅对连接级错误重试 1 次（幂等的生成请求，Provider 抖动时有用）。
async fn send_with_retry(builder: reqwest::RequestBuilder) -> Result<reqwest::Response, AiError> {
    let first = builder.try_clone().ok_or(AiError::ProviderFailed)?;
    match builder.send().await {
        Ok(response) => Ok(response),
        Err(error) if error.is_connect() => first.send().await.map_err(map_reqwest),
        Err(error) => Err(map_reqwest(error)),
    }
}

fn ensure_success(response: &reqwest::Response) -> Result<(), AiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AiError::RateLimited);
    }
    tracing::warn!(status = %status, "ai provider returned non-success status");
    Err(AiError::ProviderFailed)
}

// ============================================================
// 共享 SSE 泵：逐行读取，event:/data: 分派给协议解析器
// ============================================================

/// 一行 SSE 数据的解析结果。
#[derive(Debug)]
enum SseLineAction {
    Delta(String),
    Usage(AiUsage),
    Done,
    Failed(AiError),
}

/// 共享 SSE 读取循环：`event:` 行记录事件名，`data:` 行交给协议解析器。
/// 首字节受单独超时约束；流结束而未收到 Done 按 ProviderFailed 处理（调用方落 failed 审计）。
async fn pump_sse(
    response: reqwest::Response,
    sender: &mpsc::Sender<AiStreamEvent>,
    mut parse: impl FnMut(Option<&str>, &str) -> Option<SseLineAction>,
) -> Result<(), AiError> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut event_name: Option<String> = None;
    let mut first_byte_deadline = Some(tokio::time::Instant::now() + STREAM_FIRST_BYTE_TIMEOUT);
    loop {
        let next = match first_byte_deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, stream.next()).await {
                Ok(item) => item,
                Err(_) => return Err(AiError::Timeout),
            },
            None => stream.next().await,
        };
        let Some(item) = next else { break };
        first_byte_deadline = None;
        let bytes = item.map_err(map_reqwest)?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_owned();
            buffer.drain(..=line_end);
            if let Some(name) = line.strip_prefix("event:") {
                event_name = Some(name.trim().to_owned());
                continue;
            }
            let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                continue;
            };
            match parse(event_name.as_deref(), data) {
                Some(SseLineAction::Delta(delta)) => {
                    if !delta.is_empty() {
                        let _ = sender.send(AiStreamEvent::Delta(delta)).await;
                    }
                }
                Some(SseLineAction::Usage(usage)) => {
                    let _ = sender.send(AiStreamEvent::Usage(usage)).await;
                }
                Some(SseLineAction::Done) => {
                    let _ = sender.send(AiStreamEvent::Done).await;
                    return Ok(());
                }
                Some(SseLineAction::Failed(error)) => return Err(error),
                None => {}
            }
        }
    }
    Err(AiError::ProviderFailed)
}

fn token_count(value: Option<&serde_json::Value>) -> Option<i32> {
    value
        .and_then(serde_json::Value::as_i64)
        .map(|v| i32::try_from(v).unwrap_or(i32::MAX))
}

// ============================================================
// OpenAI 兼容协议
// ============================================================

#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new() -> Self {
        Self {
            client: build_http_client(),
        }
    }

    /// 组装请求体；api_key 仅出现在 Authorization Header。
    fn build_request(
        &self,
        settings: &AiSettings,
        request: &AiChatRequest,
        stream: bool,
    ) -> reqwest::RequestBuilder {
        let base_url = settings
            .base_url
            .as_deref()
            .unwrap_or_default()
            .trim_end_matches('/');
        let body = serde_json::json!({
            "model": settings.model.as_deref().unwrap_or_default(),
            "messages": request.messages,
            "stream": stream,
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
        });
        self.client
            .post(format!("{base_url}/chat/completions"))
            .bearer_auth(settings.api_key.as_deref().unwrap_or_default())
            .json(&body)
    }
}

impl Default for OpenAiCompatibleProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_openai_usage(value: &serde_json::Value) -> Option<AiUsage> {
    let usage = value.get("usage")?;
    Some(AiUsage {
        prompt_tokens: token_count(usage.get("prompt_tokens")),
        completion_tokens: token_count(usage.get("completion_tokens")),
    })
}

/// OpenAI SSE 行解析：`data: [DONE]` 结束，chunk 里取 usage 与增量。
fn parse_openai_line(_event: Option<&str>, data: &str) -> Option<SseLineAction> {
    if data == "[DONE]" {
        return Some(SseLineAction::Done);
    }
    let chunk = serde_json::from_str::<serde_json::Value>(data).ok()?;
    if let Some(usage) = parse_openai_usage(&chunk) {
        return Some(SseLineAction::Usage(usage));
    }
    chunk
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(serde_json::Value::as_str)
        .map(|text| SseLineAction::Delta(text.to_owned()))
}

#[async_trait]
impl AiProvider for OpenAiCompatibleProvider {
    async fn chat(
        &self,
        settings: &AiSettings,
        request: AiChatRequest,
    ) -> Result<(String, Option<AiUsage>), AiError> {
        let response = send_with_retry(self.build_request(settings, &request, false)).await?;
        ensure_success(&response)?;
        let body: serde_json::Value = response.json().await.map_err(map_reqwest)?;
        let text = body
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(serde_json::Value::as_str)
            .ok_or(AiError::InvalidOutput)?
            .to_owned();
        Ok((text, parse_openai_usage(&body)))
    }

    async fn chat_stream(
        &self,
        settings: &AiSettings,
        request: AiChatRequest,
        sender: mpsc::Sender<AiStreamEvent>,
    ) -> Result<(), AiError> {
        let response = send_with_retry(self.build_request(settings, &request, true)).await?;
        ensure_success(&response)?;
        pump_sse(response, &sender, parse_openai_line).await
    }

    async fn embed(
        &self,
        settings: &AiSettings,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AiError> {
        let base_url = settings
            .embedding_base_url
            .as_deref()
            .unwrap_or_default()
            .trim_end_matches('/');
        let body = serde_json::json!({
            "model": settings.embedding_model.as_deref().unwrap_or_default(),
            "input": texts,
        });
        let response = send_with_retry(
            self.client
                .post(format!("{base_url}/embeddings"))
                .bearer_auth(settings.api_key.as_deref().unwrap_or_default())
                .json(&body),
        )
        .await?;
        ensure_success(&response)?;
        let body: serde_json::Value = response.json().await.map_err(map_reqwest)?;
        parse_embedding_response(&body, texts.len())
    }

    async fn list_models(&self, settings: &AiSettings) -> Result<Vec<String>, AiError> {
        // OpenAI 兼容协议没有默认端点，base_url 必填。
        let base_url = settings
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(AiError::Unconfigured)?
            .trim_end_matches('/');
        let request = self
            .client
            .get(format!("{base_url}/models"))
            .bearer_auth(settings.api_key.as_deref().unwrap_or_default());
        let response = tokio::time::timeout(LIST_MODELS_TIMEOUT, request.send())
            .await
            .map_err(|_| AiError::Timeout)?
            .map_err(map_reqwest)?;
        ensure_success(&response)?;
        let body: serde_json::Value = response.json().await.map_err(map_reqwest)?;
        Ok(parse_model_list(&body))
    }
}

/// 解析 `/models` 响应：`data` 数组每项取 `id`，去重排序并截断上限。
fn parse_model_list(body: &serde_json::Value) -> Vec<String> {
    let mut models: Vec<String> = body
        .get("data")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(serde_json::Value::as_str))
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    models.sort();
    models.dedup();
    models.truncate(LIST_MODELS_MAX);
    models
}

/// 解析 `/embeddings` 响应：`data` 数组每项取 `embedding`，数量须与输入一致（顺序即输入顺序）。
fn parse_embedding_response(
    body: &serde_json::Value,
    expected_len: usize,
) -> Result<Vec<Vec<f32>>, AiError> {
    let items = body
        .get("data")
        .and_then(serde_json::Value::as_array)
        .filter(|items| items.len() == expected_len)
        .ok_or(AiError::InvalidOutput)?;
    items
        .iter()
        .map(|item| {
            item.get("embedding")
                .and_then(serde_json::Value::as_array)
                .ok_or(AiError::InvalidOutput)?
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .map(|number| number as f32)
                        .ok_or(AiError::InvalidOutput)
                })
                .collect()
        })
        .collect()
}

// ============================================================
// Anthropic Messages API
// ============================================================

#[derive(Clone)]
pub struct AnthropicProvider {
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new() -> Self {
        Self {
            client: build_http_client(),
        }
    }

    fn build_request(
        &self,
        settings: &AiSettings,
        request: &AiChatRequest,
        stream: bool,
    ) -> reqwest::RequestBuilder {
        let base_url = settings
            .base_url
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(ANTHROPIC_DEFAULT_BASE_URL)
            .trim_end_matches('/');
        let body = build_anthropic_body(settings, request, stream);
        self.client
            .post(format!("{base_url}/v1/messages"))
            .header("x-api-key", settings.api_key.as_deref().unwrap_or_default())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
    }
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// 组装 Anthropic 请求体：system 消息拆到顶层 `system` 字段，
/// `messages` 只保留 user/assistant；max_tokens 为协议必填，缺省兜底。
fn build_anthropic_body(
    settings: &AiSettings,
    request: &AiChatRequest,
    stream: bool,
) -> serde_json::Value {
    let system_prompt: Vec<&str> = request
        .messages
        .iter()
        .filter(|message| message.role == "system")
        .map(|message| message.content.as_str())
        .collect();
    let messages: Vec<serde_json::Value> = request
        .messages
        .iter()
        .filter(|message| message.role != "system")
        .map(|message| serde_json::json!({ "role": message.role, "content": message.content }))
        .collect();
    serde_json::json!({
        "model": settings.model.as_deref().unwrap_or_default(),
        "max_tokens": request.max_tokens.unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS),
        "temperature": request.temperature,
        "system": if system_prompt.is_empty() { None } else { Some(system_prompt.join("\n")) },
        "messages": messages,
        "stream": stream,
    })
}

/// 非流式响应解析：`content` 首个 text block + `usage.input/output_tokens`。
fn parse_anthropic_response(
    body: &serde_json::Value,
) -> Result<(String, Option<AiUsage>), AiError> {
    let text = body
        .get("content")
        .and_then(serde_json::Value::as_array)
        .and_then(|blocks| {
            blocks.iter().find_map(|block| {
                (block.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                    .then(|| block.get("text").and_then(serde_json::Value::as_str))
                    .flatten()
            })
        })
        .ok_or(AiError::InvalidOutput)?
        .to_owned();
    let usage = body.get("usage").map(|usage| AiUsage {
        prompt_tokens: token_count(usage.get("input_tokens")),
        completion_tokens: token_count(usage.get("output_tokens")),
    });
    Ok((text, usage))
}

/// Anthropic SSE 行解析器：input_tokens 在 message_start 先到，output_tokens 在
/// message_delta 累积；合并为单个 Usage 事件发出（与 OpenAI 适配器语义一致）。
#[derive(Default)]
struct AnthropicStreamParser {
    input_tokens: Option<i32>,
}

impl AnthropicStreamParser {
    fn parse_line(&mut self, event: Option<&str>, data: &str) -> Option<SseLineAction> {
        let payload = serde_json::from_str::<serde_json::Value>(data).ok()?;
        match event? {
            "message_start" => {
                self.input_tokens = payload
                    .get("message")
                    .and_then(|message| message.get("usage"))
                    .and_then(|usage| token_count(usage.get("input_tokens")));
                None
            }
            "content_block_delta" => payload
                .get("delta")
                .and_then(|delta| delta.get("text"))
                .and_then(serde_json::Value::as_str)
                .map(|text| SseLineAction::Delta(text.to_owned())),
            "message_delta" => {
                let output = payload
                    .get("usage")
                    .and_then(|usage| token_count(usage.get("output_tokens")));
                output.map(|completion_tokens| {
                    SseLineAction::Usage(AiUsage {
                        prompt_tokens: self.input_tokens,
                        completion_tokens: Some(completion_tokens),
                    })
                })
            }
            "message_stop" => Some(SseLineAction::Done),
            "error" => Some(SseLineAction::Failed(AiError::ProviderFailed)),
            // ping / content_block_start/stop 等事件无业务语义。
            _ => None,
        }
    }
}

#[async_trait]
impl AiProvider for AnthropicProvider {
    async fn chat(
        &self,
        settings: &AiSettings,
        request: AiChatRequest,
    ) -> Result<(String, Option<AiUsage>), AiError> {
        let response = send_with_retry(self.build_request(settings, &request, false)).await?;
        ensure_success(&response)?;
        let body: serde_json::Value = response.json().await.map_err(map_reqwest)?;
        parse_anthropic_response(&body)
    }

    async fn chat_stream(
        &self,
        settings: &AiSettings,
        request: AiChatRequest,
        sender: mpsc::Sender<AiStreamEvent>,
    ) -> Result<(), AiError> {
        let response = send_with_retry(self.build_request(settings, &request, true)).await?;
        ensure_success(&response)?;
        let mut parser = AnthropicStreamParser::default();
        pump_sse(response, &sender, |event, data| {
            parser.parse_line(event, data)
        })
        .await
    }

    async fn embed(
        &self,
        _settings: &AiSettings,
        _texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AiError> {
        // Embedding 端点为 OpenAI 兼容协议（embedding_base_url），Anthropic 适配器不支持；
        // DispatchingAiProvider 始终把 embed 路由到 OpenAI 适配器，这里仅是契约兜底。
        tracing::warn!("anthropic provider does not support embeddings");
        Err(AiError::ProviderFailed)
    }

    async fn list_models(&self, settings: &AiSettings) -> Result<Vec<String>, AiError> {
        let base_url = settings
            .base_url
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(ANTHROPIC_DEFAULT_BASE_URL)
            .trim_end_matches('/');
        let request = self
            .client
            .get(format!("{base_url}/v1/models"))
            .header("x-api-key", settings.api_key.as_deref().unwrap_or_default())
            .header("anthropic-version", ANTHROPIC_VERSION);
        let response = tokio::time::timeout(LIST_MODELS_TIMEOUT, request.send())
            .await
            .map_err(|_| AiError::Timeout)?
            .map_err(map_reqwest)?;
        ensure_success(&response)?;
        let body: serde_json::Value = response.json().await.map_err(map_reqwest)?;
        Ok(parse_model_list(&body))
    }
}

// ============================================================
// 协议分发器：按 settings.protocol 选择适配器
// ============================================================

/// 双协议分发 Provider：配置（含 protocol）随请求传入，settings 更新后下一次请求即生效。
#[derive(Clone, Default)]
pub struct DispatchingAiProvider {
    openai: OpenAiCompatibleProvider,
    anthropic: AnthropicProvider,
}

impl DispatchingAiProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AiProvider for DispatchingAiProvider {
    async fn chat(
        &self,
        settings: &AiSettings,
        request: AiChatRequest,
    ) -> Result<(String, Option<AiUsage>), AiError> {
        match settings.protocol {
            aries_core::ai::AiProtocol::OpenAi => self.openai.chat(settings, request).await,
            aries_core::ai::AiProtocol::Anthropic => self.anthropic.chat(settings, request).await,
        }
    }

    async fn chat_stream(
        &self,
        settings: &AiSettings,
        request: AiChatRequest,
        sender: mpsc::Sender<AiStreamEvent>,
    ) -> Result<(), AiError> {
        match settings.protocol {
            aries_core::ai::AiProtocol::OpenAi => {
                self.openai.chat_stream(settings, request, sender).await
            }
            aries_core::ai::AiProtocol::Anthropic => {
                self.anthropic.chat_stream(settings, request, sender).await
            }
        }
    }

    async fn embed(
        &self,
        settings: &AiSettings,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AiError> {
        // embedding_base_url 固定为 OpenAI 兼容协议，与 chat 的 protocol 无关，
        // 因此 embed 始终路由到 OpenAI 适配器（Anthropic 适配器不支持 embed）。
        self.openai.embed(settings, texts).await
    }

    async fn list_models(&self, settings: &AiSettings) -> Result<Vec<String>, AiError> {
        match settings.protocol {
            aries_core::ai::AiProtocol::OpenAi => self.openai.list_models(settings).await,
            aries_core::ai::AiProtocol::Anthropic => self.anthropic.list_models(settings).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use aries_core::ai::{AiMessage, AiSettings};

    use super::*;

    fn anthropic_settings() -> AiSettings {
        AiSettings {
            model: Some("claude-test".to_owned()),
            ..AiSettings::default()
        }
    }

    fn sample_request() -> AiChatRequest {
        AiChatRequest {
            messages: vec![
                AiMessage::system("你是助手"),
                AiMessage::user("你好"),
                AiMessage {
                    role: "assistant".to_owned(),
                    content: "你好！".to_owned(),
                },
                AiMessage::user("改写这段"),
            ],
            temperature: Some(0.5),
            max_tokens: None,
        }
    }

    #[test]
    fn anthropic_body_splits_system_and_defaults_max_tokens() {
        let body = build_anthropic_body(&anthropic_settings(), &sample_request(), true);
        assert_eq!(body["model"], "claude-test");
        // system 消息拆到顶层，messages 不含 system。
        assert_eq!(body["system"], "你是助手");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert!(messages.iter().all(|m| m["role"] != "system"));
        assert_eq!(messages[0]["role"], "user");
        // max_tokens 协议必填，缺省兜底 1024。
        assert_eq!(body["max_tokens"], ANTHROPIC_DEFAULT_MAX_TOKENS);
        assert_eq!(body["stream"], true);
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn anthropic_response_parses_text_and_usage() {
        let body = serde_json::json!({
            "content": [
                { "type": "text", "text": "改写结果" }
            ],
            "usage": { "input_tokens": 42, "output_tokens": 7 }
        });
        let (text, usage) = parse_anthropic_response(&body).unwrap();
        assert_eq!(text, "改写结果");
        assert_eq!(
            usage,
            Some(AiUsage {
                prompt_tokens: Some(42),
                completion_tokens: Some(7)
            })
        );

        // 没有 text block → InvalidOutput。
        let bad = serde_json::json!({ "content": [], "usage": {} });
        assert!(matches!(
            parse_anthropic_response(&bad),
            Err(AiError::InvalidOutput)
        ));
    }

    #[test]
    fn anthropic_sse_events_map_to_shared_stream_semantics() {
        let mut parser = AnthropicStreamParser::default();

        // message_start 只记录 input_tokens，不产生事件。
        let action = parser.parse_line(
            Some("message_start"),
            r#"{"message": {"usage": {"input_tokens": 25}}}"#,
        );
        assert!(action.is_none());

        // content_block_delta → Delta。
        match parser.parse_line(
            Some("content_block_delta"),
            r#"{"delta": {"type": "text_delta", "text": "你好"}}"#,
        ) {
            Some(SseLineAction::Delta(text)) => assert_eq!(text, "你好"),
            other => panic!("expected delta, got {other:?}"),
        }

        // message_delta → 合并 input/output 的单个 Usage 事件。
        match parser.parse_line(Some("message_delta"), r#"{"usage": {"output_tokens": 9}}"#) {
            Some(SseLineAction::Usage(usage)) => {
                assert_eq!(usage.prompt_tokens, Some(25));
                assert_eq!(usage.completion_tokens, Some(9));
            }
            other => panic!("expected usage, got {other:?}"),
        }

        // message_stop → Done；error → Failed。
        assert!(matches!(
            parser.parse_line(Some("message_stop"), "{}"),
            Some(SseLineAction::Done)
        ));
        assert!(matches!(
            parser.parse_line(Some("error"), r#"{"error": {"type": "overloaded_error"}}"#),
            Some(SseLineAction::Failed(AiError::ProviderFailed))
        ));
        // ping 等事件被忽略。
        assert!(parser.parse_line(Some("ping"), "{}").is_none());
    }

    #[test]
    fn openai_sse_line_parser_keeps_existing_semantics() {
        assert!(matches!(
            parse_openai_line(None, "[DONE]"),
            Some(SseLineAction::Done)
        ));
        match parse_openai_line(None, r#"{"choices": [{"delta": {"content": "chunk"}}]}"#) {
            Some(SseLineAction::Delta(text)) => assert_eq!(text, "chunk"),
            other => panic!("expected delta, got {other:?}"),
        }
        match parse_openai_line(
            None,
            r#"{"usage": {"prompt_tokens": 3, "completion_tokens": 4}}"#,
        ) {
            Some(SseLineAction::Usage(usage)) => {
                assert_eq!(usage.prompt_tokens, Some(3));
            }
            other => panic!("expected usage, got {other:?}"),
        }
    }

    #[test]
    fn model_list_parses_ids_and_dedups() {
        let body = serde_json::json!({
            "object": "list",
            "data": [
                { "id": "gpt-4o" },
                { "id": "gpt-4o-mini", "created": 1 },
                { "id": "gpt-4o" },
                { "object": "model", "id": "" }
            ]
        });
        assert_eq!(parse_model_list(&body), vec!["gpt-4o", "gpt-4o-mini"]);
        // 缺 data / data 非数组 → 空列表（由 HTTP 层决定是否视为 InvalidOutput）。
        assert!(parse_model_list(&serde_json::json!({})).is_empty());
        assert!(parse_model_list(&serde_json::json!({ "data": "nope" })).is_empty());
    }

    #[test]
    fn embedding_response_parses_vectors_in_input_order() {
        let body = serde_json::json!({
            "object": "list",
            "data": [
                { "index": 0, "embedding": [0.1, -0.2, 3.0] },
                { "index": 1, "embedding": [1.5, 0.0, -2.25] }
            ],
            "usage": { "prompt_tokens": 8, "total_tokens": 8 }
        });
        let vectors = parse_embedding_response(&body, 2).unwrap();
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0], vec![0.1_f32, -0.2, 3.0]);
        assert_eq!(vectors[1], vec![1.5_f32, 0.0, -2.25]);
    }

    #[test]
    fn embedding_response_rejects_malformed_payloads() {
        // 数量与输入不一致。
        let short = serde_json::json!({ "data": [{ "embedding": [0.1] }] });
        assert!(matches!(
            parse_embedding_response(&short, 2),
            Err(AiError::InvalidOutput)
        ));
        // 缺 data / 缺 embedding / 非数值元素。
        assert!(matches!(
            parse_embedding_response(&serde_json::json!({}), 0),
            Err(AiError::InvalidOutput)
        ));
        let missing = serde_json::json!({ "data": [{ "index": 0 }] });
        assert!(matches!(
            parse_embedding_response(&missing, 1),
            Err(AiError::InvalidOutput)
        ));
        let non_numeric = serde_json::json!({ "data": [{ "embedding": ["x"] }] });
        assert!(matches!(
            parse_embedding_response(&non_numeric, 1),
            Err(AiError::InvalidOutput)
        ));
        // f32 极值可无损往返。
        let extreme = serde_json::json!({ "data": [{ "embedding": [1e-40, 3.4e38] }] });
        let vectors = parse_embedding_response(&extreme, 1).unwrap();
        assert_eq!(vectors[0][0], 1e-40_f32);
        assert_eq!(vectors[0][1], 3.4e38_f32);
    }
}

// ============================================================
// AI 请求审计的 PostgreSQL 实现
// ============================================================

use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use aries_core::ai::{
    AiRequest, AiRequestListQuery, AiRequestPage, AiRequestRepository, NewAiRequest,
};
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresAiRequestRepository {
    pool: PgPool,
}

impl PostgresAiRequestRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct AiRequestRow {
    id: i64,
    feature: String,
    operator_user_id: Option<i64>,
    model: String,
    status: String,
    prompt_tokens: Option<i32>,
    completion_tokens: Option<i32>,
    latency_ms: i32,
    error_category: Option<String>,
    created_at: OffsetDateTime,
}

impl TryFrom<AiRequestRow> for AiRequest {
    type Error = AiError;

    fn try_from(row: AiRequestRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            feature: row.feature.parse().map_err(|_| AiError::Validation)?,
            operator_user_id: row.operator_user_id,
            model: row.model,
            status: row.status.parse().map_err(|_| AiError::Validation)?,
            prompt_tokens: row.prompt_tokens,
            completion_tokens: row.completion_tokens,
            latency_ms: row.latency_ms,
            error_category: row.error_category,
            created_at: row.created_at,
        })
    }
}

const AI_REQUEST_COLUMNS: &str = "id, feature, operator_user_id, model, status, \
    prompt_tokens, completion_tokens, latency_ms, error_category, created_at";

fn map_sqlx(error: sqlx::Error) -> AiError {
    tracing::error!(error = %error, "ai request repository operation failed");
    AiError::StoreUnavailable
}

#[async_trait]
impl AiRequestRepository for PostgresAiRequestRepository {
    async fn record(&self, request: NewAiRequest) -> Result<(), AiError> {
        logged_query(
            "INSERT INTO ai_requests \
             (feature, operator_user_id, model, status, prompt_tokens, completion_tokens, \
              latency_ms, error_category) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(request.feature.as_str())
        .bind(request.operator_user_id)
        .bind(&request.model)
        .bind(request.status.as_str())
        .bind(request.prompt_tokens)
        .bind(request.completion_tokens)
        .bind(request.latency_ms)
        .bind(&request.error_category)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn list(&self, query: AiRequestListQuery) -> Result<AiRequestPage, AiError> {
        let offset = (query.page.saturating_sub(1)) * query.page_size;
        let limit = i64::from(query.page_size);

        let mut conditions = vec!["true".to_string()];
        let mut bind_index = 1u32;
        if query.feature.is_some() {
            conditions.push(format!("feature = ${bind_index}"));
            bind_index += 1;
        }
        let where_clause = conditions.join(" AND ");

        let count_sql = format!("SELECT COUNT(*) FROM ai_requests WHERE {where_clause}");
        let mut count_query = logged_query_scalar::<i64>(&count_sql);
        if let Some(feature) = query.feature {
            count_query = count_query.bind(feature.as_str());
        }
        let total = count_query.fetch_one(&self.pool).await.map_err(map_sqlx)?;

        let data_sql = format!(
            "SELECT {AI_REQUEST_COLUMNS} FROM ai_requests WHERE {where_clause} \
             ORDER BY created_at DESC, id DESC LIMIT ${bind_index} OFFSET ${}",
            bind_index + 1
        );
        let mut data_query = logged_query_as::<AiRequestRow>(&data_sql);
        if let Some(feature) = query.feature {
            data_query = data_query.bind(feature.as_str());
        }
        data_query = data_query.bind(limit).bind(i64::from(offset));

        let rows = data_query.fetch_all(&self.pool).await.map_err(map_sqlx)?;
        let items: Vec<AiRequest> = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(AiRequestPage {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }
}
