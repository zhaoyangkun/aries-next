//! Contract Test：站点设置读写、字段校验与 Owner 限定权限矩阵。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

async fn create_user(
    app: &TestApp,
    username: &str,
    role: &str,
    password: &str,
) -> anyhow::Result<i64> {
    let password_hash = app
        .state
        .passwords
        .hash(password)
        .context("failed to hash test password")?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO users (username, email, password_hash, display_name, role, status) \
         VALUES ($1, $2, $3, $4, $5, 'active') RETURNING id",
    )
    .bind(username)
    .bind(format!("{username}@example.com"))
    .bind(&password_hash)
    .bind(username)
    .bind(role)
    .fetch_one(&app.state.database)
    .await
    .context("insert test user")?;
    Ok(id)
}

async fn login_cookie(app: &TestApp, login: &str, password: &str) -> anyhow::Result<String> {
    let response = app
        .admin_post(
            "/api/admin/auth/login",
            serde_json::json!({ "login": login, "password": password }),
            None,
        )
        .await?;
    ensure!(response.status == StatusCode::OK, "{}", response.body);
    response
        .set_cookie
        .context("login did not set cookie")?
        .split(';')
        .next()
        .map(str::to_owned)
        .context("failed to parse session cookie")
}

/// 全字段合法的更新负载，校验用例在其上覆写单个字段。
fn valid_payload() -> serde_json::Value {
    serde_json::json!({
        "site_name": "Aries 测试站",
        "site_description": "重写中的新一代博客",
        "site_url": "https://blog.example.com",
        "logo_url": "/api/media/files/logo.png",
        "icp_text": "京ICP备00000000号",
        "default_cover_url": "",
        "page_size_index": 20,
        "page_size_archive": 15,
        "page_size_search": 5,
        "comment_policy": "auto_approve",
        "comments_per_page": 50
    })
}

#[tokio::test]
async fn site_settings_read_update_roundtrip() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 初始读取：Migration 预插的默认单行配置。
        let initial = app.admin_get("/api/admin/site-settings", &cookie).await?;
        ensure!(initial.status == StatusCode::OK, "{}", initial.body);
        ensure!(initial.body["site_name"] == "Aries Next");
        ensure!(initial.body["site_url"] == "");
        ensure!(initial.body["page_size_index"].as_i64() == Some(10));
        ensure!(initial.body["comment_policy"] == "moderated");
        ensure!(initial.body["comments_per_page"].as_i64() == Some(20));
        ensure!(initial.body["updated_at"].is_string());

        // 全量更新 → 200，响应回显新值。
        let updated = app
            .admin_put("/api/admin/site-settings", valid_payload(), &cookie)
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["site_name"] == "Aries 测试站");
        ensure!(updated.body["site_url"] == "https://blog.example.com");
        ensure!(updated.body["logo_url"] == "/api/media/files/logo.png");
        ensure!(updated.body["page_size_index"].as_i64() == Some(20));
        ensure!(updated.body["page_size_archive"].as_i64() == Some(15));
        ensure!(updated.body["page_size_search"].as_i64() == Some(5));
        ensure!(updated.body["comment_policy"] == "auto_approve");
        ensure!(updated.body["comments_per_page"].as_i64() == Some(50));

        // GET 回读确认更新已落库。
        let reread = app.admin_get("/api/admin/site-settings", &cookie).await?;
        ensure!(reread.status == StatusCode::OK, "{}", reread.body);
        ensure!(reread.body["site_name"] == "Aries 测试站");
        ensure!(reread.body["icp_text"] == "京ICP备00000000号");

        // 审计：更新写入一条 site_settings.update。
        let (audit_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = 'site_settings.update'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(
            audit_count == 1,
            "expected 1 site settings audit, got {audit_count}"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn site_settings_validation_rejects_invalid_payloads() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 每个用例在合法负载上覆写一个非法字段，断言 400 与错误码。
        let cases: [(&str, serde_json::Value, &str); 7] = [
            // Site Name 空白。
            ("site_name", serde_json::json!("   "), "INVALID_SITE_NAME"),
            // Site Name 超过 100 字符。
            (
                "site_name",
                serde_json::json!("长".repeat(101)),
                "INVALID_SITE_NAME",
            ),
            // URL 字段不允许 javascript: Scheme。
            (
                "site_url",
                serde_json::json!("javascript:alert(1)"),
                "INVALID_URL",
            ),
            // 分页大小越界（下限）。
            ("page_size_index", serde_json::json!(0), "INVALID_PAGE_SIZE"),
            // 分页大小越界（上限）。
            (
                "page_size_archive",
                serde_json::json!(101),
                "INVALID_PAGE_SIZE",
            ),
            // 每页评论数越界。
            (
                "comments_per_page",
                serde_json::json!(4),
                "INVALID_COMMENTS_PER_PAGE",
            ),
            // 非法评论策略。
            (
                "comment_policy",
                serde_json::json!("open"),
                "INVALID_COMMENT_POLICY",
            ),
        ];
        for (field, value, expected_code) in cases {
            let mut payload = valid_payload();
            payload[field] = value;
            let response = app
                .admin_put("/api/admin/site-settings", payload, &cookie)
                .await?;
            ensure!(
                response.status == StatusCode::BAD_REQUEST,
                "field {field}: {}",
                response.body
            );
            ensure!(
                response.body["error"]["code"] == expected_code,
                "field {field}: {}",
                response.body
            );
        }

        // 全部校验失败后设置保持初始默认值，未被部分写入。
        let settings = app.admin_get("/api/admin/site-settings", &cookie).await?;
        ensure!(settings.body["site_name"] == "Aries Next");
        ensure!(settings.body["page_size_index"].as_i64() == Some(10));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn site_settings_permission_matrix_owner_only() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-ss", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-ss", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-ss", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-ss", "moderator-pass-1").await?;

        // ManageSettings 仅 Owner：Editor / Moderator 读写均 403。
        let editor_read = app
            .admin_get("/api/admin/site-settings", &editor_cookie)
            .await?;
        ensure!(
            editor_read.status == StatusCode::FORBIDDEN,
            "{}",
            editor_read.body
        );
        let editor_write = app
            .admin_put("/api/admin/site-settings", valid_payload(), &editor_cookie)
            .await?;
        ensure!(
            editor_write.status == StatusCode::FORBIDDEN,
            "{}",
            editor_write.body
        );
        let moderator_read = app
            .admin_get("/api/admin/site-settings", &moderator_cookie)
            .await?;
        ensure!(
            moderator_read.status == StatusCode::FORBIDDEN,
            "{}",
            moderator_read.body
        );

        // 未认证请求 → 401。
        let anonymous_read = app.get("/api/admin/site-settings").await?;
        ensure!(anonymous_read.status == StatusCode::UNAUTHORIZED);
        let anonymous_write = app
            .admin_put(
                "/api/admin/site-settings",
                valid_payload(),
                "aries_admin_session=missing-token",
            )
            .await?;
        ensure!(anonymous_write.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
