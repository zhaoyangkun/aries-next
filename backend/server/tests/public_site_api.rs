//! Public 站点信息端点（/api/public/site）集成测试：
//! 匿名可访问、默认值与更新后的精确字段集合、缓存策略，
//! 以及 setting_groups 中的内部配置与 Secret（smtp_password / api_key）绝不外泄。
//! 通过 `ARIES_RUN_DATABASE_TESTS=1` 门控，随机 Schema 隔离，走真实 Router + PostgreSQL。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

/// 期望的公开字段集合：与 `PublicSiteResponse` 一一对应，新增内部配置默认不得出现。
const PUBLIC_SITE_KEYS: [&str; 7] = [
    "created_at",
    "default_cover_url",
    "icp_text",
    "logo_url",
    "site_description",
    "site_name",
    "site_url",
];

async fn get_public_site(app: &TestApp) -> anyhow::Result<serde_json::Value> {
    let (status, headers, bytes) = app.get_raw("/api/public/site").await?;
    ensure!(status == StatusCode::OK);
    // 聚合缓存策略：CACHE_AGGREGATE。
    let cache = headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .context("missing cache-control")?;
    ensure!(
        cache.contains("stale-while-revalidate"),
        "unexpected: {cache}"
    );
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    let object = body.as_object().context("site must be an object")?;
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    ensure!(
        keys == PUBLIC_SITE_KEYS,
        "unexpected public site fields: {keys:?}"
    );
    Ok(body)
}

#[tokio::test]
async fn public_site_returns_defaults_without_auth() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        // 全新站点（无任何配置写入）：匿名 GET 返回 Migration 默认值。
        let body = get_public_site(&app).await?;
        ensure!(body["site_name"] == "Aries Next", "{body}");
        for key in [
            "site_description",
            "site_url",
            "logo_url",
            "icp_text",
            "default_cover_url",
        ] {
            ensure!(body[key] == "", "{key} should default to empty: {body}");
        }
        // 建站时间由 Migration 预插行生成，必须是合法的时间戳字符串。
        let created_at = body["created_at"]
            .as_str()
            .context("created_at should be a string")?;
        ensure!(
            time::OffsetDateTime::parse(created_at, &time::format_description::well_known::Rfc3339)
                .is_ok(),
            "created_at should be RFC 3339: {created_at}"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn public_site_reflects_admin_updates_but_hides_internal_fields() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 更新全部公开字段，同时把分页/评论策略等内部配置改成非默认值。
        let updated = app
            .admin_put(
                "/api/admin/site-settings",
                serde_json::json!({
                    "site_name": "Aries 博客",
                    "site_description": "记录 Rust 与生活",
                    "site_url": "https://blog.example.com",
                    "logo_url": "https://cdn.example.com/logo.png",
                    "icp_text": "京ICP备00000000号",
                    "default_cover_url": "https://cdn.example.com/cover.jpg",
                    "page_size_index": 25,
                    "page_size_archive": 30,
                    "page_size_search": 40,
                }),
                &cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);

        // 公开端点精确回显七个公开字段。
        let body = get_public_site(&app).await?;
        ensure!(body["site_name"] == "Aries 博客");
        ensure!(body["site_description"] == "记录 Rust 与生活");
        ensure!(body["site_url"] == "https://blog.example.com");
        ensure!(body["logo_url"] == "https://cdn.example.com/logo.png");
        ensure!(body["icp_text"] == "京ICP备00000000号");
        ensure!(body["default_cover_url"] == "https://cdn.example.com/cover.jpg");

        // 分页与评论策略属内部配置，绝不外泄（字段集合已由 get_public_site 精确校验，
        // 这里再显式点名防回归）。
        for key in [
            "page_size_index",
            "page_size_archive",
            "page_size_search",
            "comment_policy",
            "comments_per_page",
            "updated_at",
        ] {
            ensure!(body.get(key).is_none(), "internal field leaked: {key}");
        }
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn public_site_never_leaks_setting_group_secrets() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 在 email / ai 分组写入 Secret 与内部配置。
        let email = app
            .admin_put(
                "/api/admin/settings/email",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": {
                        "enabled": true,
                        "smtp_host": "smtp.example.com",
                        "smtp_port": 465,
                        "smtp_username": "mailer",
                        "smtp_password": "super-secret-smtp-9f2c",
                        "from_address": "hi@example.com"
                    }
                }),
                &cookie,
            )
            .await?;
        ensure!(email.status == StatusCode::OK, "{}", email.body);
        let ai = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": {
                        "enabled": true,
                        "protocol": "open_ai",
                        "base_url": "https://api.vendor.example.com",
                        "model": "test-model",
                        "api_key": "sk-test-secret-key-71ab"
                    }
                }),
                &cookie,
            )
            .await?;
        ensure!(ai.status == StatusCode::OK, "{}", ai.body);

        // 公开站点信息既不泄露 Secret 明文，也不含任何内部分组字段。
        let body = get_public_site(&app).await?;
        let raw = serde_json::to_string(&body)?;
        for secret in ["super-secret-smtp-9f2c", "sk-test-secret-key-71ab"] {
            ensure!(
                !raw.contains(secret),
                "secret leaked to public site: {secret}"
            );
        }
        for key in [
            "smtp_password",
            "smtp_host",
            "api_key",
            "base_url",
            "model",
            "analytics_id",
        ] {
            ensure!(body.get(key).is_none(), "setting group field leaked: {key}");
        }
        // 所有字段均为字符串类型，不夹带结构化内部配置。
        for value in body.as_object().context("site object")?.values() {
            ensure!(
                value.is_string(),
                "non-string field in public site: {value}"
            );
        }
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
