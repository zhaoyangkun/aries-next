//! HTTP 层模块注册与 Admin 路由装配。
//!
//! `admin_router` 汇总全部 `/api/admin/*` 子模块，并统一挂载 `enforce_origin`
//! 中间件；`public/` 下的匿名端点不经此中间件。

pub mod admin_spa;
pub mod ai;
pub mod articles;
pub mod audit;
pub mod auth;
pub mod comments;
pub mod dashboard;
pub mod email;
pub mod error;
pub mod extract;
pub mod galleries;
pub mod journals;
pub mod links;
pub mod logs;
pub mod media;
pub mod navigation;
pub mod pages;
pub mod profile;
pub mod public;
pub mod settings_groups;
pub mod site_settings;
pub mod system;
pub mod taxonomy;

use axum::{
    Router,
    extract::{Request, State},
    http::{Method, StatusCode, header::ORIGIN},
    middleware::{self, Next},
    response::Response,
};

use crate::state::AppState;
use error::ApiError;

/// 分页查询参数的共享默认值：页码从 1 开始，每页默认 20 条。
pub const DEFAULT_PAGE: u32 = 1;
pub const DEFAULT_PAGE_SIZE: u32 = 20;

/// serde `default` 属性要求函数路径，这里提供返回共享常量的 thin fn。
pub const fn default_page() -> u32 {
    DEFAULT_PAGE
}

/// 每页条数默认值（配合 `default_page` 使用）。
pub const fn default_page_size() -> u32 {
    DEFAULT_PAGE_SIZE
}

/// 装配全部 Admin 子路由并挂载 Origin 校验中间件（在 `/api/admin` 前缀下使用）。
pub fn admin_router(state: AppState) -> Router<AppState> {
    Router::new()
        .merge(auth::router())
        .merge(articles::router())
        .merge(profile::router())
        .merge(taxonomy::router())
        .merge(media::router())
        .merge(site_settings::router())
        .merge(comments::router())
        .merge(dashboard::router())
        .merge(audit::router())
        .merge(pages::router())
        .merge(journals::router())
        .merge(galleries::router())
        .merge(links::router())
        .merge(navigation::router())
        .merge(settings_groups::router())
        .merge(ai::router())
        .merge(email::router())
        .merge(logs::router())
        .merge(system::router())
        .route_layer(middleware::from_fn_with_state(state, enforce_origin))
}

/// Admin 写请求的 CSRF 防线：非 GET/HEAD/OPTIONS 必须携带与 `ADMIN_ORIGINS`
/// 精确匹配的 `Origin` Header；读请求放行。
///
/// 额外把路由层的 405（Method Not Allowed）改写为 400：对带 Origin 的跨站请求，
/// 浏览器预检与实际方法不匹配时 405 会泄露端点存在性，统一为业务错误更稳妥。
async fn enforce_origin(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    ) {
        return Ok(next.run(request).await);
    }

    let origin = request
        .headers()
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok());
    if !origin.is_some_and(|origin| {
        state
            .config
            .admin_origins
            .iter()
            .any(|allowed| allowed == origin)
    }) {
        return Err(ApiError::forbidden(
            "INVALID_ORIGIN",
            "Request origin is not allowed",
        ));
    }

    // 405 改写为 400：带 Origin 的跨站请求下，405 会泄露端点存在性，统一为业务错误更稳妥。
    let response = next.run(request).await;
    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        return Err(ApiError::bad_request(
            "METHOD_NOT_ALLOWED",
            "Method is not allowed",
        ));
    }
    Ok(response)
}
