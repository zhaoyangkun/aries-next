//! Admin AI 端点：编辑器助手（rewrite/summary/metadata，SSE 流式）+ 用量审计查询。
//! 端点挂在 /api/admin/ai 下：Session Cookie 的 Path 限定 /api/admin，
//! 挂在别处浏览器不会带 Cookie；同时自动获得 admin_router 的 Origin 校验。
//! AI 只做建议，不自动 Publish/Delete/Approve（Phase 08 §3）。

use std::convert::Infallible;
use std::time::{Duration, Instant};

use aries_core::ai::{
    AiChatRequest, AiError, AiFeature, AiRequestListQuery, AiRequestStatus, AiSettings,
    AiStreamEvent, AiUsage, NewAiRequest,
};
use aries_core::ai_prompts;
use aries_core::auth::{AuditEvent, Permission};
use aries_core::settings::SettingGroup;
use axum::{
    Json, Router,
    extract::{Query, State},
    response::sse::{Event, Sse},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::state::AppState;

use super::{auth::CurrentUser, error::ApiError};

/// 编辑器助手限流：每用户每分钟 10 次（内存滑动窗口，进程重启清零）。
const AI_RATE_LIMIT: usize = 10;
const AI_RATE_WINDOW: Duration = Duration::from_secs(60);
/// 输入长度上限：防止超长正文刷 Token 费用。
/// 正文类功能（摘要/SEO/标签/导读）上限按字符计：中文技术长文（含代码块）常超 2 万字，
/// 60k 字符约对应主流模型 60k Token 量级的输入上限，超出应明确报错而非静默截断。
const MAX_SNIPPET_LENGTH: usize = 8_000;
const MAX_CONTENT_LENGTH: usize = 60_000;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ai/editor/rewrite", post(editor_rewrite))
        .route("/ai/editor/summary", post(editor_summary))
        .route("/ai/editor/metadata", post(editor_metadata))
        .route("/ai/editor/tags", post(editor_tags))
        .route("/ai/editor/brief", post(editor_brief))
        .route("/ai/models", get(list_ai_models))
        .route("/ai/usage", get(list_ai_usage))
}

// ============================================================
// 设置加载与门禁
// ============================================================

/// 加载 AI 设置并做门禁：未启用 → 403，未配置 → 400，Feature 关闭 → 403。
/// 设置按请求实时读取，settings 更新后下一次请求即生效（无需重启重建 Provider）。
pub(super) async fn load_ai_settings(
    state: &AppState,
    feature_toggle: bool,
) -> Result<AiSettings, ApiError> {
    let record = state.settings.get_group(SettingGroup::Ai).await?;
    let settings: AiSettings =
        serde_json::from_value(record.payload).map_err(|_| ApiError::internal())?;
    if !settings.enabled {
        return Err(AiError::Disabled.into());
    }
    if !feature_toggle {
        return Err(AiError::FeatureDisabled.into());
    }
    if !settings.is_configured() {
        return Err(AiError::Unconfigured.into());
    }
    Ok(settings)
}

// ============================================================
// SSE 事件格式（与前端约定）
// ============================================================

/// 出站 SSE 事件：start → delta* → usage? → done；失败时 error 终止。
enum SseOut {
    Start,
    Delta(String),
    Usage(AiUsage),
    Done,
    Error(&'static str),
}

impl SseOut {
    fn into_event(self, feature: AiFeature, model: &str) -> Event {
        match self {
            Self::Start => Event::default().event("start").data(
                serde_json::json!({
                    "feature": feature.as_str(),
                    "model": model,
                    "prompt_version": ai_prompts::PROMPT_VERSION,
                })
                .to_string(),
            ),
            Self::Delta(text) => Event::default()
                .event("delta")
                .data(serde_json::json!({ "text": text }).to_string()),
            Self::Usage(usage) => Event::default()
                .event("usage")
                .data(serde_json::to_string(&usage).unwrap_or_default()),
            Self::Done => Event::default().event("done").data("{}"),
            Self::Error(code) => Event::default()
                .event("error")
                .data(serde_json::json!({ "code": code }).to_string()),
        }
    }
}

/// 一次流式编辑器请求的完整上下文（收敛参数，避免过长签名）。
struct EditorStreamJob {
    state: AppState,
    feature: AiFeature,
    settings: AiSettings,
    request: AiChatRequest,
    operator_id: i64,
    input_len: usize,
    validate_json_output: bool,
    sender: mpsc::Sender<Result<Event, Infallible>>,
}

/// 驱动一次流式 AI 请求：启动事件 → 转发 Provider 增量 → 收尾（done/error）→ 落审计。
/// metadata 类 Feature 在 done 前校验累积文本为合法 JSON，不合格按 invalid_output 失败处理。
async fn run_editor_stream(job: EditorStreamJob) {
    let EditorStreamJob {
        state,
        feature,
        settings,
        request,
        operator_id,
        input_len,
        validate_json_output,
        sender,
    } = job;
    let started = Instant::now();
    let model = settings.model.clone().unwrap_or_default();

    let send = |event: SseOut| {
        let sender = sender.clone();
        let model = model.clone();
        async move {
            sender
                .send(Ok(event.into_event(feature, &model)))
                .await
                .is_ok()
        }
    };

    if !send(SseOut::Start).await {
        return record_ai_request(
            &state,
            build_ai_request(
                feature,
                &model,
                started,
                None,
                AiRequestStatus::Cancelled,
                None,
                Some(operator_id),
            ),
        )
        .await;
    }

    let (inner_tx, mut inner_rx) = mpsc::channel::<AiStreamEvent>(64);
    let provider = state.ai.clone();
    let provider_settings = settings.clone();
    let provider_handle = tokio::spawn(async move {
        provider
            .chat_stream(&provider_settings, request, inner_tx)
            .await
    });

    let mut accumulated = String::new();
    let mut usage: Option<AiUsage> = None;
    let mut failure: Option<AiError> = None;
    let mut cancelled = false;

    while let Some(event) = inner_rx.recv().await {
        let out = match event {
            AiStreamEvent::Delta(delta) => {
                accumulated.push_str(&delta);
                SseOut::Delta(delta)
            }
            AiStreamEvent::Usage(value) => {
                usage = Some(value);
                SseOut::Usage(value)
            }
            AiStreamEvent::Done => break,
        };
        if !send(out).await {
            // 客户端断开：取消流，落 cancelled 审计。
            cancelled = true;
            provider_handle.abort();
            break;
        }
    }

    if !cancelled && failure.is_none() {
        match provider_handle.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failure = Some(error),
            Err(_) => failure = Some(AiError::ProviderFailed),
        }
    }

    // metadata：Provider 必须输出合法 JSON，服务端校验通过才发 done。
    if !cancelled
        && failure.is_none()
        && validate_json_output
        && serde_json::from_str::<serde_json::Value>(accumulated.trim()).is_err()
    {
        failure = Some(AiError::InvalidOutput);
    }

    let (status, terminal) = if cancelled {
        (AiRequestStatus::Cancelled, None)
    } else if let Some(error) = &failure {
        let code = match error {
            AiError::Timeout => "AI_PROVIDER_TIMEOUT",
            AiError::RateLimited => "AI_RATE_LIMITED",
            AiError::InvalidOutput => "AI_INVALID_OUTPUT",
            _ => "AI_PROVIDER_FAILED",
        };
        (AiRequestStatus::Failed, Some(SseOut::Error(code)))
    } else {
        (AiRequestStatus::Success, Some(SseOut::Done))
    };
    if let Some(event) = terminal {
        let _ = send(event).await;
    }

    let error_category = failure.as_ref().map(|error| error.category().to_owned());
    record_ai_request(
        &state,
        build_ai_request(
            feature,
            &model,
            started,
            usage,
            status,
            error_category,
            Some(operator_id),
        ),
    )
    .await;
    write_ai_audit(&state, operator_id, feature, input_len, status).await;
}

/// 组装审计记录（latency 从 started 算起）。
fn build_ai_request(
    feature: AiFeature,
    model: &str,
    started: Instant,
    usage: Option<AiUsage>,
    status: AiRequestStatus,
    error_category: Option<String>,
    operator_user_id: Option<i64>,
) -> NewAiRequest {
    let usage = usage.unwrap_or_default();
    NewAiRequest {
        feature,
        operator_user_id,
        model: model.to_owned(),
        status,
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        latency_ms: i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX),
        error_category,
    }
}

async fn record_ai_request(state: &AppState, request: NewAiRequest) {
    if let Err(error) = state.ai_requests.record(request).await {
        // 审计写入失败只记日志，不影响已完成的流式响应。
        tracing::warn!(error = %error, "failed to record ai request");
    }
}

async fn write_ai_audit(
    state: &AppState,
    operator_id: i64,
    feature: AiFeature,
    input_len: usize,
    status: AiRequestStatus,
) {
    // Audit 不保存完整 Prompt：只记录输入长度与 Hash（Phase 08 §6）。
    let event = AuditEvent {
        actor_user_id: Some(operator_id),
        action: format!("ai.{feature}"),
        target_type: "ai_request".to_owned(),
        target_id: None,
        metadata: serde_json::json!({
            "input_len": input_len,
            "status": status.as_str(),
        }),
    };
    if let Err(error) = state.auth.write_audit(event).await {
        tracing::warn!(error = %error, "failed to write ai audit");
    }
}

// ============================================================
// 编辑器助手端点
// ============================================================

#[derive(Debug, Deserialize)]
struct RewriteRequest {
    text: String,
}

#[derive(Debug, Deserialize)]
struct SummaryRequest {
    title: String,
    content: String,
}

fn ai_sse_response(
    state: AppState,
    feature: AiFeature,
    settings: AiSettings,
    request: AiChatRequest,
    operator_id: i64,
    input_len: usize,
    validate_json_output: bool,
) -> Response {
    let (tx, mut rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(run_editor_stream(EditorStreamJob {
        state,
        feature,
        settings,
        request,
        operator_id,
        input_len,
        validate_json_output,
        sender: tx,
    }));
    Sse::new(futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx))).into_response()
}

async fn prepare_editor_call(
    state: &AppState,
    current: &CurrentUser,
    input_len: usize,
    max_len: usize,
) -> Result<AiSettings, ApiError> {
    current.require(Permission::ManageContent)?;
    if input_len == 0 || input_len > max_len {
        return Err(ApiError::bad_request(
            "INVALID_AI_INPUT",
            "AI input length is invalid",
        ));
    }
    // 限流先于 Provider 调用，避免刷 Token 费用。
    let rate_key = format!("ai-editor:{}", current.user.id);
    let decision = state
        .rate_limiter
        .check(&rate_key, AI_RATE_LIMIT, AI_RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }
    load_ai_settings(state, true).await
}

async fn editor_rewrite(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<RewriteRequest>,
) -> Result<Response, ApiError> {
    let text = request.text.trim().to_owned();
    let settings =
        prepare_editor_call(&state, &current, text.chars().count(), MAX_SNIPPET_LENGTH).await?;
    if !settings.features.editor_assist {
        return Err(AiError::FeatureDisabled.into());
    }
    let messages = ai_prompts::editor_rewrite_messages(&text);
    Ok(ai_sse_response(
        state,
        AiFeature::EditorRewrite,
        settings,
        AiChatRequest {
            messages,
            ..AiChatRequest::default()
        },
        current.user.id,
        text.chars().count(),
        false,
    ))
}

async fn editor_summary(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<SummaryRequest>,
) -> Result<Response, ApiError> {
    let input_len = request.title.chars().count() + request.content.chars().count();
    let settings = prepare_editor_call(&state, &current, input_len, MAX_CONTENT_LENGTH).await?;
    if !settings.features.editor_assist {
        return Err(AiError::FeatureDisabled.into());
    }
    let messages = ai_prompts::editor_summary_messages(&request.title, &request.content);
    Ok(ai_sse_response(
        state,
        AiFeature::EditorSummary,
        settings,
        AiChatRequest {
            messages,
            ..AiChatRequest::default()
        },
        current.user.id,
        input_len,
        false,
    ))
}

async fn editor_metadata(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<SummaryRequest>,
) -> Result<Response, ApiError> {
    let input_len = request.title.chars().count() + request.content.chars().count();
    let settings = prepare_editor_call(&state, &current, input_len, MAX_CONTENT_LENGTH).await?;
    if !settings.features.editor_assist {
        return Err(AiError::FeatureDisabled.into());
    }
    let messages = ai_prompts::editor_metadata_messages(&request.title, &request.content);
    Ok(ai_sse_response(
        state,
        AiFeature::EditorMetadata,
        settings,
        AiChatRequest {
            messages,
            ..AiChatRequest::default()
        },
        current.user.id,
        input_len,
        // metadata 要求 Provider 输出 JSON，服务端校验后才发 done。
        true,
    ))
}

/// 标签推荐：输出 JSON `{"tags": [...]}`，前端按名称匹配/创建标签。
async fn editor_tags(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<SummaryRequest>,
) -> Result<Response, ApiError> {
    let input_len = request.title.chars().count() + request.content.chars().count();
    let settings = prepare_editor_call(&state, &current, input_len, MAX_CONTENT_LENGTH).await?;
    if !settings.features.editor_assist {
        return Err(AiError::FeatureDisabled.into());
    }
    let messages = ai_prompts::editor_tags_messages(&request.title, &request.content);
    Ok(ai_sse_response(
        state,
        AiFeature::EditorTags,
        settings,
        AiChatRequest {
            messages,
            ..AiChatRequest::default()
        },
        current.user.id,
        input_len,
        true,
    ))
}

/// AI 导读：输出 150 字以内 TL;DR 纯文本，由前端写入文章 ai_brief 字段随文保存。
async fn editor_brief(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<SummaryRequest>,
) -> Result<Response, ApiError> {
    let input_len = request.title.chars().count() + request.content.chars().count();
    let settings = prepare_editor_call(&state, &current, input_len, MAX_CONTENT_LENGTH).await?;
    if !settings.features.editor_assist {
        return Err(AiError::FeatureDisabled.into());
    }
    let messages = ai_prompts::ai_brief_messages(&request.title, &request.content);
    Ok(ai_sse_response(
        state,
        AiFeature::AiBrief,
        settings,
        AiChatRequest {
            messages,
            ..AiChatRequest::default()
        },
        current.user.id,
        input_len,
        false,
    ))
}

// ============================================================
// 模型列表（管理端「获取模型」）
// ============================================================

/// 「获取模型」限流：每用户每分钟 10 次（内存滑动窗口）。
const LIST_MODELS_RATE_LIMIT: usize = 10;
const LIST_MODELS_RATE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "AiModelsResponse",
    description = "Provider 返回的模型 ID 列表（去重排序，上限 500）。"
)]
pub(crate) struct AiModelsResponse {
    pub(crate) models: Vec<String>,
}

/// 拉取当前 Provider 账号的可用模型列表。
/// 与编辑器助手不同：不要求总开关已启用、也不要求 model 已配置——
/// 典型场景是配置阶段拉列表选模型；只要求已保存 api_key。
#[utoipa::path(
    get,
    path = "/api/admin/ai/models",
    tag = "Admin AI",
    operation_id = "listAiModels",
    summary = "拉取当前 Provider 账号的可用模型列表",
    description = "只要求已保存 api_key，不要求总开关已启用或 model 已配置；OpenAI 兼容协议还要求已保存 base_url。每用户限流 10 次/分钟。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "可用模型 ID 列表", body = AiModelsResponse),
        (status = 400, description = "未保存 api_key/base_url"),
        (status = 401, description = "未认证"),
        (status = 403, description = "无 settings:manage 权限"),
        (status = 502, description = "Provider 请求失败或超时"),
    )
)]
pub(crate) async fn list_ai_models(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<AiModelsResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let rate_key = format!("ai-models:{}", current.user.id);
    let decision =
        state
            .rate_limiter
            .check(&rate_key, LIST_MODELS_RATE_LIMIT, LIST_MODELS_RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }

    let record = state.settings.get_group(SettingGroup::Ai).await?;
    let settings: AiSettings =
        serde_json::from_value(record.payload).map_err(|_| ApiError::internal())?;
    if settings.api_key.as_deref().is_none_or(str::is_empty) {
        return Err(AiError::Unconfigured.into());
    }
    let models = state.ai.list_models(&settings).await?;
    Ok(Json(AiModelsResponse { models }))
}

// ============================================================
// 用量审计查询
// ============================================================

#[derive(Debug, Deserialize)]
struct AiUsageParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    feature: Option<String>,
}

#[derive(Debug, Serialize)]
struct AiUsageItem {
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

#[derive(Debug, Serialize)]
struct AiUsagePageResponse {
    items: Vec<AiUsageItem>,
    total: i64,
    page: u32,
    page_size: u32,
}

async fn list_ai_usage(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<AiUsageParams>,
) -> Result<Json<AiUsagePageResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let feature = params
        .feature
        .as_deref()
        .map(str::parse::<AiFeature>)
        .transpose()?;
    let result = state
        .ai_requests
        .list(AiRequestListQuery {
            page: params.page.max(1),
            page_size: params.page_size.clamp(1, 100),
            feature,
        })
        .await?;
    Ok(Json(AiUsagePageResponse {
        items: result
            .items
            .into_iter()
            .map(|item| AiUsageItem {
                id: item.id,
                feature: item.feature.as_str().to_owned(),
                operator_user_id: item.operator_user_id,
                model: item.model,
                status: item.status.as_str().to_owned(),
                prompt_tokens: item.prompt_tokens,
                completion_tokens: item.completion_tokens,
                latency_ms: item.latency_ms,
                error_category: item.error_category,
                created_at: item.created_at,
            })
            .collect(),
        total: result.total,
        page: result.page,
        page_size: result.page_size,
    }))
}

// ============================================================
// 评论 AI 审核挂钩
// ============================================================

/// 评论审核结论的约定 JSON 输出（见 ai_prompts::comment_moderation_messages）。
#[derive(Debug, Deserialize)]
struct ModerationVerdict {
    risk: String,
    reason: Option<String>,
    confidence: Option<f32>,
}

/// 高置信垃圾阈值：只有达到阈值才允许 AI 把评论标记为 spam。
const SPAM_CONFIDENCE_THRESHOLD: f32 = 0.8;
/// 评论审核用短超时：不拖累访客提交的主流程。
const MODERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// 评论创建后的 AI 审核：任何失败都静默回退原流程（warn 日志 + failed 审计），
/// 绝不影响访客提交。高置信 spam 标记由 Repository 一并改状态。
pub(crate) async fn moderate_comment_with_ai(
    state: &AppState,
    comment: aries_core::comments::Comment,
) -> aries_core::comments::Comment {
    let started = Instant::now();
    let fallback = || comment.clone();

    let record = match state.settings.get_group(SettingGroup::Ai).await {
        Ok(record) => record,
        Err(_) => return comment,
    };
    let settings: AiSettings = match serde_json::from_value(record.payload) {
        Ok(settings) => settings,
        Err(_) => return comment,
    };
    if !settings.enabled || !settings.features.comment_moderation || !settings.is_configured() {
        return comment;
    }

    let model = settings.model.clone().unwrap_or_default();
    let messages = ai_prompts::comment_moderation_messages(&comment.content_markdown);
    let outcome = tokio::time::timeout(
        MODERATION_TIMEOUT,
        state.ai.chat(
            &settings,
            AiChatRequest {
                messages,
                temperature: Some(0.0),
                ..AiChatRequest::default()
            },
        ),
    )
    .await;

    let (text, usage) = match outcome {
        Ok(Ok((text, usage))) => (text, usage),
        Ok(Err(error)) => {
            tracing::warn!(error = %error, comment_id = comment.id, "ai comment moderation failed");
            record_moderation_request(
                state,
                started,
                &model,
                None,
                AiRequestStatus::Failed,
                Some(error.category()),
            )
            .await;
            return fallback();
        }
        Err(_) => {
            record_moderation_request(
                state,
                started,
                &model,
                None,
                AiRequestStatus::Failed,
                Some("timeout"),
            )
            .await;
            return fallback();
        }
    };

    // 解析约定 JSON；输出不合格按失败回退。
    let verdict: ModerationVerdict = match serde_json::from_str(text.trim()) {
        Ok(verdict) => verdict,
        Err(_) => {
            record_moderation_request(
                state,
                started,
                &model,
                usage,
                AiRequestStatus::Failed,
                Some("invalid_output"),
            )
            .await;
            return fallback();
        }
    };
    let risk: aries_core::comments::AiRisk = match verdict.risk.parse() {
        Ok(risk) => risk,
        Err(_) => {
            record_moderation_request(
                state,
                started,
                &model,
                usage,
                AiRequestStatus::Failed,
                Some("invalid_output"),
            )
            .await;
            return fallback();
        }
    };

    // 只有高置信 spam 才允许 AI 改状态；其余只记录风险结论。
    let effective_risk = if risk == aries_core::comments::AiRisk::Spam
        && verdict.confidence.unwrap_or(0.0) >= SPAM_CONFIDENCE_THRESHOLD
    {
        risk
    } else if risk == aries_core::comments::AiRisk::Spam {
        // 置信不足的 spam 降级为 suspicious，避免误伤。
        aries_core::comments::AiRisk::Suspicious
    } else {
        risk
    };

    let updated = match state
        .comments
        .apply_ai_assessment(aries_core::comments::AiAssessment {
            comment_id: comment.id,
            risk: effective_risk,
            reason: verdict.reason,
            confidence: verdict.confidence,
        })
        .await
    {
        Ok(updated) => updated,
        Err(error) => {
            tracing::warn!(error = %error, comment_id = comment.id, "failed to store ai assessment");
            return fallback();
        }
    };
    record_moderation_request(
        state,
        started,
        &model,
        usage,
        AiRequestStatus::Success,
        None,
    )
    .await;
    updated
}

async fn record_moderation_request(
    state: &AppState,
    started: Instant,
    model: &str,
    usage: Option<AiUsage>,
    status: AiRequestStatus,
    error_category: Option<&str>,
) {
    // 系统触发：operator 为空。
    let request = build_ai_request(
        AiFeature::CommentModeration,
        model,
        started,
        usage,
        status,
        error_category.map(str::to_owned),
        None,
    );
    if let Err(error) = state.ai_requests.record(request).await {
        tracing::warn!(error = %error, "failed to record comment moderation request");
    }
}
