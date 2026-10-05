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

use super::{auth::CurrentUser, error::ApiError, extract::ApiJson};

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

/// 导航节点创建/更新请求体。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct NavigationPayload {
    /// 父节点 ID；最多两级，超出返回 400 `INVALID_NAVIGATION_HIERARCHY`。
    parent_id: Option<i64>,
    /// 菜单文案。
    label: String,
    /// `article` / `page` / `category` / `url`。
    target_type: String,
    /// 内部目标 ID；`target_type` 非 `url` 时必填（400 `INVALID_NAVIGATION_TARGET`）。
    target_id: Option<i64>,
    /// 外链地址；`target_type` 为 `url` 时必填。
    url: Option<String>,
    #[serde(default)]
    open_in_new_tab: bool,
    #[serde(default = "default_visible")]
    visible: bool,
    #[serde(default)]
    sort_order: i32,
}

/// 导航节点响应体（平铺，前端按 `parent_id` 组树）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct NavigationItemResponse {
    id: i64,
    parent_id: Option<i64>,
    label: String,
    /// `article` / `page` / `category` / `url`。
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

/// 导航排序请求体。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct NavigationOrderPayload {
    /// 全部导航节点 ID 的目标顺序；服务端在事务内原子重写 `sort_order`，任一 ID 不存在则整体回滚。
    item_ids: Vec<i64>,
}

/// 平铺返回全部导航节点（含隐藏项），前端按 `parent_id` 组树。
#[utoipa::path(
    get,
    path = "/api/admin/navigation",
    tag = "Admin Navigation",
    operation_id = "listNavigation",
    summary = "导航节点列表",
    description = "稳定排序 `parent_id NULLS FIRST, sort_order ASC, id ASC`。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "导航节点列表", body = Vec<NavigationItemResponse>),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_navigation(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<NavigationItemResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    let items = state.navigation.list().await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

/// 创建导航节点。
#[utoipa::path(
    post,
    path = "/api/admin/navigation",
    tag = "Admin Navigation",
    operation_id = "createNavigationItem",
    summary = "创建导航节点",
    description = "最多两级，超出返回 400 `INVALID_NAVIGATION_HIERARCHY`；`target_type` 为 `url` 时必须提供 `url`，其余类型必须提供 `target_id`（400 `INVALID_NAVIGATION_TARGET`）。",
    security(("cookieAuth" = [])),
    request_body(content = NavigationPayload, content_type = "application/json", description = "导航节点字段"),
    responses(
        (status = 201, description = "返回新建的导航节点", body = NavigationItemResponse),
        (status = 400, description = "层级或目标字段非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn create_navigation_item(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<NavigationPayload>,
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

/// 全量更新导航节点。
#[utoipa::path(
    put,
    path = "/api/admin/navigation/{id}",
    tag = "Admin Navigation",
    operation_id = "updateNavigationItem",
    summary = "全量更新导航节点",
    description = "层级与目标字段校验同创建。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "导航节点 ID")),
    request_body(content = NavigationPayload, content_type = "application/json", description = "导航节点字段"),
    responses(
        (status = 200, description = "返回更新后的导航节点", body = NavigationItemResponse),
        (status = 400, description = "层级或目标字段非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "导航节点不存在", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn update_navigation_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(item_id): Path<i64>,
    ApiJson(request): ApiJson<NavigationPayload>,
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

/// 物理删除导航节点。
#[utoipa::path(
    delete,
    path = "/api/admin/navigation/{id}",
    tag = "Admin Navigation",
    operation_id = "deleteNavigationItem",
    summary = "删除导航节点",
    description = "仍含子节点时返回 409 `NAVIGATION_CONFLICT`，须先删除或移动子节点。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "导航节点 ID")),
    responses(
        (status = 204, description = "导航节点已删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "导航节点不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "仍含子节点（NAVIGATION_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn delete_navigation_item(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(item_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    state.navigation.delete(item_id).await?;
    write_navigation_audit(&state, &current, "navigation.deleted", item_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 原子批量重排导航节点。
#[utoipa::path(
    put,
    path = "/api/admin/navigation/order",
    tag = "Admin Navigation",
    operation_id = "reorderNavigation",
    summary = "批量重排导航节点",
    description = "事务内按 `item_ids` 顺序重写 `sort_order`，任一 ID 不存在则整体回滚并返回 404 `NAVIGATION_NOT_FOUND`。",
    security(("cookieAuth" = [])),
    request_body(content = NavigationOrderPayload, content_type = "application/json", description = "目标顺序"),
    responses(
        (status = 204, description = "排序已应用"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "任一导航节点不存在（NAVIGATION_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn reorder_navigation(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<NavigationOrderPayload>,
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
