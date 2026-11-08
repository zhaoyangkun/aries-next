//! 公开站点信息：site_settings 的公开投影，绝不暴露任何内部字段或 Secret。

use axum::{Router, extract::State, response::Response, routing::get};
use serde::Serialize;

use aries_core::settings::{SettingGroup, SocialSettings};

use crate::state::AppState;

use super::{CACHE_AGGREGATE, json_with_cache};
use crate::http::error::ApiError;

/// 注册公开站点信息路由（挂载在 `/api/public` 下）。
pub fn router() -> Router<AppState> {
    Router::new().route("/site", get(get_public_site))
}

/// 社交链接公开投影：Owner 在设置中配置，公开展示在 footer。
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "PublicSocialLink",
    description = "社交链接条目；`icon` 为前端图标键，未知值前端退化为通用链接图标。"
)]
struct PublicSocialLink {
    name: String,
    url: String,
    icon: Option<String>,
}

/// 只投影公开安全的字段；`site_settings` 表未来新增的内部配置默认不出现在这里。
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "PublicSiteResponse",
    description = "站点公开信息；只含站点名称、描述、URL、Logo、ICP 备案号、公安备案号、社交链接与默认封面，绝不返回任何内部配置。"
)]
struct PublicSiteResponse {
    site_name: String,
    site_description: String,
    site_url: String,
    logo_url: String,
    icp_text: String,
    police_text: String,
    default_cover_url: String,
    /// 社交链接（footer 展示）；未配置或读取失败时为空数组，不影响站点信息可用性。
    social_links: Vec<PublicSocialLink>,
    /// 建站时间（展示端 footer「本站已运行 X 天」）。
    #[serde(with = "time::serde::rfc3339")]
    created_at: time::OffsetDateTime,
}

/// 站点公开信息端点：site_settings 的白名单投影 + 社交链接（降级为空列表）。
/// 匿名可访问，带聚合缓存头；新增内部配置默认不出现在此响应。
#[utoipa::path(
    get,
    path = "/api/public/site",
    tag = "Public Site",
    operation_id = "getPublicSite",
    summary = "公开站点信息",
    description = "site_settings 的公开投影；只含站点名称、描述、URL、Logo、ICP 备案号、公安备案号、社交链接与默认封面，绝不返回任何内部配置。可匿名访问并带 CDN 缓存头。",
    responses(
        (status = 200, description = "公开站点信息", body = PublicSiteResponse),
    )
)]
async fn get_public_site(State(state): State<AppState>) -> Result<Response, ApiError> {
    let settings = state.site_settings.get().await?;
    // 社交链接是锦上添花：分组行缺失或读取失败时降级为空列表，
    // 不让站点信息整体 500（Migration 预插行，正常路径不会走到降级）。
    let social_links = match state.settings.get_group(SettingGroup::Social).await {
        Ok(record) => {
            let parsed: SocialSettings =
                serde_json::from_value(record.payload).map_err(|_| ApiError::internal())?;
            parsed
                .links
                .into_iter()
                .map(|link| PublicSocialLink {
                    name: link.name.trim().to_owned(),
                    url: link.url.trim().to_owned(),
                    icon: link.icon,
                })
                .collect()
        }
        Err(error) => {
            tracing::warn!(error = %error, "social settings unavailable; projecting empty links");
            Vec::new()
        }
    };
    Ok(json_with_cache(
        &PublicSiteResponse {
            site_name: settings.site_name,
            site_description: settings.site_description,
            site_url: settings.site_url,
            logo_url: settings.logo_url,
            icp_text: settings.icp_text,
            police_text: settings.police_text,
            default_cover_url: settings.default_cover_url,
            social_links,
            created_at: settings.created_at,
        },
        CACHE_AGGREGATE,
    ))
}
