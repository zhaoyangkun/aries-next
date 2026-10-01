//! Public 匿名只读端点：文章、分类、标签、归档、搜索与站点信息。
//! 不挂 `enforce_origin`，不要求 Session；缓存策略见各处理器上的 Cache-Control 约定。

pub mod articles;
pub mod comments;
pub mod extended;
pub mod site;
pub mod taxonomy;

use axum::{
    Json, Router,
    http::header,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::http::error::ApiError;
use crate::state::AppState;

/// 越界分页守卫：page > 1 且当前页结果为空时返回 404，阻止爬虫沿 `?page=N` 生成无限重复 URL。
/// page=1 的空结果（如无搜索结果、目标下暂无评论）保持 200。
pub fn reject_out_of_range_page(page: u32, item_count: usize) -> Result<(), ApiError> {
    if page > 1 && item_count == 0 {
        return Err(ApiError::not_found(
            "PAGE_OUT_OF_RANGE",
            "Page is out of range",
        ));
    }
    Ok(())
}

/// 聚合类端点（site/categories/tags/archives）：短缓存 + 后台回源。
pub const CACHE_AGGREGATE: &str = "public, max-age=60, stale-while-revalidate=300";
/// 普通文章详情：可公共缓存。
pub const CACHE_ARTICLE: &str = "public, max-age=60";
/// 密码文章与写端点（access/views）：禁止任何缓存。
pub const CACHE_NO_STORE: &str = "no-store";

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(articles::router())
        .merge(site::router())
        .merge(taxonomy::router())
        .merge(extended::router())
        .merge(comments::router())
}

/// 带 Cache-Control 的 JSON 响应；公开端点必须显式声明缓存策略。
pub fn json_with_cache<T: Serialize>(body: &T, cache_control: &'static str) -> Response {
    ([(header::CACHE_CONTROL, cache_control)], Json(body)).into_response()
}

/// 客户端 IP：优先反向代理透传的 X-Forwarded-For / X-Real-IP，其次对端地址，缺失时退化为 "unknown"。
/// 仅用于内存滑动窗口去重与限流，不持久化、不进入日志之外的存储。
pub fn client_ip(headers: &axum::http::HeaderMap, peer: Option<std::net::SocketAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        })
        .or_else(|| peer.map(|addr| addr.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// 客户端标识：IP + User-Agent 的组合指纹，缺失时退化为 "unknown|"。
pub fn client_fingerprint(headers: &axum::http::HeaderMap) -> String {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    format!("{}|{user_agent}", client_ip(headers, None))
}
