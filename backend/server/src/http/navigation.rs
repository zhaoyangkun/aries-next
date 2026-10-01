//! Admin 导航菜单管理：两级树（平铺返回，前端组树）、增删改、原子批量排序。

use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    navigation::{NavigationItem, NavigationItemUpdate, NavigationTargetType, NewNavigationItem},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::state::AppState;

use super::{auth::CurrentUser, error::ApiError};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/navigation",
            get(list_navigation).post(create_navigation_item),
        )
        .route("/navigation/order", put(reorder_navigation))
        .route(
            "/navigation/{id}",
            put(update_navigation_item).delete(delete_navigation_item),
        )
}

#[derive(Debug, Deserialize)]
struct NavigationPayload {
    parent_id: Option<i64>,
    label: String,
    target_type: String,
    target_id: Option<i64>,
    url: Option<String>,
    #[serde(default)]
    open_in_new_tab: bool,
    #[serde(default = "default_visible")]
    visible: bool,
    #[serde(default)]
    sort_order: i32,
}

#[derive(Debug, Serialize)]
struct NavigationItemResponse {
    id: i64,
    parent_id: Option<i64>,
    label: String,
    target_type: String,
    target_id: Option<i64>,
    url: Option<String>,
    open_in_new_tab: bool,
    visible: bool,
    sort_order: i32,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<NavigationItem> for NavigationItemResponse {
    fn from(item: NavigationItem) -> Self {
        Self {
            id: item.id,
            parent_id: item.parent_id,
            label: item.label,
            target_type: item.target_type.as_str().to_owned(),
            target_id: item.target_id,
            url: item.url,
            open_in_new_tab: item.open_in_new_tab,
            visible: item.visible,
            sort_order: item.sort_order,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct NavigationOrderPayload {
    item_ids: Vec<i64>,
}

async fn list_navigation(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<NavigationItemResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    let items = state.navigation.list().await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

async fn create_navigation_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<NavigationPayload>,
) -> Result<(StatusCode, Json<NavigationItemResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let target_type = NavigationTargetType::from_str(&request.target_type)?;
    let item = state
        .navigation
        .create(NewNavigationItem {
            parent_id: request.parent_id,
            label: request.label,
            target_type,
            target_id: request.target_id,
            url: request.url,
            open_in_new_tab: request.open_in_new_tab,
            visible: request.visible,
            sort_order: request.sort_order,
        })
        .await?;
    write_navigation_audit(&state, &current, "navigation.created", item.id).await?;
    Ok((StatusCode::CREATED, Json(item.into())))
}

async fn update_navigation_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(item_id): Path<i64>,
    Json(request): Json<NavigationPayload>,
) -> Result<Json<NavigationItemResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let target_type = NavigationTargetType::from_str(&request.target_type)?;
    let item = state
        .navigation
        .update(
            item_id,
            NavigationItemUpdate {
                parent_id: request.parent_id,
                label: request.label,
                target_type,
                target_id: request.target_id,
                url: request.url,
                open_in_new_tab: request.open_in_new_tab,
                visible: request.visible,
                sort_order: request.sort_order,
            },
        )
        .await?;
    write_navigation_audit(&state, &current, "navigation.updated", item.id).await?;
    Ok(Json(item.into()))
}

async fn delete_navigation_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(item_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state.navigation.delete(item_id).await?;
    write_navigation_audit(&state, &current, "navigation.deleted", item_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reorder_navigation(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<NavigationOrderPayload>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    // 事务内原子重写 sort_order；任一 id 不存在则整体回滚。
    state.navigation.reorder(request.item_ids).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "navigation.reordered".to_owned(),
            target_type: "navigation".to_owned(),
            target_id: None,
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn write_navigation_audit(
    state: &AppState,
    current: &CurrentUser,
    action: &str,
    item_id: i64,
) -> Result<(), ApiError> {
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: action.to_owned(),
            target_type: "navigation".to_owned(),
            target_id: Some(item_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(())
}

const fn default_visible() -> bool {
    true
}
