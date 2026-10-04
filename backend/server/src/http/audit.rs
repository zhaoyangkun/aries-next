//! Admin Audit Log 查询：操作人/动作/目标/日期范围筛选 + 分页，仅 Owner 可访问。

use aries_core::auth::{AuditListQuery, AuditLogEntry, Permission};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::state::AppState;

use super::{auth::CurrentUser, error::ApiError};

pub fn router() -> Router<AppState> {
    Router::new().route("/audit-logs", get(list_audit_logs))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
struct AuditListParams {
    /// 页码，从 1 开始。
    #[serde(default = "super::default_page")]
    page: u32,
    /// 每页条数（1–100）。
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 操作人 User ID 精确过滤。
    actor_user_id: Option<i64>,
    /// 动作精确过滤，如 `article.created`、`comment.moderated`。
    action: Option<String>,
    /// 目标类型精确过滤，如 `article`、`comment`、`site_settings`。
    target_type: Option<String>,
    /// 目标 ID 精确过滤。
    target_id: Option<String>,
    /// 时间范围起点（按 `created_at` 左闭右开过滤），格式 RFC 3339；非法值返回 400 `INVALID_DATE_RANGE`。
    start: Option<String>,
    /// 时间范围终点，格式同 `start`。
    end: Option<String>,
}

/// 将 RFC 3339 字符串解析为 `OffsetDateTime`，非法值返回统一 400。
fn parse_rfc3339(value: &str) -> Result<OffsetDateTime, ApiError> {
    OffsetDateTime::parse(value.trim(), &Rfc3339).map_err(|_| {
        ApiError::bad_request(
            "INVALID_DATE_RANGE",
            "start/end must be valid RFC 3339 timestamps",
        )
    })
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "AuditPageResponse", description = "审计日志分页。")]
struct AuditPageResponse {
    items: Vec<AuditLogResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "AuditLogEntry", description = "单条审计日志记录。")]
struct AuditLogResponse {
    id: i64,
    actor_user_id: Option<i64>,
    actor_username: Option<String>,
    action: String,
    target_type: String,
    target_id: Option<String>,
    metadata: serde_json::Value,
    created_at: OffsetDateTime,
}

impl From<AuditLogEntry> for AuditLogResponse {
    fn from(entry: AuditLogEntry) -> Self {
        Self {
            id: entry.id,
            actor_user_id: entry.actor_user_id,
            actor_username: entry.actor_username,
            action: entry.action,
            target_type: entry.target_type,
            target_id: entry.target_id,
            metadata: entry.metadata,
            created_at: entry.created_at,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/admin/audit-logs",
    tag = "Admin Audit",
    operation_id = "listAuditLogs",
    summary = "审计日志查询",
    description = "Admin 敏感操作审计日志查询，仅 Owner（`settings:manage`）可访问。时间范围按 `created_at` 左闭右开过滤，格式 RFC 3339；非法时间值返回 400 `INVALID_DATE_RANGE`。",
    security(("cookieAuth" = [])),
    params(AuditListParams),
    responses(
        (status = 200, description = "审计日志分页", body = AuditPageResponse),
        (status = 400, description = "start/end 非合法 RFC 3339 时间", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "非 Owner（settings:manage）", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_audit_logs(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<AuditListParams>,
) -> Result<Json<AuditPageResponse>, ApiError> {
    // Audit Log 含敏感操作记录，只开放给 Owner（ManageSettings）。
    current.require(Permission::ManageSettings)?;
    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let result = state
        .auth
        .list_audit(AuditListQuery {
            page,
            page_size,
            actor_user_id: params.actor_user_id,
            action: params
                .action
                .clone()
                .filter(|value| !value.trim().is_empty()),
            target_type: params
                .target_type
                .clone()
                .filter(|value| !value.trim().is_empty()),
            target_id: params
                .target_id
                .clone()
                .filter(|value| !value.trim().is_empty()),
            start: params.start.as_deref().map(parse_rfc3339).transpose()?,
            end: params.end.as_deref().map(parse_rfc3339).transpose()?,
        })
        .await?;
    Ok(Json(AuditPageResponse {
        items: result.items.into_iter().map(Into::into).collect(),
        total: result.total,
        page: result.page,
        page_size: result.page_size,
    }))
}
