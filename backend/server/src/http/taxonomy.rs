use aries_core::{
    auth::{AuditEvent, Permission},
    content::{
        Category, CategoryKind, CategoryUpdate, ContentError, NewCategory, NewTag, Tag, TagUpdate,
    },
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

use super::{auth::CurrentUser, error::ApiError};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/categories", get(list_categories).post(create_category))
        .route(
            "/categories/{id}",
            axum::routing::put(update_category).delete(delete_category),
        )
        .route("/tags", get(list_tags).post(create_tag))
        .route(
            "/tags/{id}",
            axum::routing::put(update_tag).delete(delete_tag),
        )
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(
    title = "CreateTaxonomyRequest",
    description = "创建/更新分类或标签的请求体。"
)]
struct CreateTaxonomyRequest {
    /// 显示名称；`slug` 缺省时以 name 归一化生成。
    name: String,
    /// 自定义 Slug；缺省时取 name。
    slug: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "CategoryResponse",
    description = "管理端分类 DTO；`/api/admin/categories` 固定 `kind = article`，link / gallery 分类由各自模块管理。"
)]
pub(crate) struct CategoryResponse {
    id: i64,
    parent_id: Option<i64>,
    /// 分类类型（本模块固定为 `article`）。
    kind: String,
    name: String,
    slug: String,
    description: String,
}

impl From<Category> for CategoryResponse {
    fn from(category: Category) -> Self {
        Self {
            id: category.id,
            parent_id: category.parent_id,
            kind: category.kind.as_str().to_owned(),
            name: category.name,
            slug: category.slug,
            description: category.description,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "TagResponse", description = "管理端标签 DTO。")]
struct TagResponse {
    id: i64,
    name: String,
    slug: String,
}

impl From<Tag> for TagResponse {
    fn from(tag: Tag) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
            slug: tag.slug,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/admin/categories",
    tag = "Admin Taxonomy",
    operation_id = "listCategories",
    summary = "获取文章分类列表",
    description = "管理端分类接口固定 article kind（link / gallery 分类由各自模块管理），返回可用于 Article Editor 的 Category 列表。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "分类列表", body = [CategoryResponse]),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_categories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<CategoryResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    // 管理端分类接口固定 article kind，保持现有行为（link/gallery 分类由各自模块管理）。
    let categories = state.content.list_categories(CategoryKind::Article).await?;
    Ok(Json(categories.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    post,
    path = "/api/admin/categories",
    tag = "Admin Taxonomy",
    operation_id = "createCategory",
    summary = "新建文章分类",
    description = "创建 article kind 分类；slug 缺省时以 name 归一化生成，唯一冲突返回 409 `TAXONOMY_CONFLICT`。",
    security(("cookieAuth" = [])),
    request_body(content = CreateTaxonomyRequest, description = "分类名称与可选 Slug"),
    responses(
        (status = 201, description = "返回新建的分类", body = CategoryResponse),
        (status = 400, description = "参数校验失败", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 409, description = "name 或 slug 已存在（`TAXONOMY_CONFLICT`）", body = crate::openapi::ErrorResponse),
    )
)]
async fn create_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<CreateTaxonomyRequest>,
) -> Result<(StatusCode, Json<CategoryResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let slug = request.slug.unwrap_or_else(|| request.name.clone());
    let category = state
        .content
        .create_category(NewCategory {
            parent_id: None,
            kind: CategoryKind::Article,
            name: request.name,
            slug,
            description: String::new(),
        })
        .await
        .map_err(map_taxonomy_error)?;
    write_taxonomy_audit(
        &state,
        &current,
        "category.created",
        "category",
        category.id,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(category.into())))
}

#[utoipa::path(
    get,
    path = "/api/admin/tags",
    tag = "Admin Taxonomy",
    operation_id = "listTags",
    summary = "获取标签列表",
    description = "返回可用于 Article Editor 的 Tag 列表。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "标签列表", body = [TagResponse]),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_tags(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<TagResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    let tags = state.content.list_tags().await?;
    Ok(Json(tags.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    post,
    path = "/api/admin/tags",
    tag = "Admin Taxonomy",
    operation_id = "createTag",
    summary = "新建标签",
    description = "slug 缺省时以 name 归一化生成，唯一冲突返回 409 `TAXONOMY_CONFLICT`。",
    security(("cookieAuth" = [])),
    request_body(content = CreateTaxonomyRequest, description = "标签名称与可选 Slug"),
    responses(
        (status = 201, description = "返回新建的标签", body = TagResponse),
        (status = 400, description = "参数校验失败", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 409, description = "name 或 slug 已存在（`TAXONOMY_CONFLICT`）", body = crate::openapi::ErrorResponse),
    )
)]
async fn create_tag(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<CreateTaxonomyRequest>,
) -> Result<(StatusCode, Json<TagResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let slug = request.slug.unwrap_or_else(|| request.name.clone());
    let tag = state
        .content
        .create_tag(NewTag {
            name: request.name,
            slug,
        })
        .await
        .map_err(map_taxonomy_error)?;
    write_taxonomy_audit(&state, &current, "tag.created", "tag", tag.id).await?;
    Ok((StatusCode::CREATED, Json(tag.into())))
}

#[utoipa::path(
    put,
    path = "/api/admin/categories/{id}",
    tag = "Admin Taxonomy",
    operation_id = "updateCategory",
    summary = "更新文章分类",
    description = "更新分类的 `name` / `slug`；slug 复用统一 Normalize 规则，唯一冲突返回 409 `TAXONOMY_CONFLICT`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "分类 ID")),
    request_body(content = CreateTaxonomyRequest, description = "分类名称与可选 Slug"),
    responses(
        (status = 200, description = "返回更新后的分类", body = CategoryResponse),
        (status = 400, description = "参数校验失败", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "分类不存在（`TAXONOMY_NOT_FOUND`）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "name 或 slug 已存在（`TAXONOMY_CONFLICT`）", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(category_id): Path<i64>,
    Json(request): Json<CreateTaxonomyRequest>,
) -> Result<Json<CategoryResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let slug = request.slug.unwrap_or_else(|| request.name.clone());
    let category = state
        .content
        .update_category(
            category_id,
            CategoryKind::Article,
            CategoryUpdate {
                name: request.name,
                slug,
            },
        )
        .await
        .map_err(map_taxonomy_error)?;
    write_taxonomy_audit(
        &state,
        &current,
        "category.updated",
        "category",
        category.id,
    )
    .await?;
    Ok(Json(category.into()))
}

#[utoipa::path(
    delete,
    path = "/api/admin/categories/{id}",
    tag = "Admin Taxonomy",
    operation_id = "deleteCategory",
    summary = "删除文章分类",
    description = "仍被 Article 引用时返回 409，并在 `error.details` 携带引用信息。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "分类 ID")),
    responses(
        (status = 204, description = "分类已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "分类不存在（`TAXONOMY_NOT_FOUND`）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "分类仍被引用", body = crate::openapi::ErrorResponse),
    )
)]
async fn delete_category(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(category_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state
        .content
        .delete_category(category_id, CategoryKind::Article)
        .await
        .map_err(map_taxonomy_error)?;
    write_taxonomy_audit(
        &state,
        &current,
        "category.deleted",
        "category",
        category_id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/admin/tags/{id}",
    tag = "Admin Taxonomy",
    operation_id = "updateTag",
    summary = "更新标签",
    description = "更新标签的 `name` / `slug`；唯一冲突返回 409 `TAXONOMY_CONFLICT`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "标签 ID")),
    request_body(content = CreateTaxonomyRequest, description = "标签名称与可选 Slug"),
    responses(
        (status = 200, description = "返回更新后的标签", body = TagResponse),
        (status = 400, description = "参数校验失败", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "标签不存在（`TAXONOMY_NOT_FOUND`）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "name 或 slug 已存在（`TAXONOMY_CONFLICT`）", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_tag(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(tag_id): Path<i64>,
    Json(request): Json<CreateTaxonomyRequest>,
) -> Result<Json<TagResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let slug = request.slug.unwrap_or_else(|| request.name.clone());
    let tag = state
        .content
        .update_tag(
            tag_id,
            TagUpdate {
                name: request.name,
                slug,
            },
        )
        .await
        .map_err(map_taxonomy_error)?;
    write_taxonomy_audit(&state, &current, "tag.updated", "tag", tag.id).await?;
    Ok(Json(tag.into()))
}

#[utoipa::path(
    delete,
    path = "/api/admin/tags/{id}",
    tag = "Admin Taxonomy",
    operation_id = "deleteTag",
    summary = "删除标签",
    description = "仍被 Article 引用时返回 409，并在 `error.details` 携带引用信息。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "标签 ID")),
    responses(
        (status = 204, description = "标签已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "标签不存在（`TAXONOMY_NOT_FOUND`）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "标签仍被引用", body = crate::openapi::ErrorResponse),
    )
)]
async fn delete_tag(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(tag_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state
        .content
        .delete_tag(tag_id)
        .await
        .map_err(map_taxonomy_error)?;
    write_taxonomy_audit(&state, &current, "tag.deleted", "tag", tag_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn write_taxonomy_audit(
    state: &AppState,
    current: &CurrentUser,
    action: &str,
    target_type: &str,
    target_id: i64,
) -> Result<(), ApiError> {
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: action.to_owned(),
            target_type: target_type.to_owned(),
            target_id: Some(target_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(())
}

pub(super) fn map_taxonomy_error(error: ContentError) -> ApiError {
    match error {
        ContentError::Conflict => {
            ApiError::conflict("TAXONOMY_CONFLICT", "Taxonomy name or slug already exists")
        }
        ContentError::NotFound => {
            ApiError::not_found("TAXONOMY_NOT_FOUND", "Taxonomy was not found")
        }
        other => other.into(),
    }
}
