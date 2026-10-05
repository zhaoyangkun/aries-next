//! Admin 日志（短内容 Timeline）管理：CRUD + 分页，visibility public/private。
//! Private 日志只允许管理端访问；Public 端点在 http/public/extended.rs 单独过滤。

use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    journals::{
        Journal, JournalError, JournalListQuery, JournalUpdate, JournalVisibility, NewJournal,
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::state::AppState;

use super::{articles::render_markdown, auth::CurrentUser, error::ApiError, extract::ApiJson};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/journals", get(list_journals).post(create_journal))
        .route(
            "/journals/{id}",
            get(get_journal).put(update_journal).delete(delete_journal),
        )
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
struct JournalListParams {
    /// 页码，从 1 开始。
    #[serde(default = "super::default_page")]
    page: u32,
    /// 每页条数（1–100）。
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 可见性过滤：`public` / `private`；非法值返回 400。
    visibility: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(
    title = "JournalRequest",
    description = "日志创建/更新入参；`content_html` 由服务端 Render，visibility 缺省为 `public`。"
)]
struct JournalPayload {
    /// 日志正文 Markdown，长度 1–2000。
    content_markdown: String,
    /// 可见性：`public` / `private`，缺省 `public`。
    #[serde(default)]
    visibility: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "JournalResponse", description = "日志详情。")]
struct JournalResponse {
    id: i64,
    content_markdown: String,
    content_html: String,
    visibility: String,
    created_by: Option<i64>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<Journal> for JournalResponse {
    fn from(journal: Journal) -> Self {
        Self {
            id: journal.id,
            content_markdown: journal.content_markdown,
            content_html: journal.content_html,
            visibility: journal.visibility.as_str().to_owned(),
            created_by: journal.created_by,
            created_at: journal.created_at,
            updated_at: journal.updated_at,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "JournalPageResponse",
    description = "日志分页；稳定排序 `created_at DESC, id DESC`，不含已软删除记录。"
)]
struct JournalPageResponse {
    items: Vec<JournalResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

#[utoipa::path(
    get,
    path = "/api/admin/journals",
    tag = "Admin Journals",
    operation_id = "listJournals",
    summary = "日志分页列表",
    description = "按创建时间倒序分页返回日志；可按 visibility 过滤，非法 visibility 返回 400 `INVALID_JOURNAL_VISIBILITY`。",
    security(("cookieAuth" = [])),
    params(JournalListParams),
    responses(
        (status = 200, description = "日志分页", body = JournalPageResponse),
        (status = 400, description = "visibility 值非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_journals(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<JournalListParams>,
) -> Result<Json<JournalPageResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let visibility = params
        .visibility
        .as_deref()
        .map(JournalVisibility::from_str)
        .transpose()?;
    let result = state
        .journals
        .list(JournalListQuery {
            page: params.page.max(1),
            page_size: params.page_size.clamp(1, 100),
            visibility,
        })
        .await?;
    Ok(Json(JournalPageResponse {
        items: result.items.into_iter().map(Into::into).collect(),
        total: result.total,
        page: result.page,
        page_size: result.page_size,
    }))
}

#[utoipa::path(
    post,
    path = "/api/admin/journals",
    tag = "Admin Journals",
    operation_id = "createJournal",
    summary = "创建日志",
    description = "创建日志；`content_markdown` 长度 1–2000，`content_html` 由服务端 Render，visibility 缺省 `public`。",
    security(("cookieAuth" = [])),
    request_body(content = JournalPayload, content_type = "application/json", description = "日志入参"),
    responses(
        (status = 201, description = "返回新建的日志", body = JournalResponse),
        (status = 400, description = "正文长度或 visibility 非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn create_journal(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<JournalPayload>,
) -> Result<(StatusCode, Json<JournalResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let visibility = request
        .visibility
        .as_deref()
        .map(JournalVisibility::from_str)
        .transpose()?
        .unwrap_or(JournalVisibility::Public);
    let content_html = render_markdown(&state, request.content_markdown.clone()).await?;
    let journal = state
        .journals
        .create(NewJournal {
            content_markdown: request.content_markdown,
            content_html,
            visibility,
            created_by: current.user.id,
        })
        .await?;
    write_journal_audit(&state, &current, "journal.created", journal.id).await?;
    Ok((StatusCode::CREATED, Json(journal.into())))
}

#[utoipa::path(
    get,
    path = "/api/admin/journals/{id}",
    tag = "Admin Journals",
    operation_id = "getJournal",
    summary = "日志详情",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "日志 ID")),
    responses(
        (status = 200, description = "日志详情", body = JournalResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "日志不存在（JOURNAL_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_journal(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(journal_id): Path<i64>,
) -> Result<Json<JournalResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let journal = state
        .journals
        .find(journal_id)
        .await?
        .ok_or(JournalError::NotFound)?;
    Ok(Json(journal.into()))
}

#[utoipa::path(
    put,
    path = "/api/admin/journals/{id}",
    tag = "Admin Journals",
    operation_id = "updateJournal",
    summary = "全量更新日志",
    description = "全量更新日志；`content_html` 由服务端重新 Render，visibility 缺省 `public`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "日志 ID")),
    request_body(content = JournalPayload, content_type = "application/json", description = "日志入参"),
    responses(
        (status = 200, description = "返回更新后的日志", body = JournalResponse),
        (status = 400, description = "正文长度或 visibility 非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "日志不存在（JOURNAL_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_journal(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(journal_id): Path<i64>,
    ApiJson(request): ApiJson<JournalPayload>,
) -> Result<Json<JournalResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let visibility = request
        .visibility
        .as_deref()
        .map(JournalVisibility::from_str)
        .transpose()?
        .unwrap_or(JournalVisibility::Public);
    let content_html = render_markdown(&state, request.content_markdown.clone()).await?;
    let journal = state
        .journals
        .update(
            journal_id,
            JournalUpdate {
                content_markdown: request.content_markdown,
                content_html,
                visibility,
            },
        )
        .await?;
    write_journal_audit(&state, &current, "journal.updated", journal.id).await?;
    Ok(Json(journal.into()))
}

#[utoipa::path(
    delete,
    path = "/api/admin/journals/{id}",
    tag = "Admin Journals",
    operation_id = "deleteJournal",
    summary = "软删除日志",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "日志 ID")),
    responses(
        (status = 204, description = "日志已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "日志不存在（JOURNAL_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
async fn delete_journal(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(journal_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state.journals.delete(journal_id).await?;
    write_journal_audit(&state, &current, "journal.deleted", journal_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn write_journal_audit(
    state: &AppState,
    current: &CurrentUser,
    action: &str,
    journal_id: i64,
) -> Result<(), ApiError> {
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: action.to_owned(),
            target_type: "journal".to_owned(),
            target_id: Some(journal_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(())
}
