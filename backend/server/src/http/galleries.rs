//! Admin 图库管理：Gallery CRUD + 分页、条目增删改与原子排序、图库分类管理。
//! 图库分类复用 categories 表（kind = 'gallery'），删除沿用引用保护 + 物理删除惯例。

use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    content::{CategoryKind, CategoryUpdate, NewCategory},
    galleries::{
        Gallery, GalleryError, GalleryItemDetail, GalleryItemUpdate, GalleryListQuery,
        GalleryStatus, GalleryUpdate, NewGallery, NewGalleryItem,
    },
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
        .route("/galleries", get(list_galleries).post(create_gallery))
        .route(
            "/galleries/categories",
            get(list_gallery_categories).post(create_gallery_category),
        )
        .route(
            "/galleries/categories/{id}",
            put(update_gallery_category).delete(delete_gallery_category),
        )
        .route(
            "/galleries/{id}",
            get(get_gallery).put(update_gallery).delete(delete_gallery),
        )
        .route(
            "/galleries/{id}/items",
            get(list_gallery_items).post(add_gallery_item),
        )
        .route("/galleries/{id}/items/order", put(reorder_gallery_items))
        .route(
            "/galleries/{id}/items/{item_id}",
            put(update_gallery_item).delete(remove_gallery_item),
        )
}

// ============================================================
// DTO
// ============================================================

#[derive(Debug, Deserialize)]
struct GalleryListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    status: Option<String>,
    category_id: Option<i64>,
    keyword: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GalleryPayload {
    category_id: i64,
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    cover_media_id: Option<i64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    sort_order: i32,
}

#[derive(Debug, Serialize)]
struct GalleryResponse {
    id: i64,
    category_id: i64,
    slug: String,
    title: String,
    description: String,
    cover_media_id: Option<i64>,
    status: String,
    sort_order: i32,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<Gallery> for GalleryResponse {
    fn from(gallery: Gallery) -> Self {
        Self {
            id: gallery.id,
            category_id: gallery.category_id,
            slug: gallery.slug,
            title: gallery.title,
            description: gallery.description,
            cover_media_id: gallery.cover_media_id,
            status: gallery.status.as_str().to_owned(),
            sort_order: gallery.sort_order,
            created_at: gallery.created_at,
            updated_at: gallery.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct GalleryPageResponse {
    items: Vec<GalleryResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

/// 内联媒体摘要：软删除资产为 null，width/height 探测失败为 null。
#[derive(Debug, Serialize)]
struct GalleryItemMediaResponse {
    id: i64,
    url: String,
    alt: String,
    width: Option<i32>,
    height: Option<i32>,
}

#[derive(Debug, Serialize)]
struct GalleryItemResponse {
    id: i64,
    gallery_id: i64,
    media_asset_id: i64,
    alt: String,
    location: String,
    sort_order: i32,
    created_at: OffsetDateTime,
    media: Option<GalleryItemMediaResponse>,
}

impl From<GalleryItemDetail> for GalleryItemResponse {
    fn from(detail: GalleryItemDetail) -> Self {
        let item = detail.item;
        Self {
            id: item.id,
            gallery_id: item.gallery_id,
            media_asset_id: item.media_asset_id,
            alt: item.alt,
            location: item.location,
            sort_order: item.sort_order,
            created_at: item.created_at,
            media: detail.media.map(|media| GalleryItemMediaResponse {
                id: media.id,
                url: media.url,
                alt: media.alt,
                width: media.width,
                height: media.height,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GalleryItemPayload {
    media_asset_id: i64,
    #[serde(default)]
    alt: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    sort_order: i32,
}

#[derive(Debug, Deserialize)]
struct GalleryItemUpdatePayload {
    #[serde(default)]
    alt: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    sort_order: i32,
}

#[derive(Debug, Deserialize)]
struct GalleryItemOrderPayload {
    item_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
struct CategoryPayload {
    name: String,
    slug: Option<String>,
}

// ============================================================
// Gallery CRUD
// ============================================================

async fn list_galleries(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<GalleryListParams>,
) -> Result<Json<GalleryPageResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let status = params
        .status
        .as_deref()
        .map(GalleryStatus::from_str)
        .transpose()?;
    let result = state
        .galleries
        .list_galleries(GalleryListQuery {
            page: params.page.max(1),
            page_size: params.page_size.clamp(1, 100),
            category_id: params.category_id,
            status,
            keyword: params.keyword.filter(|value| !value.trim().is_empty()),
        })
        .await?;
    Ok(Json(GalleryPageResponse {
        items: result.items.into_iter().map(Into::into).collect(),
        total: result.total,
        page: result.page,
        page_size: result.page_size,
    }))
}

async fn create_gallery(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<GalleryPayload>,
) -> Result<(StatusCode, Json<GalleryResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let status = request
        .status
        .as_deref()
        .map(GalleryStatus::from_str)
        .transpose()?
        .unwrap_or(GalleryStatus::Draft);
    let gallery = state
        .galleries
        .create_gallery(NewGallery {
            category_id: request.category_id,
            slug: request.slug,
            title: request.title,
            description: request.description,
            cover_media_id: request.cover_media_id,
            status,
            sort_order: request.sort_order,
        })
        .await?;
    write_gallery_audit(&state, &current, "gallery.created", gallery.id).await?;
    Ok((StatusCode::CREATED, Json(gallery.into())))
}

async fn get_gallery(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
) -> Result<Json<GalleryResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let gallery = state
        .galleries
        .find_gallery(gallery_id)
        .await?
        .ok_or(GalleryError::NotFound)?;
    Ok(Json(gallery.into()))
}

async fn update_gallery(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
    Json(request): Json<GalleryPayload>,
) -> Result<Json<GalleryResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let status = request
        .status
        .as_deref()
        .map(GalleryStatus::from_str)
        .transpose()?
        .unwrap_or(GalleryStatus::Draft);
    let gallery = state
        .galleries
        .update_gallery(
            gallery_id,
            GalleryUpdate {
                category_id: request.category_id,
                slug: request.slug,
                title: request.title,
                description: request.description,
                cover_media_id: request.cover_media_id,
                status,
                sort_order: request.sort_order,
            },
        )
        .await?;
    write_gallery_audit(&state, &current, "gallery.updated", gallery.id).await?;
    Ok(Json(gallery.into()))
}

async fn delete_gallery(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state.galleries.delete_gallery(gallery_id).await?;
    write_gallery_audit(&state, &current, "gallery.deleted", gallery_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================
// Gallery Items
// ============================================================

async fn list_gallery_items(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
) -> Result<Json<Vec<GalleryItemResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    // 先确认图库存在，避免对不存在图库返回空列表造成歧义。
    state
        .galleries
        .find_gallery(gallery_id)
        .await?
        .ok_or(GalleryError::NotFound)?;
    let items = state.galleries.list_items(gallery_id).await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

async fn add_gallery_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
    Json(request): Json<GalleryItemPayload>,
) -> Result<(StatusCode, Json<GalleryItemResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let item = state
        .galleries
        .add_item(NewGalleryItem {
            gallery_id,
            media_asset_id: request.media_asset_id,
            alt: request.alt,
            location: request.location,
            sort_order: request.sort_order,
        })
        .await?;
    write_gallery_audit(&state, &current, "gallery.item_added", gallery_id).await?;
    Ok((StatusCode::CREATED, Json(item.into())))
}

async fn update_gallery_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((gallery_id, item_id)): Path<(i64, i64)>,
    Json(request): Json<GalleryItemUpdatePayload>,
) -> Result<Json<GalleryItemResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let item = state
        .galleries
        .update_item(
            item_id,
            GalleryItemUpdate {
                alt: request.alt,
                location: request.location,
                sort_order: request.sort_order,
            },
        )
        .await?;
    write_gallery_audit(&state, &current, "gallery.item_updated", gallery_id).await?;
    Ok(Json(item.into()))
}

async fn remove_gallery_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((gallery_id, item_id)): Path<(i64, i64)>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state.galleries.remove_item(item_id).await?;
    write_gallery_audit(&state, &current, "gallery.item_removed", gallery_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reorder_gallery_items(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
    Json(request): Json<GalleryItemOrderPayload>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state
        .galleries
        .reorder_items(gallery_id, request.item_ids)
        .await?;
    write_gallery_audit(&state, &current, "gallery.items_reordered", gallery_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================
// Gallery Categories（kind = gallery）
// ============================================================

async fn list_gallery_categories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<CategoryResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    let categories = state.content.list_categories(CategoryKind::Gallery).await?;
    Ok(Json(categories.into_iter().map(Into::into).collect()))
}

async fn create_gallery_category(
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
            kind: CategoryKind::Gallery,
            name: request.name,
            slug,
            description: String::new(),
        })
        .await
        .map_err(map_taxonomy_error)?;
    write_gallery_audit(&state, &current, "gallery_category.created", category.id).await?;
    Ok((StatusCode::CREATED, Json(category.into())))
}

async fn update_gallery_category(
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
            CategoryKind::Gallery,
            CategoryUpdate {
                name: request.name,
                slug,
            },
        )
        .await
        .map_err(map_taxonomy_error)?;
    write_gallery_audit(&state, &current, "gallery_category.updated", category.id).await?;
    Ok(Json(category.into()))
}

async fn delete_gallery_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(category_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state
        .content
        .delete_category(category_id, CategoryKind::Gallery)
        .await
        .map_err(map_taxonomy_error)?;
    write_gallery_audit(&state, &current, "gallery_category.deleted", category_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn write_gallery_audit(
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
            target_type: "gallery".to_owned(),
            target_id: Some(target_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(())
}
