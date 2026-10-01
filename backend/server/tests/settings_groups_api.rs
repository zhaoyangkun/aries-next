//! Phase 06/08 Contract Test：设置分组（appearance/email/integrations/ai）读写、
//! 乐观锁冲突、secret write-only 语义与权限矩阵。

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

#[tokio::test]
async fn settings_email_secret_and_optimistic_lock() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 初始读取：预插默认行，version = 1，secret 未设置。
        let initial = app
            .admin_get("/api/admin/settings/email", &owner_cookie)
            .await?;
        ensure!(initial.status == StatusCode::OK, "{}", initial.body);
        ensure!(initial.body["group"] == "email");
        ensure!(initial.body["version"].as_i64() == Some(1));
        ensure!(initial.body["settings"]["enabled"] == false);
        ensure!(initial.body["settings"]["smtp_password_set"] == false);

        // 写入 SMTP 配置 + 密码。
        let updated = app
            .admin_put(
                "/api/admin/settings/email",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": {
                        "enabled": true,
                        "smtp_host": "smtp.example.com",
                        "smtp_port": 465,
                        "smtp_username": "mailer",
                        "smtp_password": "super-secret",
                        "from_address": "hi@example.com"
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["version"].as_i64() == Some(2));
        ensure!(updated.body["settings"]["smtp_password_set"] == true);
        // 响应绝不回传明文密码。
        ensure!(updated.body["settings"].get("smtp_password").is_none());

        // GET 回读：同样不得回显明文 secret，只回 `smtp_password_set`。
        let read_back = app
            .admin_get("/api/admin/settings/email", &owner_cookie)
            .await?;
        ensure!(read_back.status == StatusCode::OK, "{}", read_back.body);
        ensure!(read_back.body["version"].as_i64() == Some(2));
        ensure!(read_back.body["settings"]["smtp_password_set"] == true);
        ensure!(read_back.body["settings"].get("smtp_password").is_none());
        ensure!(read_back.body["settings"]["smtp_host"] == "smtp.example.com");

        // 乐观锁：过期版本 → 409。
        let stale = app
            .admin_put(
                "/api/admin/settings/email",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": { "enabled": false }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(stale.status == StatusCode::CONFLICT, "{}", stale.body);
        ensure!(stale.body["error"]["code"] == "SETTING_VERSION_CONFLICT");

        // secret = null：保持不变；其余字段全量覆盖。
        let keep = app
            .admin_put(
                "/api/admin/settings/email",
                serde_json::json!({
                    "expected_version": 2,
                    "settings": {
                        "enabled": true,
                        "smtp_host": "smtp2.example.com",
                        "smtp_port": 587,
                        "smtp_username": "mailer2",
                        "smtp_password": null,
                        "from_address": "hi2@example.com"
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(keep.status == StatusCode::OK, "{}", keep.body);
        ensure!(keep.body["version"].as_i64() == Some(3));
        ensure!(keep.body["settings"]["smtp_host"] == "smtp2.example.com");
        ensure!(
            keep.body["settings"]["smtp_password_set"] == true,
            "null must keep the secret"
        );

        // 库内仍保存原密码。
        let (stored,): (serde_json::Value,) =
            sqlx::query_as("SELECT payload FROM setting_groups WHERE grp = 'email'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(stored["smtp_password"] == "super-secret");

        // secret = 空字符串：清除。
        let cleared = app
            .admin_put(
                "/api/admin/settings/email",
                serde_json::json!({
                    "expected_version": 3,
                    "settings": {
                        "enabled": false,
                        "smtp_password": ""
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(cleared.status == StatusCode::OK, "{}", cleared.body);
        ensure!(cleared.body["settings"]["smtp_password_set"] == false);

        // 非法 group → 404。
        let unknown = app
            .admin_get("/api/admin/settings/site", &owner_cookie)
            .await?;
        ensure!(unknown.status == StatusCode::NOT_FOUND);
        // 非法 payload 结构 → 400。
        let invalid = app
            .admin_put(
                "/api/admin/settings/appearance",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": { "color_scheme": "neon" }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            invalid.status == StatusCode::BAD_REQUEST,
            "{}",
            invalid.body
        );
        ensure!(invalid.body["error"]["code"] == "INVALID_SETTING_PAYLOAD");

        // appearance 正常读写。
        let appearance = app
            .admin_put(
                "/api/admin/settings/appearance",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": {
                        "logo_url": "https://cdn.example.com/logo.png",
                        "color_scheme": "dark",
                        "list_density": "compact"
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(appearance.status == StatusCode::OK, "{}", appearance.body);
        ensure!(appearance.body["settings"]["color_scheme"] == "dark");

        // 审计：settings.email.updated ×3 + settings.appearance.updated。
        let (audit_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE target_type = 'setting_group'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(
            audit_count == 4,
            "expected 4 settings audit events, got {audit_count}"
        );
        let (email_audit,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs WHERE action = 'settings.email.updated'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(email_audit == 3);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn settings_partial_patch_merges_without_resetting_fields() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // email 组：先完整配置（version 1 → 2）。
        let updated = app
            .admin_put(
                "/api/admin/settings/email",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": {
                        "enabled": true,
                        "smtp_host": "smtp.example.com",
                        "smtp_port": 465,
                        "smtp_username": "mailer",
                        "smtp_password": "patch-secret",
                        "from_address": "hi@example.com",
                        "from_name": "Aries"
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);

        // 只 PATCH smtp_host：enabled、其余字段与 secret 均不得重置。
        let patched = app
            .admin_put(
                "/api/admin/settings/email",
                serde_json::json!({
                    "expected_version": 2,
                    "settings": { "smtp_host": "smtp2.example.com" }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(patched.status == StatusCode::OK, "{}", patched.body);
        let settings = &patched.body["settings"];
        ensure!(settings["smtp_host"] == "smtp2.example.com");
        ensure!(
            settings["enabled"] == true,
            "partial patch must not reset enabled"
        );
        ensure!(settings["smtp_port"] == 465);
        ensure!(settings["smtp_username"] == "mailer");
        ensure!(settings["from_address"] == "hi@example.com");
        ensure!(settings["from_name"] == "Aries");
        ensure!(
            settings["smtp_password_set"] == true,
            "partial patch must keep the secret"
        );

        // ai 组：先完整配置（version 1 → 2）。
        let ai_updated = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": {
                        "enabled": true,
                        "protocol": "anthropic",
                        "base_url": "https://api.anthropic.com",
                        "model": "claude-1",
                        "api_key": "sk-patch-secret",
                        "features": { "editor_assist": true, "comment_moderation": true }
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(ai_updated.status == StatusCode::OK, "{}", ai_updated.body);

        // 只 PATCH model：enabled、protocol、features 与 api_key 均不得重置。
        let ai_patched = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 2,
                    "settings": { "model": "claude-2" }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(ai_patched.status == StatusCode::OK, "{}", ai_patched.body);
        let settings = &ai_patched.body["settings"];
        ensure!(settings["model"] == "claude-2");
        ensure!(
            settings["enabled"] == true,
            "partial patch must not reset enabled"
        );
        ensure!(
            settings["protocol"] == "anthropic",
            "partial patch must not reset protocol"
        );
        ensure!(settings["base_url"] == "https://api.anthropic.com");
        ensure!(
            settings["features"]["editor_assist"] == true
                && settings["features"]["comment_moderation"] == true,
            "partial patch must not reset features"
        );
        ensure!(
            settings["api_key_set"] == true,
            "partial patch must keep the api_key"
        );

        // 显式 null 与缺省同语义：字段保持不变。
        let keep = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 3,
                    "settings": { "model": null }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(keep.status == StatusCode::OK, "{}", keep.body);
        ensure!(keep.body["settings"]["model"] == "claude-2");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn settings_permission_matrix_owner_only() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-s", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-s", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-s", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-s", "moderator-pass-1").await?;

        // ManageSettings 仅 owner：editor / moderator 读写均 403。
        let editor_read = app
            .admin_get("/api/admin/settings/appearance", &editor_cookie)
            .await?;
        ensure!(
            editor_read.status == StatusCode::FORBIDDEN,
            "{}",
            editor_read.body
        );
        let moderator_write = app
            .admin_put(
                "/api/admin/settings/integrations",
                serde_json::json!({ "expected_version": 1, "settings": { "analytics_id": "x" } }),
                &moderator_cookie,
            )
            .await?;
        ensure!(
            moderator_write.status == StatusCode::FORBIDDEN,
            "{}",
            moderator_write.body
        );
        // editor 写 ai 分组同样 403。
        let editor_write_ai = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": { "enabled": true, "api_key": "sk-x" }
                }),
                &editor_cookie,
            )
            .await?;
        ensure!(
            editor_write_ai.status == StatusCode::FORBIDDEN,
            "{}",
            editor_write_ai.body
        );

        let unauthenticated = app.get("/api/admin/settings/email").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        // 无有效 Session 的写请求（伪造 Cookie）→ 401。
        let forged_write = app
            .admin_put(
                "/api/admin/settings/email",
                serde_json::json!({ "expected_version": 1, "settings": { "enabled": true } }),
                "aries_admin_session=forged-token",
            )
            .await?;
        ensure!(forged_write.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn settings_integrations_group_roundtrip() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 初始读取：预插默认行，version = 1，字段全为空。
        let initial = app
            .admin_get("/api/admin/settings/integrations", &owner_cookie)
            .await?;
        ensure!(initial.status == StatusCode::OK, "{}", initial.body);
        ensure!(initial.body["group"] == "integrations");
        ensure!(initial.body["version"].as_i64() == Some(1));
        ensure!(initial.body["settings"]["analytics_id"].is_null());
        ensure!(initial.body["settings"]["site_verification_token"].is_null());

        // 正常写入，version 递增。
        let updated = app
            .admin_put(
                "/api/admin/settings/integrations",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": {
                        "analytics_id": "G-ABCDEF1234",
                        "site_verification_token": "verify-token-1"
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["version"].as_i64() == Some(2));
        ensure!(updated.body["settings"]["analytics_id"] == "G-ABCDEF1234");

        // GET 回读与写入一致。
        let read_back = app
            .admin_get("/api/admin/settings/integrations", &owner_cookie)
            .await?;
        ensure!(read_back.status == StatusCode::OK, "{}", read_back.body);
        ensure!(read_back.body["settings"]["site_verification_token"] == "verify-token-1");

        // 乐观锁：过期版本 → 409。
        let stale = app
            .admin_put(
                "/api/admin/settings/integrations",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": { "analytics_id": "G-OTHER" }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(stale.status == StatusCode::CONFLICT, "{}", stale.body);
        ensure!(stale.body["error"]["code"] == "SETTING_VERSION_CONFLICT");

        // 字段类型非法（analytics_id 应为字符串）→ 400。
        let invalid = app
            .admin_put(
                "/api/admin/settings/integrations",
                serde_json::json!({
                    "expected_version": 2,
                    "settings": { "analytics_id": 123 }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            invalid.status == StatusCode::BAD_REQUEST,
            "{}",
            invalid.body
        );
        ensure!(invalid.body["error"]["code"] == "INVALID_SETTING_PAYLOAD");

        // 冲突与校验失败不写入、不记审计：version 仍是 2，审计仅 1 条。
        let (version,): (i32,) =
            sqlx::query_as("SELECT version FROM setting_groups WHERE grp = 'integrations'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(version == 2);
        let (audit_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs WHERE action = 'settings.integrations.updated'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(audit_count == 1);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn settings_ai_group_secret_semantics() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 初始读取：预插默认行，version = 1，api_key 未设置，protocol 默认 open_ai。
        let initial = app
            .admin_get("/api/admin/settings/ai", &owner_cookie)
            .await?;
        ensure!(initial.status == StatusCode::OK, "{}", initial.body);
        ensure!(initial.body["group"] == "ai");
        ensure!(initial.body["version"].as_i64() == Some(1));
        ensure!(initial.body["settings"]["enabled"] == false);
        ensure!(initial.body["settings"]["protocol"] == "open_ai");
        ensure!(initial.body["settings"]["api_key_set"] == false);

        // 写入完整 AI 配置 + api_key。
        let updated = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 1,
                    "settings": {
                        "enabled": true,
                        "protocol": "anthropic",
                        "base_url": "https://api.anthropic.com",
                        "model": "claude-test",
                        "api_key": "sk-ant-secret",
                        "features": { "editor_assist": true, "comment_moderation": false }
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["version"].as_i64() == Some(2));
        ensure!(updated.body["settings"]["protocol"] == "anthropic");
        ensure!(updated.body["settings"]["api_key_set"] == true);
        ensure!(updated.body["settings"]["features"]["editor_assist"] == true);
        // 响应绝不回传明文 api_key。
        ensure!(updated.body["settings"].get("api_key").is_none());

        // GET 回读：同样不得回显明文 secret。
        let read_back = app
            .admin_get("/api/admin/settings/ai", &owner_cookie)
            .await?;
        ensure!(read_back.status == StatusCode::OK, "{}", read_back.body);
        ensure!(read_back.body["settings"]["api_key_set"] == true);
        ensure!(read_back.body["settings"].get("api_key").is_none());

        // 库内仍保存明文密钥（服务端真实存储）。
        let (stored,): (serde_json::Value,) =
            sqlx::query_as("SELECT payload FROM setting_groups WHERE grp = 'ai'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(stored["api_key"] == "sk-ant-secret");

        // api_key = null：保持不变；其余字段全量覆盖。
        let keep = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 2,
                    "settings": {
                        "enabled": true,
                        "protocol": "open_ai",
                        "base_url": "https://api.openai.com",
                        "model": "gpt-test",
                        "api_key": null
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(keep.status == StatusCode::OK, "{}", keep.body);
        ensure!(keep.body["version"].as_i64() == Some(3));
        ensure!(keep.body["settings"]["model"] == "gpt-test");
        ensure!(
            keep.body["settings"]["api_key_set"] == true,
            "null must keep the secret"
        );

        // api_key = 空字符串：清除。
        let cleared = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 3,
                    "settings": { "enabled": false, "api_key": "" }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(cleared.status == StatusCode::OK, "{}", cleared.body);
        ensure!(cleared.body["settings"]["api_key_set"] == false);

        // 乐观锁：过期版本 → 409。
        let stale = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 2,
                    "settings": { "enabled": true }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(stale.status == StatusCode::CONFLICT, "{}", stale.body);
        ensure!(stale.body["error"]["code"] == "SETTING_VERSION_CONFLICT");

        // 非法 protocol 枚举值 → 400。
        let invalid = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": 4,
                    "settings": { "protocol": "gemini" }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            invalid.status == StatusCode::BAD_REQUEST,
            "{}",
            invalid.body
        );
        ensure!(invalid.body["error"]["code"] == "INVALID_SETTING_PAYLOAD");

        // 审计：settings.ai.updated ×3（冲突与 400 不计）。
        let (audit_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = 'settings.ai.updated'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(audit_count == 3);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
