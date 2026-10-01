//! Admin 自定义页面管理：CRUD + 分页，content_html 由 HTTP 层渲染后入库。

use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    pages::{NewPage, Page, PageError, PageListQuery, PageStatus, PageUpdate},
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
        .route("/pages", get(list_pages).post(create_page))
        .route(
            "/pages/{id}",
            get(get_page).put(update_page).delete(delete_page),
        )
}

#[derive(Debug, Deserialize)]
struct PageListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    status: Option<String>,
    keyword: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PagePayload {
    slug: String,
    title: String,
    #[serde(default)]
    content_markdown: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    sort_order: i32,
}

/// 页面 DTO：不返回内部字段（当前无敏感列，但保持 DTO/Domain 分离约定）。
#[derive(Debug, Serialize)]
struct PageResponse {
    id: i64,
    slug: String,
    title: String,
    content_markdown: String,
    content_html: String,
    status: String,
    sort_order: i32,
    created_by: Option<i64>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<Page> for PageResponse {
    fn from(page: Page) -> Self {
        Self {
            id: page.id,
            slug: page.slug,
            title: page.title,
            content_markdown: page.content_markdown,
            content_html: page.content_html,
            status: page.status.as_str().to_owned(),
            sort_order: page.sort_order,
            created_by: page.created_by,
            created_at: page.created_at,
            updated_at: page.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct PagePageResponse {
    items: Vec<PageResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

async fn list_pages(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<PageListParams>,
) -> Result<Json<PagePageResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let status = params
        .status
        .as_deref()
        .map(PageStatus::from_str)
        .transpose()?;
    let result = state
        .pages
        .list(PageListQuery {
            page: params.page.max(1),
            page_size: params.page_size.clamp(1, 100),
            status,
            keyword: params.keyword.filter(|value| !value.trim().is_empty()),
        })
        .await?;
    Ok(Json(PagePageResponse {
        items: result.items.into_iter().map(Into::into).collect(),
        total: result.total,
        page: result.page,
        page_size: result.page_size,
    }))
}

async fn create_page(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<PagePayload>,
) -> Result<(StatusCode, Json<PageResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let status = request
        .status
        .as_deref()
        .map(PageStatus::from_str)
        .transpose()?
        .unwrap_or(PageStatus::Draft);
    let content_html = render_markdown(&state, request.content_markdown.clone()).await?;
    let page = state
        .pages
        .create(NewPage {
            slug: request.slug,
            title: request.title,
            content_markdown: request.content_markdown,
            content_html,
            status,
            sort_order: request.sort_order,
            created_by: current.user.id,
        })
        .await?;
    write_page_audit(&state, &current, "page.created", page.id).await?;
    Ok((StatusCode::CREATED, Json(page.into())))
}

async fn get_page(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(page_id): Path<i64>,
) -> Result<Json<PageResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let page = state
        .pages
        .find(page_id)
        .await?
        .ok_or(PageError::NotFound)?;
    Ok(Json(page.into()))
}

async fn update_page(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(page_id): Path<i64>,
    Json(request): Json<PagePayload>,
) -> Result<Json<PageResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let status = request
        .status
        .as_deref()
        .map(PageStatus::from_str)
        .transpose()?
        .unwrap_or(PageStatus::Draft);
    let content_html = render_markdown(&state, request.content_markdown.clone()).await?;
    let page = state
        .pages
        .update(
            page_id,
            PageUpdate {
                slug: request.slug,
                title: request.title,
                content_markdown: request.content_markdown,
                content_html,
                status,
                sort_order: request.sort_order,
            },
        )
        .await?;
    write_page_audit(&state, &current, "page.updated", page.id).await?;
    Ok(Json(page.into()))
}

async fn delete_page(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(page_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state.pages.delete(page_id).await?;
    write_page_audit(&state, &current, "page.deleted", page_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn write_page_audit(
    state: &AppState,
    current: &CurrentUser,
    action: &str,
    page_id: i64,
) -> Result<(), ApiError> {
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: action.to_owned(),
            target_type: "page".to_owned(),
            target_id: Some(page_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(())
}
