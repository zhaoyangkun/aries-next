//! AI 能力域：Provider 适配契约、结构化设置与请求审计。
//! core 不依赖具体 Vendor；AI 只做建议与标记，不自动 Publish/Delete/Approve（Phase 08 §3）。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::mpsc;

// ============================================================
// 功能标识
// ============================================================

/// AI 功能标识，与 `ai_requests.feature` 的 CHECK 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiFeature {
    EditorRewrite,
    EditorSummary,
    EditorMetadata,
    CommentModeration,
}

impl AiFeature {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EditorRewrite => "editor_rewrite",
            Self::EditorSummary => "editor_summary",
            Self::EditorMetadata => "editor_metadata",
            Self::CommentModeration => "comment_moderation",
        }
    }
}

impl fmt::Display for AiFeature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AiFeature {
    type Err = AiError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "editor_rewrite" => Ok(Self::EditorRewrite),
            "editor_summary" => Ok(Self::EditorSummary),
            "editor_metadata" => Ok(Self::EditorMetadata),
            "comment_moderation" => Ok(Self::CommentModeration),
            _ => Err(AiError::Validation),
        }
    }
}

// ============================================================
// 设置（setting_groups 的 ai 分组 payload）
// ============================================================

/// 功能开关：editor_assist 控制 rewrite/summary/metadata 三个编辑器端点。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiFeatureToggles {
    pub editor_assist: bool,
    pub comment_moderation: bool,
}

/// Provider 协议：openai 兼容 `/chat/completions`，anthropic 为 Messages API。
/// 默认 openai，保证旧 payload（无 protocol 字段）反序列化兼容。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    #[default]
    OpenAi,
    Anthropic,
}

impl AiProtocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }
}

/// AI 设置：`api_key` 为 write-only secret——PUT 省略/null 保持不变，
/// 空字符串清除，非空更新；GET 永不回传明文（HTTP 层转成 `api_key_set`）。
/// `#[serde(default)]` 允许 jsonb 缺省字段，便于增量演进。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub enabled: bool,
    pub protocol: AiProtocol,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub features: AiFeatureToggles,
}

impl AiSettings {
    /// 配置完整性：三者缺一即视为未配置（调用方映射为 400）。
    pub fn is_configured(&self) -> bool {
        self.base_url.as_deref().is_some_and(|v| !v.is_empty())
            && self.model.as_deref().is_some_and(|v| !v.is_empty())
            && self.api_key.as_deref().is_some_and(|v| !v.is_empty())
    }
}

// ============================================================
// Provider 契约
// ============================================================

/// 对话消息：role 取值 system/user/assistant，不做枚举以便兼容各家扩展。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiMessage {
    pub role: String,
    pub content: String,
}

impl AiMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_owned(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_owned(),
            content: content.into(),
        }
    }
}

/// 对话请求；temperature/max_tokens 可空，缺省由 Provider 决定。
#[derive(Debug, Clone, Default)]
pub struct AiChatRequest {
    pub messages: Vec<AiMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// Token 用量；部分 Provider 流式末尾才回传，均允许为空。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiUsage {
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
}

/// 流式事件：Delta 为增量文本，Usage 在结束前回传，Done 表示正常结束。
/// 失败不走事件流，由 `chat_stream` 的 Err 返回（HTTP 层转成 error 事件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiStreamEvent {
    Delta(String),
    Usage(AiUsage),
    Done,
}

/// AI Provider 适配接口，由 infra 层实现（OpenAI 兼容协议）。
/// 设置按调用传入而非构建期绑定：settings 更新后下一次请求即生效，无需重启。
#[async_trait]
pub trait AiProvider: Send + Sync {
    /// 非流式对话，返回完整文本与用量。
    async fn chat(
        &self,
        settings: &AiSettings,
        request: AiChatRequest,
    ) -> Result<(String, Option<AiUsage>), AiError>;

    /// 流式对话：通过 sender 回传增量与用量，正常结束发 Done；
    /// 返回 Err 表示流失败（可能已发出部分 Delta，由调用方决定如何收尾）。
    async fn chat_stream(
        &self,
        settings: &AiSettings,
        request: AiChatRequest,
        sender: mpsc::Sender<AiStreamEvent>,
    ) -> Result<(), AiError>;
}

// ============================================================
// 请求审计
// ============================================================

/// AI 请求状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiRequestStatus {
    Success,
    Failed,
    Cancelled,
}

impl AiRequestStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl fmt::Display for AiRequestStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AiRequestStatus {
    type Err = AiError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "success" => Ok(Self::Success),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(AiError::Validation),
        }
    }
}

/// 记录一次 AI 请求（落库即审计，不存完整 Prompt）。
#[derive(Debug, Clone)]
pub struct NewAiRequest {
    pub feature: AiFeature,
    /// 系统触发（评论自动审核）为 None。
    pub operator_user_id: Option<i64>,
    pub model: String,
    pub status: AiRequestStatus,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub latency_ms: i32,
    /// 失败分类（timeout/rate_limited/provider/invalid_output），不含原始错误细节。
    pub error_category: Option<String>,
}

/// AI 请求审计记录。
#[derive(Debug, Clone)]
pub struct AiRequest {
    pub id: i64,
    pub feature: AiFeature,
    pub operator_user_id: Option<i64>,
    pub model: String,
    pub status: AiRequestStatus,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
    pub latency_ms: i32,
    pub error_category: Option<String>,
    pub created_at: OffsetDateTime,
}

/// 审计查询参数。
#[derive(Debug, Clone, Default)]
pub struct AiRequestListQuery {
    pub page: u32,
    pub page_size: u32,
    pub feature: Option<AiFeature>,
}

/// 分页审计列表。
#[derive(Debug, Clone)]
pub struct AiRequestPage {
    pub items: Vec<AiRequest>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// AI 请求审计 Repository，由 infra 层实现。
#[async_trait]
pub trait AiRequestRepository: Send + Sync {
    /// 记录一次请求；审计写入失败不应中断主流程，但需向上报告。
    async fn record(&self, request: NewAiRequest) -> Result<(), AiError>;

    /// 分页查询（按创建时间倒序，可按 feature 过滤）。
    async fn list(&self, query: AiRequestListQuery) -> Result<AiRequestPage, AiError>;
}

// ============================================================
// Error
// ============================================================

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AiError {
    /// AI 总开关未启用。
    #[error("ai is disabled")]
    Disabled,
    /// 缺少 base_url/model/api_key。
    #[error("ai provider is not configured")]
    Unconfigured,
    /// 单个 Feature 开关未启用。
    #[error("ai feature is disabled")]
    FeatureDisabled,
    #[error("ai provider request failed")]
    ProviderFailed,
    #[error("ai provider request timed out")]
    Timeout,
    #[error("ai provider rate limited the request")]
    RateLimited,
    /// Provider 输出不符合约定格式（如 metadata 未返回合法 JSON）。
    #[error("ai provider returned invalid output")]
    InvalidOutput,
    #[error("invalid ai parameter")]
    Validation,
    #[error("ai store unavailable")]
    StoreUnavailable,
}

impl AiError {
    /// 落库用的失败分类，不含原始错误细节（防 Secret 泄露）。
    pub fn category(&self) -> &'static str {
        match self {
            Self::Disabled | Self::Unconfigured | Self::FeatureDisabled | Self::Validation => {
                "config"
            }
            Self::ProviderFailed => "provider",
            Self::Timeout => "timeout",
            Self::RateLimited => "rate_limited",
            Self::InvalidOutput => "invalid_output",
            Self::StoreUnavailable => "store",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_settings_tolerate_missing_fields_and_detect_configuration() {
        // jsonb 中缺省字段应反序列化为默认值，而不是报错。
        let settings: AiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, AiSettings::default());
        assert!(!settings.enabled);
        assert!(!settings.is_configured());
        // 旧 payload 无 protocol 字段，默认 openai。
        assert_eq!(settings.protocol, AiProtocol::OpenAi);

        let configured: AiSettings = serde_json::from_str(
            r#"{
                "enabled": true,
                "protocol": "anthropic",
                "base_url": "https://api.example.com/v1",
                "model": "gpt-x",
                "api_key": "sk-test",
                "features": { "editor_assist": true }
            }"#,
        )
        .unwrap();
        assert!(configured.is_configured());
        assert_eq!(configured.protocol, AiProtocol::Anthropic);
        assert!(configured.features.editor_assist);
        // 缺省的 feature 开关为 false。
        assert!(!configured.features.comment_moderation);
    }

    #[test]
    fn ai_feature_parses_only_known_values() {
        assert_eq!("editor_rewrite".parse(), Ok(AiFeature::EditorRewrite));
        assert_eq!(
            "comment_moderation".parse(),
            Ok(AiFeature::CommentModeration)
        );
        assert_eq!("embedding".parse::<AiFeature>(), Err(AiError::Validation));
    }
}
