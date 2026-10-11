//! Admin 邮件端点：SMTP 测试发送与投递审计查询。
//! 测试发送走真实 `EmailSender`（配置从 `setting_groups` 现读），
//! 结果落 `email_deliveries` 审计；端点按用户限流，防止刷 SMTP 配额。

use std::time::Duration;

use aries_core::{
    auth::Permission,
    email::{self, EmailDeliveryListQuery, EmailDeliveryStatus, NewEmailDelivery},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{
    http::{
        auth::{CurrentUser, normalize_email},
        default_page, default_page_size,
        error::ApiError,
        extract::ApiJson,
    },
    notify::load_email_settings,
    state::AppState,
};

/// 测试邮件限流：每用户 15 分钟 5 封，防止刷 SMTP 配额/被判定滥发。
const TEST_RATE_LIMIT: usize = 5;
const TEST_RATE_WINDOW: Duration = Duration::from_secs(15 * 60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings/email/test", post(send_test_email))
        .route("/email-deliveries", get(list_email_deliveries))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(description = "发送 SMTP 测试邮件；`recipient` 为接收邮箱。")]
struct SendTestEmailRequest {
    /// 接收测试邮件的邮箱地址。
    recipient: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct SendTestEmailResponse {
    /// 发送结果：成功固定为 `sent`。
    status: &'static str,
}

#[utoipa::path(
    post,
    path = "/api/admin/settings/email/test",
    tag = "Admin Settings",
    operation_id = "sendTestEmail",
    summary = "发送 SMTP 测试邮件",
    description = "用当前保存的 SMTP 配置向指定邮箱发送测试邮件，结果记入投递审计（`email_deliveries`，模板 `test`）。需要 `settings:manage` 权限；每用户 5 次/15 分钟限流。邮件功能未启用或配置不完整返回 400 `EMAIL_NOT_CONFIGURED`；SMTP 发送失败返回 502 `EMAIL_SEND_FAILED`（失败原因落投递审计的 `last_error`，不回传细节）。",
    security(("cookieAuth" = [])),
    request_body(content = SendTestEmailRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "测试邮件已发送", body = SendTestEmailResponse),
        (status = 400, description = "收件邮箱格式非法或邮件功能未配置（EMAIL_NOT_CONFIGURED）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无设置管理权限", body = crate::openapi::ErrorResponse),
        (status = 429, description = "触发限流（RATE_LIMITED）", body = crate::openapi::ErrorResponse),
        (status = 502, description = "SMTP 发送失败（EMAIL_SEND_FAILED）", body = crate::openapi::ErrorResponse),
    )
)]
async fn send_test_email(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<SendTestEmailRequest>,
) -> Result<Json<SendTestEmailResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let recipient = normalize_email(&request.recipient)?;

    let rate_key = format!("email-test:{}", current.user.id);
    let decision = state
        .rate_limiter
        .check(&rate_key, TEST_RATE_LIMIT, TEST_RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }

    let settings = load_email_settings(&state)
        .await
        .map_err(ApiError::internal_logged)?;
    if !settings.enabled {
        return Err(ApiError::bad_request(
            "EMAIL_NOT_CONFIGURED",
            "Email delivery is disabled; enable and save SMTP settings first",
        ));
    }

    let site = state.site_settings.get().await?;
    let rendered = email::render_test_email(&site.site_name);
    // 投递审计先行：发送失败也能在投递列表看到记录与原因。
    let delivery_id = state
        .email_deliveries
        .create(NewEmailDelivery::new(
            None,
            recipient.clone(),
            rendered.subject.clone(),
            email::TEMPLATE_TEST.to_owned(),
        ))
        .await
        .map_err(ApiError::internal_logged)?
        .id;

    let message = email::EmailMessage {
        to: recipient,
        subject: rendered.subject,
        text: rendered.text,
        html: Some(rendered.html),
    };
    match state.email.send(&settings, &message).await {
        Ok(()) => {
            let _ = state.email_deliveries.mark_sent(delivery_id).await;
            Ok(Json(SendTestEmailResponse { status: "sent" }))
        }
        Err(error @ email::EmailError::NotConfigured) => {
            let _ = state
                .email_deliveries
                .mark_failed(delivery_id, &error.to_string())
                .await;
            Err(ApiError::bad_request(
                "EMAIL_NOT_CONFIGURED",
                "Email delivery is disabled or SMTP settings are incomplete",
            ))
        }
        Err(error) => {
            let _ = state
                .email_deliveries
                .mark_failed(delivery_id, &error.to_string())
                .await;
            // SMTP 细节（主机、响应码）只落投递审计与日志，不回传客户端。
            Err(ApiError::bad_gateway(
                "EMAIL_SEND_FAILED",
                "Failed to send test email; check email deliveries for details",
            ))
        }
    }
}

// ============================================================
// 投递审计
// ============================================================

#[derive(Debug, Deserialize, utoipa::IntoParams)]
struct EmailDeliveryListParams {
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_page_size")]
    page_size: u32,
    /// 状态过滤：`pending` / `sent` / `failed` / `bounced`；缺省返回全部。
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(description = "投递审计记录；不含任何凭据或邮件正文。")]
struct EmailDeliveryResponse {
    id: i64,
    /// 关联评论 ID；密码重置、测试邮件等系统邮件为 null。
    comment_id: Option<i64>,
    recipient_email: String,
    subject: String,
    /// 模板标识：`password_reset` / `comment_new` / `comment_reply` / `test`。
    template: String,
    status: String,
    attempts: i32,
    max_attempts: i32,
    last_error: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct EmailDeliveryPageResponse {
    items: Vec<EmailDeliveryResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

#[utoipa::path(
    get,
    path = "/api/admin/email-deliveries",
    tag = "Admin Settings",
    operation_id = "listEmailDeliveries",
    summary = "邮件投递审计列表",
    description = "分页查询全部外发邮件的投递记录（评论通知、密码重置、测试邮件），按创建时间降序。需要 `settings:manage` 权限。",
    security(("cookieAuth" = [])),
    params(EmailDeliveryListParams),
    responses(
        (status = 200, description = "投递记录分页", body = EmailDeliveryPageResponse),
        (status = 400, description = "状态过滤值非法（INVALID_EMAIL_DELIVERY_STATUS）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无设置管理权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_email_deliveries(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<EmailDeliveryListParams>,
) -> Result<Json<EmailDeliveryPageResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let status = params
        .status
        .as_deref()
        .map(str::parse::<EmailDeliveryStatus>)
        .transpose()
        .map_err(|_| {
            ApiError::bad_request(
                "INVALID_EMAIL_DELIVERY_STATUS",
                "Status must be one of pending, sent, failed, bounced",
            )
        })?;

    let page = state
        .email_deliveries
        .list(EmailDeliveryListQuery {
            page: params.page,
            page_size: params.page_size,
            status,
        })
        .await
        .map_err(ApiError::internal_logged)?;

    Ok(Json(EmailDeliveryPageResponse {
        items: page
            .items
            .into_iter()
            .map(|delivery| EmailDeliveryResponse {
                id: delivery.id,
                comment_id: delivery.comment_id,
                recipient_email: delivery.recipient_email,
                subject: delivery.subject,
                template: delivery.template,
                status: delivery.status.as_str().to_owned(),
                attempts: delivery.attempts,
                max_attempts: delivery.max_attempts,
                last_error: delivery.last_error,
                created_at: delivery.created_at,
                updated_at: delivery.updated_at,
            })
            .collect(),
        total: page.total,
        page: page.page,
        page_size: page.page_size,
    }))
}
