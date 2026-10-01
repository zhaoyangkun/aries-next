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

use super::{articles::render_markdown, auth::CurrentUser, error::ApiError};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/journals", get(list_journals).post(create_journal))
        .route(
            "/journals/{id}",
            get(get_journal).put(update_journal).delete(delete_journal),
        )
}

#[derive(Debug, Deserialize)]
struct JournalListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    visibility: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JournalPayload {
    content_markdown: String,
    #[serde(default)]
    visibility: Option<String>,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
struct JournalPageResponse {
    items: Vec<JournalResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

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

async fn create_journal(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<JournalPayload>,
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

async fn update_journal(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(journal_id): Path<i64>,
    Json(request): Json<JournalPayload>,
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
