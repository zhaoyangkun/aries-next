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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub(crate) struct PageListParams {
    /// 页码，从 1 开始。
    #[serde(default = "super::default_page")]
    page: u32,
    /// 每页条数（1–100）。
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 按状态过滤：`draft` / `published`。
    status: Option<String>,
    /// 对标题与 Slug 做模糊匹配；空白值被忽略。
    keyword: Option<String>,
}

/// 自定义页面创建/更新请求体。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct PagePayload {
    /// 1–160 个小写字母、数字或连字符；软删除行不占用 Slug。
    slug: String,
    /// 页面标题。
    title: String,
    /// Markdown 正文，服务端渲染为 `content_html` 后入库。
    #[serde(default)]
    content_markdown: String,
    /// `draft` / `published`，缺省 `draft`。
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    sort_order: i32,
}

/// 页面 DTO：不返回内部字段（当前无敏感列，但保持 DTO/Domain 分离约定）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct PageResponse {
    id: i64,
    /// 1–160 个小写字母、数字或连字符。
    slug: String,
    title: String,
    /// Markdown 源文。
    content_markdown: String,
    /// 服务端渲染后的 HTML。
    content_html: String,
    /// `draft` / `published`。
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct PagePageResponse {
    items: Vec<PageResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

/// 分页查询自定义页面。
#[utoipa::path(
    get,
    path = "/api/admin/pages",
    tag = "Admin Pages",
    operation_id = "listPages",
    summary = "分页查询自定义页面",
    description = "稳定排序 `sort_order ASC, id ASC`，不含已软删除记录。",
    security(("cookieAuth" = [])),
    params(PageListParams),
    responses(
        (status = 200, description = "自定义页面分页结果", body = PagePageResponse),
        (status = 400, description = "status 参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_pages(
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

/// 创建自定义页面。
#[utoipa::path(
    post,
    path = "/api/admin/pages",
    tag = "Admin Pages",
    operation_id = "createPage",
    summary = "创建自定义页面",
    description = "`content_html` 由服务端 Render Markdown 后入库。Slug 冲突返回 409 `SLUG_CONFLICT`。",
    security(("cookieAuth" = [])),
    request_body(content = PagePayload, content_type = "application/json", description = "页面字段"),
    responses(
        (status = 201, description = "返回新建的页面", body = PageResponse),
        (status = 400, description = "参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 409, description = "Slug 冲突（SLUG_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn create_page(
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

/// 获取自定义页面详情。
#[utoipa::path(
    get,
    path = "/api/admin/pages/{id}",
    tag = "Admin Pages",
    operation_id = "getPage",
    summary = "获取自定义页面详情",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "页面 ID")),
    responses(
        (status = 200, description = "页面详情", body = PageResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "页面不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn get_page(
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

/// 全量更新自定义页面。
#[utoipa::path(
    put,
    path = "/api/admin/pages/{id}",
    tag = "Admin Pages",
    operation_id = "updatePage",
    summary = "全量更新自定义页面",
    description = "`content_html` 由服务端重新 Render。Slug 冲突返回 409 `SLUG_CONFLICT`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "页面 ID")),
    request_body(content = PagePayload, content_type = "application/json", description = "页面字段"),
    responses(
        (status = 200, description = "返回更新后的页面", body = PageResponse),
        (status = 400, description = "参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "页面不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "Slug 冲突（SLUG_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn update_page(
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

/// 软删除自定义页面。
#[utoipa::path(
    delete,
    path = "/api/admin/pages/{id}",
    tag = "Admin Pages",
    operation_id = "deletePage",
    summary = "软删除自定义页面",
    description = "删除后同 Slug 可重新创建。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "页面 ID")),
    responses(
        (status = 204, description = "页面已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "页面不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn delete_page(
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
