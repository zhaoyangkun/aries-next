//! OpenAPI 文档：utoipa 注解生成的 API 契约（全量端点）。
//!
//! 全部端点已从手写 `docs/openapi.yaml` 迁移到代码注解，机器生成物即
//! `docs/openapi.yaml` 本身，由 CI 漂移检查（导出结果与提交文件逐字节 diff）
//! 和 `backend/server/tests/openapi_docs.rs` 保证代码与契约永不脱节。
//! 修改 API 时只需更新注解后重新导出（`cargo run -p aries-server --bin
//! openapi-export > docs/openapi.yaml`）。

use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Aries Next API",
        version = "0.1.0",
        description = "Aries Next 博客系统 API（由 utoipa 注解自动生成，权威契约即本文件）",
    ),
    servers((url = "/")),
    paths(
        // Admin Auth
        crate::http::auth::bootstrap_status,
        crate::http::auth::bootstrap,
        crate::http::auth::login,
        crate::http::auth::logout,
        crate::http::auth::session,
        crate::http::auth::forgot_password,
        crate::http::auth::reset_password,
        // Admin Articles
        crate::http::articles::list_articles,
        crate::http::articles::create_article,
        crate::http::articles::get_article,
        crate::http::articles::update_article,
        crate::http::articles::change_status,
        crate::http::articles::preview_article,
        crate::http::articles::delete_article,
        crate::http::articles::list_revisions,
        crate::http::articles::restore_revision,
        crate::http::articles::reorder_articles,
        // Admin Taxonomy
        crate::http::taxonomy::list_categories,
        crate::http::taxonomy::create_category,
        crate::http::taxonomy::update_category,
        crate::http::taxonomy::delete_category,
        crate::http::taxonomy::list_tags,
        crate::http::taxonomy::create_tag,
        crate::http::taxonomy::update_tag,
        crate::http::taxonomy::delete_tag,
        // Admin Media
        crate::http::media::list_media,
        crate::http::media::upload_media,
        crate::http::media::upload_remote,
        crate::http::media::batch_delete_media,
        crate::http::media::get_media,
        crate::http::media::update_media,
        crate::http::media::delete_media,
        crate::http::media::list_media_usages,
        crate::http::media::import_markdown,
        crate::http::media::get_import,
        crate::http::media::commit_import,
        // Admin Site Settings
        crate::http::site_settings::get_settings,
        crate::http::site_settings::update_settings,
        // Admin Comments
        crate::http::comments::list_comments,
        crate::http::comments::get_comment,
        crate::http::comments::change_comment_status,
        crate::http::comments::reply_to_comment,
        crate::http::comments::delete_comment,
        // Admin Dashboard / Audit
        crate::http::dashboard::get_dashboard,
        crate::http::audit::list_audit_logs,
        // Admin Pages
        crate::http::pages::list_pages,
        crate::http::pages::create_page,
        crate::http::pages::get_page,
        crate::http::pages::update_page,
        crate::http::pages::delete_page,
        // Admin Journals
        crate::http::journals::list_journals,
        crate::http::journals::create_journal,
        crate::http::journals::get_journal,
        crate::http::journals::update_journal,
        crate::http::journals::delete_journal,
        // Admin Galleries
        crate::http::galleries::list_galleries,
        crate::http::galleries::create_gallery,
        crate::http::galleries::get_gallery,
        crate::http::galleries::update_gallery,
        crate::http::galleries::delete_gallery,
        crate::http::galleries::list_gallery_items,
        crate::http::galleries::add_gallery_item,
        crate::http::galleries::reorder_gallery_items,
        crate::http::galleries::update_gallery_item,
        crate::http::galleries::remove_gallery_item,
        crate::http::galleries::list_gallery_categories,
        crate::http::galleries::create_gallery_category,
        crate::http::galleries::update_gallery_category,
        crate::http::galleries::delete_gallery_category,
        // Admin Links
        crate::http::links::list_links,
        crate::http::links::create_link,
        crate::http::links::get_link,
        crate::http::links::update_link,
        crate::http::links::delete_link,
        crate::http::links::list_link_categories,
        crate::http::links::create_link_category,
        crate::http::links::update_link_category,
        crate::http::links::delete_link_category,
        // Admin Navigation
        crate::http::navigation::list_navigation,
        crate::http::navigation::create_navigation_item,
        crate::http::navigation::update_navigation_item,
        crate::http::navigation::delete_navigation_item,
        crate::http::navigation::reorder_navigation,
        // Admin Settings Groups
        crate::http::settings_groups::get_setting_group,
        crate::http::settings_groups::update_setting_group,
        // Admin AI
        crate::http::ai::editor_rewrite,
        crate::http::ai::editor_summary,
        crate::http::ai::editor_metadata,
        crate::http::ai::editor_tags,
        crate::http::ai::editor_brief,
        crate::http::ai::list_ai_models,
        crate::http::ai::list_ai_usage,
        // Admin Logs
        crate::http::logs::list_logs,
        crate::http::logs::list_log_stats,
        crate::http::logs::list_log_targets,
        crate::http::logs::get_sql_logging,
        crate::http::logs::set_sql_logging,
        crate::http::logs::get_filter_override,
        crate::http::logs::set_filter_override,
        crate::http::logs::tail_logs,
        // Admin Profile
        crate::http::profile::get_profile,
        crate::http::profile::update_profile,
        crate::http::profile::update_password,
        // Public Site
        crate::http::public::site::get_public_site,
        // Public Articles（含搜索）
        crate::http::public::articles::list_public_articles,
        crate::http::public::articles::get_public_article,
        crate::http::public::articles::unlock_article,
        crate::http::public::articles::record_article_view,
        crate::http::public::articles::search_public_articles,
        crate::http::public::articles::search_suggest,
        crate::http::public::articles::search_ask,
        crate::http::public::articles::related_articles,
        crate::http::public::articles::list_public_archives,
        // Public Taxonomy
        crate::http::public::taxonomy::list_public_categories,
        crate::http::public::taxonomy::list_category_articles,
        crate::http::public::taxonomy::list_public_tags,
        crate::http::public::taxonomy::list_tag_articles,
        // Public Comments
        crate::http::public::comments::list_article_comments,
        crate::http::public::comments::list_page_comments,
        crate::http::public::comments::create_comment,
        crate::http::public::comments::reply_comment,
        // Public Extended（Pages / Journals / Galleries / Links / Navigation）
        crate::http::public::extended::get_page,
        crate::http::public::extended::list_journals,
        crate::http::public::extended::list_galleries,
        crate::http::public::extended::get_gallery,
        crate::http::public::extended::list_photos,
        crate::http::public::extended::list_links,
        crate::http::public::extended::list_navigation,
        // Public Media（匿名文件服务）
        crate::http::media::serve_media_file,
        // Health
        crate::live,
        crate::ready,
    ),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

/// 统一注册安全方案：Admin Session 为 HttpOnly Cookie（Path 限定 /api/admin）。
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme(
                "cookieAuth",
                SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("aries_admin_session"))),
            );
    }
}

/// 统一错误响应体（与 `http::error::ApiError` 的实际 JSON 形状一致）：
/// `{ "error": { "code", "message", "details?" } }`。
/// 所有端点的 4xx/5xx 响应统一引用本类型。
#[derive(utoipa::ToSchema)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(utoipa::ToSchema)]
pub struct ErrorBody {
    /// 机器可读错误码，如 `UNAUTHORIZED`、`ARTICLE_NOT_FOUND`。
    pub code: String,
    /// 人类可读错误信息。
    pub message: String,
    /// 结构化补充信息（如引用计数），大多数错误为空，序列化时省略。
    pub details: Option<serde_json::Value>,
}

/// 健康检查响应（`aries_core::health::HealthResponse` 的文档镜像，避免给 core 加 utoipa 依赖）。
#[derive(utoipa::ToSchema)]
#[schema(title = "HealthResponse")]
pub struct HealthView {
    /// 固定为 `ok`。
    pub status: String,
    /// 固定为 `aries-server`。
    pub service: String,
}

/// 序列化为 YAML（导出脚本与漂移检查共用）。
pub fn to_yaml() -> anyhow::Result<String> {
    Ok(ApiDoc::openapi().to_yaml()?)
}
