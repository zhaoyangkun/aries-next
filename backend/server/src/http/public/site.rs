//! 公开站点信息：site_settings 的公开投影，绝不暴露任何内部字段或 Secret。

use axum::{Router, extract::State, response::Response, routing::get};
use serde::Serialize;

use crate::state::AppState;

use super::{CACHE_AGGREGATE, json_with_cache};
use crate::http::error::ApiError;

pub fn router() -> Router<AppState> {
    Router::new().route("/site", get(get_public_site))
}

/// 只投影公开安全的字段；`site_settings` 表未来新增的内部配置默认不出现在这里。
#[derive(Debug, Serialize)]
struct PublicSiteResponse {
    site_name: String,
    site_description: String,
    site_url: String,
    logo_url: String,
    icp_text: String,
    default_cover_url: String,
    /// 建站时间（展示端 footer「本站已运行 X 天」）。
    #[serde(with = "time::serde::rfc3339")]
    created_at: time::OffsetDateTime,
}

async fn get_public_site(State(state): State<AppState>) -> Result<Response, ApiError> {
    let settings = state.site_settings.get().await?;
    Ok(json_with_cache(
        &PublicSiteResponse {
            site_name: settings.site_name,
            site_description: settings.site_description,
            site_url: settings.site_url,
            logo_url: settings.logo_url,
            icp_text: settings.icp_text,
            default_cover_url: settings.default_cover_url,
            created_at: settings.created_at,
        },
        CACHE_AGGREGATE,
    ))
}
