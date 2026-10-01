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

#[derive(Debug, Deserialize)]
struct CreateTaxonomyRequest {
    name: String,
    slug: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct CategoryResponse {
    id: i64,
    parent_id: Option<i64>,
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

#[derive(Debug, Serialize)]
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

async fn list_categories(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<CategoryResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    // 管理端分类接口固定 article kind，保持现有行为（link/gallery 分类由各自模块管理）。
    let categories = state.content.list_categories(CategoryKind::Article).await?;
    Ok(Json(categories.into_iter().map(Into::into).collect()))
}

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

async fn list_tags(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<TagResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    let tags = state.content.list_tags().await?;
    Ok(Json(tags.into_iter().map(Into::into).collect()))
}

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
