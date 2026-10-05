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
    extract::ApiJson,
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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct GalleryListParams {
    /// 页码，从 1 开始。
    #[serde(default = "super::default_page")]
    page: u32,
    /// 每页条数（1–100）。
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 状态过滤：`draft` / `published`。
    status: Option<String>,
    /// 按图库分类（kind = `gallery`）过滤。
    category_id: Option<i64>,
    /// 对标题与 Slug 做模糊匹配；空白值被忽略。
    keyword: Option<String>,
}

/// 图库创建 / 全量更新请求体。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct GalleryPayload {
    /// 分类 ID，必须指向 kind 为 `gallery` 的分类。
    category_id: i64,
    /// URL 友好的唯一 Slug。
    slug: String,
    title: String,
    #[serde(default)]
    description: String,
    /// 封面媒体资产 ID。
    cover_media_id: Option<i64>,
    /// 状态：`draft` / `published`；缺省为 `draft`。
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    sort_order: i32,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GalleryResponse {
    id: i64,
    category_id: i64,
    slug: String,
    title: String,
    description: String,
    cover_media_id: Option<i64>,
    /// 状态：`draft` / `published`。
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GalleryPageResponse {
    items: Vec<GalleryResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

/// 内联媒体摘要：软删除资产为 null，width/height 探测失败为 null。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GalleryItemMediaResponse {
    id: i64,
    url: String,
    alt: String,
    width: Option<i32>,
    height: Option<i32>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GalleryItemResponse {
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

/// 添加图库条目请求体。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct GalleryItemPayload {
    /// 媒体资产 ID；同一媒体在同一图库中只可出现一次。
    media_asset_id: i64,
    /// 图片替代文本。
    #[serde(default)]
    alt: String,
    /// 拍摄地点。
    #[serde(default)]
    location: String,
    #[serde(default)]
    sort_order: i32,
}

/// 更新图库条目请求体（媒体引用创建后不可改）。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct GalleryItemUpdatePayload {
    /// 图片替代文本。
    #[serde(default)]
    alt: String,
    /// 拍摄地点。
    #[serde(default)]
    location: String,
    #[serde(default)]
    sort_order: i32,
}

/// 原子批量重排请求体：按 `item_ids` 顺序重写 `sort_order`。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct GalleryItemOrderPayload {
    /// 条目 ID 有序列表；任一 ID 不属于该图库则整体回滚。
    item_ids: Vec<i64>,
}

/// 图库分类创建 / 更新请求体。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct CategoryPayload {
    name: String,
    /// Slug 缺省时取 `name`。
    slug: Option<String>,
}

// ============================================================
// Gallery CRUD
// ============================================================

/// 分页查询图库列表。
#[utoipa::path(
    get,
    path = "/api/admin/galleries",
    tag = "Admin Galleries",
    operation_id = "listGalleries",
    summary = "分页查询图库列表",
    description = "支持按状态、分类与关键字过滤；稳定排序 `sort_order ASC, id ASC`，不含已软删除记录。",
    security(("cookieAuth" = [])),
    params(GalleryListParams),
    responses(
        (status = 200, description = "图库分页结果", body = GalleryPageResponse),
        (status = 400, description = "状态参数无效", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_galleries(
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

/// 创建图库。
#[utoipa::path(
    post,
    path = "/api/admin/galleries",
    tag = "Admin Galleries",
    operation_id = "createGallery",
    summary = "创建图库",
    description = "`category_id` 必须指向 kind 为 `gallery` 的分类；Slug 冲突返回 409 `GALLERY_CONFLICT`。",
    security(("cookieAuth" = [])),
    request_body(content = GalleryPayload, description = "图库创建参数"),
    responses(
        (status = 201, description = "返回新建的图库", body = GalleryResponse),
        (status = 400, description = "请求参数无效", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 409, description = "Slug 或媒体资产冲突", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn create_gallery(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<GalleryPayload>,
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

/// 获取图库详情。
#[utoipa::path(
    get,
    path = "/api/admin/galleries/{id}",
    tag = "Admin Galleries",
    operation_id = "getGallery",
    summary = "获取图库详情",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "图库 ID")),
    responses(
        (status = 200, description = "图库详情", body = GalleryResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "图库不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn get_gallery(
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

/// 全量更新图库。
#[utoipa::path(
    put,
    path = "/api/admin/galleries/{id}",
    tag = "Admin Galleries",
    operation_id = "updateGallery",
    summary = "全量更新图库",
    description = "Slug 冲突返回 409 `GALLERY_CONFLICT`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "图库 ID")),
    request_body(content = GalleryPayload, description = "图库全量更新参数"),
    responses(
        (status = 200, description = "返回更新后的图库", body = GalleryResponse),
        (status = 400, description = "请求参数无效", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "图库不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "Slug 或媒体资产冲突", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn update_gallery(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
    ApiJson(request): ApiJson<GalleryPayload>,
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

/// 软删除图库。
#[utoipa::path(
    delete,
    path = "/api/admin/galleries/{id}",
    tag = "Admin Galleries",
    operation_id = "deleteGallery",
    summary = "软删除图库",
    description = "软删除图库；其条目一并解除引用。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "图库 ID")),
    responses(
        (status = 204, description = "图库已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "图库不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn delete_gallery(
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

/// 查询图库条目列表。
#[utoipa::path(
    get,
    path = "/api/admin/galleries/{id}/items",
    tag = "Admin Galleries",
    operation_id = "listGalleryItems",
    summary = "查询图库条目列表",
    description = "稳定排序 `sort_order ASC, id ASC`；图库不存在返回 404。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "图库 ID")),
    responses(
        (status = 200, description = "图库条目列表", body = Vec<GalleryItemResponse>),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "图库不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_gallery_items(
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

/// 向图库添加媒体条目。
#[utoipa::path(
    post,
    path = "/api/admin/galleries/{id}/items",
    tag = "Admin Galleries",
    operation_id = "addGalleryItem",
    summary = "向图库添加媒体条目",
    description = "同一媒体在同一图库中只可出现一次，重复返回 409 `GALLERY_CONFLICT`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "图库 ID")),
    request_body(content = GalleryItemPayload, description = "条目添加参数"),
    responses(
        (status = 201, description = "返回新建的图库条目", body = GalleryItemResponse),
        (status = 400, description = "请求参数无效", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "图库或媒体资产不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "同一媒体已存在于该图库", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn add_gallery_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
    ApiJson(request): ApiJson<GalleryItemPayload>,
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

/// 更新图库条目。
#[utoipa::path(
    put,
    path = "/api/admin/galleries/{id}/items/{item_id}",
    tag = "Admin Galleries",
    operation_id = "updateGalleryItem",
    summary = "更新图库条目",
    description = "更新条目的 `alt` / `location` / `sort_order`；媒体引用创建后不可改。",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "图库 ID"),
        ("item_id" = i64, Path, description = "条目 ID"),
    ),
    request_body(content = GalleryItemUpdatePayload, description = "条目更新参数"),
    responses(
        (status = 200, description = "返回更新后的图库条目", body = GalleryItemResponse),
        (status = 400, description = "请求参数无效", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "图库或条目不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn update_gallery_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((gallery_id, item_id)): Path<(i64, i64)>,
    ApiJson(request): ApiJson<GalleryItemUpdatePayload>,
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

/// 物理移除图库条目。
#[utoipa::path(
    delete,
    path = "/api/admin/galleries/{id}/items/{item_id}",
    tag = "Admin Galleries",
    operation_id = "removeGalleryItem",
    summary = "物理移除图库条目",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "图库 ID"),
        ("item_id" = i64, Path, description = "条目 ID"),
    ),
    responses(
        (status = 204, description = "条目已移除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "条目不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn remove_gallery_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((gallery_id, item_id)): Path<(i64, i64)>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state.galleries.remove_item(item_id).await?;
    write_gallery_audit(&state, &current, "gallery.item_removed", gallery_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 原子批量重排图库条目。
#[utoipa::path(
    put,
    path = "/api/admin/galleries/{id}/items/order",
    tag = "Admin Galleries",
    operation_id = "reorderGalleryItems",
    summary = "原子批量重排图库条目",
    description = "事务内按 `item_ids` 顺序重写 `sort_order`，任一 ID 不属于该图库则整体回滚。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "图库 ID")),
    request_body(content = GalleryItemOrderPayload, description = "有序条目 ID 列表"),
    responses(
        (status = 204, description = "排序已应用"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "图库或条目不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn reorder_gallery_items(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(gallery_id): Path<i64>,
    ApiJson(request): ApiJson<GalleryItemOrderPayload>,
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

/// 查询图库分类列表。
#[utoipa::path(
    get,
    path = "/api/admin/galleries/categories",
    tag = "Admin Galleries",
    operation_id = "listGalleryCategories",
    summary = "查询图库分类列表",
    description = "返回 kind 为 `gallery` 的分类列表。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "图库分类列表", body = Vec<CategoryResponse>),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_gallery_categories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<CategoryResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    let categories = state.content.list_categories(CategoryKind::Gallery).await?;
    Ok(Json(categories.into_iter().map(Into::into).collect()))
}

/// 创建图库分类。
#[utoipa::path(
    post,
    path = "/api/admin/galleries/categories",
    tag = "Admin Galleries",
    operation_id = "createGalleryCategory",
    summary = "创建图库分类",
    description = "创建 kind 为 `gallery` 的分类；Slug 缺省时取 `name`，唯一冲突返回 409 `TAXONOMY_CONFLICT`。",
    security(("cookieAuth" = [])),
    request_body(content = CategoryPayload, description = "分类创建参数"),
    responses(
        (status = 201, description = "返回新建的图库分类", body = CategoryResponse),
        (status = 400, description = "分类名无效", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 409, description = "Slug 唯一冲突", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn create_gallery_category(
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

/// 更新图库分类。
#[utoipa::path(
    put,
    path = "/api/admin/galleries/categories/{id}",
    tag = "Admin Galleries",
    operation_id = "updateGalleryCategory",
    summary = "更新图库分类",
    description = "更新图库分类的 `name` / `slug`；唯一冲突返回 409 `TAXONOMY_CONFLICT`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "分类 ID")),
    request_body(content = CategoryPayload, description = "分类更新参数"),
    responses(
        (status = 200, description = "返回更新后的图库分类", body = CategoryResponse),
        (status = 400, description = "分类名无效", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "分类不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "Slug 唯一冲突", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn update_gallery_category(
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

/// 物理删除图库分类。
#[utoipa::path(
    delete,
    path = "/api/admin/galleries/categories/{id}",
    tag = "Admin Galleries",
    operation_id = "deleteGalleryCategory",
    summary = "物理删除图库分类",
    description = "仍被 Gallery 引用时返回 409 `TAXONOMY_IN_USE`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "分类 ID")),
    responses(
        (status = 204, description = "图库分类已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "分类不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "分类仍被图库引用", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn delete_gallery_category(
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
