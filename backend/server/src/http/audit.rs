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

#[derive(Debug, Deserialize)]
struct AuditListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    actor_user_id: Option<i64>,
    action: Option<String>,
    target_type: Option<String>,
    target_id: Option<String>,
    /// 时间范围按 `created_at` 左闭右开过滤，格式 RFC 3339。
    /// 用 String 接收再手动解析，保证非法值走统一的 400 JSON 错误结构。
    start: Option<String>,
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

#[derive(Debug, Serialize)]
struct AuditPageResponse {
    items: Vec<AuditLogResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

#[derive(Debug, Serialize)]
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
