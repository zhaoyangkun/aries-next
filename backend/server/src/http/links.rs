//! Admin 友情链接管理：CRUD + 分页、友链分类管理（kind = 'link'）。
//! URL 仅允许 http/https（core 校验），分类删除沿用引用保护 + 物理删除惯例。

use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    content::{CategoryKind, CategoryUpdate, NewCategory},
    links::{Link, LinkError, LinkListQuery, LinkStatus, LinkUpdate, NewLink},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::state::AppState;

use super::{
    auth::CurrentUser,
    error::ApiError,
    extract::ApiJson,
    taxonomy::{CategoryResponse, map_taxonomy_error},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/links", get(list_links).post(create_link))
        .route(
            "/links/categories",
            get(list_link_categories).post(create_link_category),
        )
        .route(
            "/links/categories/{id}",
            put(update_link_category).delete(delete_link_category),
        )
        .route(
            "/links/{id}",
            get(get_link).put(update_link).delete(delete_link),
        )
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub(crate) struct LinkListParams {
    /// 页码，从 1 开始。
    #[serde(default = "super::default_page")]
    page: u32,
    /// 每页条数（1–100）。
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 按状态过滤：`active` / `inactive`。
    status: Option<String>,
    /// 按友链分类过滤。
    category_id: Option<i64>,
    /// 对标题与 URL 做模糊匹配；空白值被忽略。
    keyword: Option<String>,
}

/// 友情链接创建/更新请求体。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct LinkPayload {
    /// 可空（允许未分类）；须指向 kind 为 `link` 的分类。
    category_id: Option<i64>,
    /// 标题。
    title: String,
    /// 链接地址，仅允许 http/https，否则返回 400 `INVALID_LINK_URL`。
    url: String,
    /// 图标地址，可空。
    icon_url: Option<String>,
    #[serde(default)]
    description: String,
    /// `active` / `inactive`，缺省 `active`。
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    sort_order: i32,
}

/// 友情链接响应体。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LinkResponse {
    id: i64,
    category_id: Option<i64>,
    /// 标题。
    title: String,
    /// 链接地址，仅允许 http/https。
    url: String,
    /// 图标地址，可空。
    icon_url: Option<String>,
    description: String,
    /// `active` / `inactive`。
    status: String,
    sort_order: i32,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<Link> for LinkResponse {
    fn from(link: Link) -> Self {
        Self {
            id: link.id,
            category_id: link.category_id,
            title: link.title,
            url: link.url,
            icon_url: link.icon_url,
            description: link.description,
            status: link.status.as_str().to_owned(),
            sort_order: link.sort_order,
            created_at: link.created_at,
            updated_at: link.updated_at,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LinkPageResponse {
    items: Vec<LinkResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

/// 友链分类创建/更新请求体。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct CategoryPayload {
    name: String,
    /// 缺省时取 `name`。
    slug: Option<String>,
}

/// 分页查询友情链接。
#[utoipa::path(
    get,
    path = "/api/admin/links",
    tag = "Admin Links",
    operation_id = "listLinks",
    summary = "分页查询友情链接",
    description = "稳定排序 `sort_order ASC, id ASC`，不含已软删除记录。",
    security(("cookieAuth" = [])),
    params(LinkListParams),
    responses(
        (status = 200, description = "友情链接分页结果", body = LinkPageResponse),
        (status = 400, description = "status 参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_links(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<LinkListParams>,
) -> Result<Json<LinkPageResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let status = params
        .status
        .as_deref()
        .map(LinkStatus::from_str)
        .transpose()?;
    let result = state
        .links
        .list(LinkListQuery {
            page: params.page.max(1),
            page_size: params.page_size.clamp(1, 100),
            category_id: params.category_id,
            status,
            keyword: params.keyword.filter(|value| !value.trim().is_empty()),
        })
        .await?;
    Ok(Json(LinkPageResponse {
        items: result.items.into_iter().map(Into::into).collect(),
        total: result.total,
        page: result.page,
        page_size: result.page_size,
    }))
}

/// 创建友情链接。
#[utoipa::path(
    post,
    path = "/api/admin/links",
    tag = "Admin Links",
    operation_id = "createLink",
    summary = "创建友情链接",
    description = "`url` 仅允许 http/https，否则返回 400 `INVALID_LINK_URL`；`status` 缺省为 `active`。",
    security(("cookieAuth" = [])),
    request_body(content = LinkPayload, content_type = "application/json", description = "友情链接字段"),
    responses(
        (status = 201, description = "返回新建的友情链接", body = LinkResponse),
        (status = 400, description = "URL 非法或 status 非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn create_link(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<LinkPayload>,
) -> Result<(StatusCode, Json<LinkResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let status = request
        .status
        .as_deref()
        .map(LinkStatus::from_str)
        .transpose()?
        .unwrap_or(LinkStatus::Active);
    let link = state
        .links
        .create(NewLink {
            category_id: request.category_id,
            title: request.title,
            url: request.url,
            icon_url: request.icon_url,
            description: request.description,
            status,
            sort_order: request.sort_order,
        })
        .await?;
    write_link_audit(&state, &current, "link.created", link.id).await?;
    Ok((StatusCode::CREATED, Json(link.into())))
}

/// 获取友情链接详情。
#[utoipa::path(
    get,
    path = "/api/admin/links/{id}",
    tag = "Admin Links",
    operation_id = "getLink",
    summary = "获取友情链接详情",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "友情链接 ID")),
    responses(
        (status = 200, description = "友情链接详情", body = LinkResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "友情链接不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn get_link(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(link_id): Path<i64>,
) -> Result<Json<LinkResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let link = state
        .links
        .find(link_id)
        .await?
        .ok_or(LinkError::NotFound)?;
    Ok(Json(link.into()))
}

/// 全量更新友情链接。
#[utoipa::path(
    put,
    path = "/api/admin/links/{id}",
    tag = "Admin Links",
    operation_id = "updateLink",
    summary = "全量更新友情链接",
    description = "`url` 仅允许 http/https；`status` 缺省为 `active`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "友情链接 ID")),
    request_body(content = LinkPayload, content_type = "application/json", description = "友情链接字段"),
    responses(
        (status = 200, description = "返回更新后的友情链接", body = LinkResponse),
        (status = 400, description = "URL 非法或 status 非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "友情链接不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn update_link(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(link_id): Path<i64>,
    ApiJson(request): ApiJson<LinkPayload>,
) -> Result<Json<LinkResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let status = request
        .status
        .as_deref()
        .map(LinkStatus::from_str)
        .transpose()?
        .unwrap_or(LinkStatus::Active);
    let link = state
        .links
        .update(
            link_id,
            LinkUpdate {
                category_id: request.category_id,
                title: request.title,
                url: request.url,
                icon_url: request.icon_url,
                description: request.description,
                status,
                sort_order: request.sort_order,
            },
        )
        .await?;
    write_link_audit(&state, &current, "link.updated", link.id).await?;
    Ok(Json(link.into()))
}

/// 软删除友情链接。
#[utoipa::path(
    delete,
    path = "/api/admin/links/{id}",
    tag = "Admin Links",
    operation_id = "deleteLink",
    summary = "软删除友情链接",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "友情链接 ID")),
    responses(
        (status = 204, description = "友情链接已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "友情链接不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn delete_link(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(link_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state.links.delete(link_id).await?;
    write_link_audit(&state, &current, "link.deleted", link_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================
// Link Categories（kind = link）
// ============================================================

/// 返回 kind 为 `link` 的友链分类列表。
#[utoipa::path(
    get,
    path = "/api/admin/links/categories",
    tag = "Admin Links",
    operation_id = "listLinkCategories",
    summary = "友链分类列表",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "友链分类列表", body = Vec<super::taxonomy::CategoryResponse>),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_link_categories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<CategoryResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    let categories = state.content.list_categories(CategoryKind::Link).await?;
    Ok(Json(categories.into_iter().map(Into::into).collect()))
}

/// 创建友链分类。
#[utoipa::path(
    post,
    path = "/api/admin/links/categories",
    tag = "Admin Links",
    operation_id = "createLinkCategory",
    summary = "创建友链分类",
    description = "Slug 缺省时取 `name`，唯一冲突返回 409 `TAXONOMY_CONFLICT`。",
    security(("cookieAuth" = [])),
    request_body(content = CategoryPayload, content_type = "application/json", description = "友链分类字段"),
    responses(
        (status = 201, description = "返回新建的友链分类", body = super::taxonomy::CategoryResponse),
        (status = 400, description = "参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 409, description = "slug 冲突（TAXONOMY_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn create_link_category(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<CategoryPayload>,
) -> Result<(StatusCode, Json<CategoryResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let slug = request.slug.unwrap_or_else(|| request.name.clone());
    let category = state
        .content
        .create_category(NewCategory {
            parent_id: None,
            kind: CategoryKind::Link,
            name: request.name,
            slug,
            description: String::new(),
        })
        .await
        .map_err(map_taxonomy_error)?;
    write_link_audit(&state, &current, "link_category.created", category.id).await?;
    Ok((StatusCode::CREATED, Json(category.into())))
}

/// 更新友链分类的 name/slug。
#[utoipa::path(
    put,
    path = "/api/admin/links/categories/{id}",
    tag = "Admin Links",
    operation_id = "updateLinkCategory",
    summary = "更新友链分类",
    description = "唯一冲突返回 409 `TAXONOMY_CONFLICT`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "友链分类 ID")),
    request_body(content = CategoryPayload, content_type = "application/json", description = "友链分类字段"),
    responses(
        (status = 200, description = "返回更新后的友链分类", body = super::taxonomy::CategoryResponse),
        (status = 400, description = "参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "友链分类不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "slug 冲突（TAXONOMY_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn update_link_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(category_id): Path<i64>,
    ApiJson(request): ApiJson<CategoryPayload>,
) -> Result<Json<CategoryResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let slug = request.slug.unwrap_or_else(|| request.name.clone());
    let category = state
        .content
        .update_category(
            category_id,
            CategoryKind::Link,
            CategoryUpdate {
                name: request.name,
                slug,
            },
        )
        .await
        .map_err(map_taxonomy_error)?;
    write_link_audit(&state, &current, "link_category.updated", category.id).await?;
    Ok(Json(category.into()))
}

/// 物理删除友链分类。
#[utoipa::path(
    delete,
    path = "/api/admin/links/categories/{id}",
    tag = "Admin Links",
    operation_id = "deleteLinkCategory",
    summary = "删除友链分类",
    description = "仍存在未删除的友链引用时返回 409 `TAXONOMY_IN_USE`（FK 为 `ON DELETE SET NULL`，回收站中的友链不阻塞删除）。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "友链分类 ID")),
    responses(
        (status = 204, description = "友链分类已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "友链分类不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "分类仍被引用（TAXONOMY_IN_USE）", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn delete_link_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(category_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state
        .content
        .delete_category(category_id, CategoryKind::Link)
        .await
        .map_err(map_taxonomy_error)?;
    write_link_audit(&state, &current, "link_category.deleted", category_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn write_link_audit(
    state: &AppState,
    current: &CurrentUser,
    action: &str,
    target_id: i64,
) -> Result<(), ApiError> {
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: action.to_owned(),
            target_type: "link".to_owned(),
            target_id: Some(target_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(())
}
