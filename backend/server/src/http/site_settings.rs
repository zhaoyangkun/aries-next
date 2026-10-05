//! 站点设置 Admin 端点：单行配置读写，仅 Owner 可管理。

use aries_core::{
    auth::{AuditEvent, Permission},
    media::{CommentPolicy, SiteSettings, SiteSettingsUpdate},
};
use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

use super::{auth::CurrentUser, error::ApiError, extract::ApiJson};

pub fn router() -> Router<AppState> {
    Router::new().route("/site-settings", get(get_settings).put(update_settings))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "SiteSettingsResponse", description = "单行站点设置。")]
struct SiteSettingsResponse {
    /// 站点名称（1–100 字符）。
    site_name: String,
    site_description: String,
    /// 站点外部 URL；非空时必须是合法 http/https URL。
    site_url: String,
    logo_url: String,
    /// ICP 备案号文本，展示在 Footer。
    icp_text: String,
    /// 文章默认封面 URL。
    default_cover_url: String,
    /// 首页分页大小（1–100）。
    page_size_index: i32,
    /// 归档页分页大小（1–100）。
    page_size_archive: i32,
    /// 搜索页分页大小（1–100）。
    page_size_search: i32,
    /// 评论策略：`closed` / `moderated` / `auto_approve`。
    comment_policy: String,
    /// 评论每页条数（5–100）。
    comments_per_page: i32,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: time::OffsetDateTime,
}

impl From<SiteSettings> for SiteSettingsResponse {
    fn from(settings: SiteSettings) -> Self {
        Self {
            site_name: settings.site_name,
            site_description: settings.site_description,
            site_url: settings.site_url,
            logo_url: settings.logo_url,
            icp_text: settings.icp_text,
            default_cover_url: settings.default_cover_url,
            page_size_index: settings.page_size_index,
            page_size_archive: settings.page_size_archive,
            page_size_search: settings.page_size_search,
            comment_policy: settings.comment_policy.as_str().to_owned(),
            comments_per_page: settings.comments_per_page,
            updated_at: settings.updated_at,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(
    title = "UpdateSiteSettingsRequest",
    description = "全量更新站点设置。`site_url`/`logo_url`/`default_cover_url` 允许为空或以 `/` 开头的站内绝对路径，非空外链必须是 http/https URL（400 `INVALID_URL`）；三个 Page Size 取值 1–100（400 `INVALID_PAGE_SIZE`）；`comments_per_page` 取值 5–100；`comment_policy` 为 `closed`/`moderated`/`auto_approve`（400 `INVALID_COMMENT_POLICY`）。"
)]
struct UpdateSiteSettingsRequest {
    /// 站点名称，Trim 后 1–100 字符（400 `INVALID_SITE_NAME`）。
    site_name: String,
    #[serde(default)]
    site_description: String,
    #[serde(default)]
    site_url: String,
    #[serde(default)]
    logo_url: String,
    #[serde(default)]
    icp_text: String,
    #[serde(default)]
    default_cover_url: String,
    page_size_index: i32,
    page_size_archive: i32,
    page_size_search: i32,
    /// 评论策略，缺省 `moderated`。
    #[serde(default = "default_comment_policy")]
    comment_policy: String,
    /// 评论每页条数，缺省 20。
    #[serde(default = "default_comments_per_page")]
    comments_per_page: i32,
}

fn default_comment_policy() -> String {
    "moderated".to_owned()
}

fn default_comments_per_page() -> i32 {
    20
}

#[utoipa::path(
    get,
    path = "/api/admin/site-settings",
    tag = "Admin Site Settings",
    operation_id = "getSiteSettings",
    summary = "读取站点设置",
    description = "读取单行站点设置，仅 Owner（`settings:manage`）可访问。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "单行站点设置", body = SiteSettingsResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "非 Owner（settings:manage）", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_settings(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<SiteSettingsResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let settings = state.site_settings.get().await?;
    Ok(Json(settings.into()))
}

#[utoipa::path(
    put,
    path = "/api/admin/site-settings",
    tag = "Admin Site Settings",
    operation_id = "updateSiteSettings",
    summary = "全量更新站点设置",
    description = "全量更新站点设置；URL 字段允许为空、站内绝对路径或 http/https 外链，Page Size 取值 1–100，`comments_per_page` 取值 5–100。写 Audit（`site_settings.update`）。",
    security(("cookieAuth" = [])),
    request_body(content = UpdateSiteSettingsRequest, content_type = "application/json", description = "站点设置入参"),
    responses(
        (status = 200, description = "站点设置已更新", body = SiteSettingsResponse),
        (status = 400, description = "字段校验失败（INVALID_SITE_NAME / INVALID_URL / INVALID_PAGE_SIZE 等）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "非 Owner（settings:manage）", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_settings(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<UpdateSiteSettingsRequest>,
) -> Result<Json<SiteSettingsResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let site_name = request.site_name.trim().to_owned();
    if site_name.is_empty() || site_name.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "INVALID_SITE_NAME",
            "Site name must contain 1 to 100 characters",
        ));
    }
    validate_optional_url("site_url", &request.site_url)?;
    validate_optional_url("logo_url", &request.logo_url)?;
    validate_optional_url("default_cover_url", &request.default_cover_url)?;
    for page_size in [
        request.page_size_index,
        request.page_size_archive,
        request.page_size_search,
    ] {
        if !(1..=100).contains(&page_size) {
            return Err(ApiError::bad_request(
                "INVALID_PAGE_SIZE",
                "Page size must be between 1 and 100",
            ));
        }
    }
    if !(5..=100).contains(&request.comments_per_page) {
        return Err(ApiError::bad_request(
            "INVALID_COMMENTS_PER_PAGE",
            "Comments per page must be between 5 and 100",
        ));
    }
    let comment_policy: CommentPolicy = request
        .comment_policy
        .parse()
        .map_err(|_| ApiError::bad_request("INVALID_COMMENT_POLICY", "Invalid comment policy"))?;

    let settings = state
        .site_settings
        .update(SiteSettingsUpdate {
            site_name,
            site_description: request.site_description.trim().to_owned(),
            site_url: request.site_url.trim().to_owned(),
            logo_url: request.logo_url.trim().to_owned(),
            icp_text: request.icp_text.trim().to_owned(),
            default_cover_url: request.default_cover_url.trim().to_owned(),
            page_size_index: request.page_size_index,
            page_size_archive: request.page_size_archive,
            page_size_search: request.page_size_search,
            comment_policy,
            comments_per_page: request.comments_per_page,
        })
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "site_settings.update".to_owned(),
            target_type: "site_settings".to_owned(),
            target_id: None,
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(Json(settings.into()))
}

/// URL 字段允许为空；非空时必须是 http/https 绝对 URL 或站内绝对路径。
fn validate_optional_url(field: &'static str, value: &str) -> Result<(), ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    if value.starts_with('/') && !value.starts_with("//") && !value.contains("..") {
        return Ok(());
    }
    let valid = reqwest::Url::parse(value)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"));
    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request_with_details(
            "INVALID_URL",
            "URL field is invalid",
            serde_json::json!({ "field": field }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_validation_accepts_empty_paths_and_http_urls() {
        assert!(validate_optional_url("site_url", "").is_ok());
        assert!(validate_optional_url("logo_url", "/api/media/files/2026/08/a.png").is_ok());
        assert!(validate_optional_url("site_url", "https://blog.example.com").is_ok());
        assert!(validate_optional_url("site_url", "javascript:alert(1)").is_err());
        assert!(validate_optional_url("site_url", "//evil.example.com").is_err());
        assert!(validate_optional_url("logo_url", "/../secret").is_err());
    }
}
