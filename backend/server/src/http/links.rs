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

#[derive(Debug, Deserialize)]
struct LinkListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    status: Option<String>,
    category_id: Option<i64>,
    keyword: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LinkPayload {
    category_id: Option<i64>,
    title: String,
    url: String,
    icon_url: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    sort_order: i32,
}

#[derive(Debug, Serialize)]
struct LinkResponse {
    id: i64,
    category_id: Option<i64>,
    title: String,
    url: String,
    icon_url: Option<String>,
    description: String,
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

#[derive(Debug, Serialize)]
struct LinkPageResponse {
    items: Vec<LinkResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

#[derive(Debug, Deserialize)]
struct CategoryPayload {
    name: String,
    slug: Option<String>,
}

async fn list_links(
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

async fn create_link(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<LinkPayload>,
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

async fn get_link(
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

async fn update_link(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(link_id): Path<i64>,
    Json(request): Json<LinkPayload>,
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

async fn delete_link(
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

async fn list_link_categories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<CategoryResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    let categories = state.content.list_categories(CategoryKind::Link).await?;
    Ok(Json(categories.into_iter().map(Into::into).collect()))
}

async fn create_link_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<CategoryPayload>,
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

async fn update_link_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(category_id): Path<i64>,
    Json(request): Json<CategoryPayload>,
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

async fn delete_link_category(
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
