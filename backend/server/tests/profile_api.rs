//! Contract Test：个人资料读写与修改密码，重点覆盖旧密码校验与
//! 改密成功后全量撤销 Session 的安全语义。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::{OWNER_PASSWORD, TestApp};

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
async fn profile_read_update_and_validation() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 初始读取：返回 Bootstrap 时写入的资料与权限列表。
        let profile = app.admin_get("/api/admin/profile", &cookie).await?;
        ensure!(profile.status == StatusCode::OK, "{}", profile.body);
        ensure!(profile.body["username"] == "owner");
        ensure!(profile.body["email"] == "owner@example.com");
        ensure!(profile.body["display_name"] == "Owner");
        ensure!(profile.body["role"] == "owner");
        ensure!(
            profile.body["permissions"]
                .as_array()
                .is_some_and(|permissions| permissions
                    .iter()
                    .any(|permission| permission == "profile:manage")),
            "{}",
            profile.body
        );

        // 正常更新：Email 归一化为 trim + 小写，头像允许 http(s) URL。
        let updated = app
            .admin_put(
                "/api/admin/profile",
                serde_json::json!({
                    "email": "  NewOwner@Example.COM ",
                    "display_name": "站长",
                    "avatar_url": "https://cdn.example.com/avatar.png"
                }),
                &cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["email"] == "newowner@example.com");
        ensure!(updated.body["display_name"] == "站长");
        ensure!(updated.body["avatar_url"] == "https://cdn.example.com/avatar.png");

        // GET 回读确认更新已落库。
        let reread = app.admin_get("/api/admin/profile", &cookie).await?;
        ensure!(reread.status == StatusCode::OK, "{}", reread.body);
        ensure!(reread.body["email"] == "newowner@example.com");
        ensure!(reread.body["display_name"] == "站长");

        // 非法 Email → 400 INVALID_EMAIL。
        let bad_email = app
            .admin_put(
                "/api/admin/profile",
                serde_json::json!({
                    "email": "not-an-email",
                    "display_name": "站长"
                }),
                &cookie,
            )
            .await?;
        ensure!(
            bad_email.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_email.body
        );
        ensure!(bad_email.body["error"]["code"] == "INVALID_EMAIL");

        // 空 Display Name → 400 INVALID_DISPLAY_NAME。
        let empty_name = app
            .admin_put(
                "/api/admin/profile",
                serde_json::json!({
                    "email": "newowner@example.com",
                    "display_name": "   "
                }),
                &cookie,
            )
            .await?;
        ensure!(
            empty_name.status == StatusCode::BAD_REQUEST,
            "{}",
            empty_name.body
        );
        ensure!(empty_name.body["error"]["code"] == "INVALID_DISPLAY_NAME");

        // 非 http(s) 头像 URL → 400 INVALID_AVATAR_URL。
        let bad_avatar = app
            .admin_put(
                "/api/admin/profile",
                serde_json::json!({
                    "email": "newowner@example.com",
                    "display_name": "站长",
                    "avatar_url": "javascript:alert(1)"
                }),
                &cookie,
            )
            .await?;
        ensure!(
            bad_avatar.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_avatar.body
        );
        ensure!(bad_avatar.body["error"]["code"] == "INVALID_AVATAR_URL");

        // 校验失败不得写入：资料保持上一次成功更新的值。
        let after_failures = app.admin_get("/api/admin/profile", &cookie).await?;
        ensure!(after_failures.body["display_name"] == "站长");

        // 未认证请求 → 401。
        let anonymous = app.get("/api/admin/profile").await?;
        ensure!(anonymous.status == StatusCode::UNAUTHORIZED);

        // 审计：仅成功更新写入一条 profile.updated。
        let (audit_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = 'profile.updated'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(
            audit_count == 1,
            "expected 1 profile audit, got {audit_count}"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn profile_password_change_revokes_all_sessions() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let bootstrap_cookie = app.bootstrap_owner().await?;
        // 再次登录签发第二个 Session（登录只撤销请求方自带 Cookie 的 Session），
        // 模拟同一账号的多端会话。
        let second_cookie = login_cookie(&app, "owner", OWNER_PASSWORD).await?;
        let both_valid = app
            .admin_get("/api/admin/profile", &bootstrap_cookie)
            .await?;
        ensure!(both_valid.status == StatusCode::OK, "{}", both_valid.body);

        // 旧密码错误 → 400 INVALID_CURRENT_PASSWORD，且当前会话保持有效。
        let wrong_current = app
            .admin_put(
                "/api/admin/profile/password",
                serde_json::json!({
                    "current_password": "wrong-password-1",
                    "new_password": "new-password-2026"
                }),
                &second_cookie,
            )
            .await?;
        ensure!(
            wrong_current.status == StatusCode::BAD_REQUEST,
            "{}",
            wrong_current.body
        );
        ensure!(wrong_current.body["error"]["code"] == "INVALID_CURRENT_PASSWORD");
        let still_valid = app.admin_get("/api/admin/profile", &second_cookie).await?;
        ensure!(still_valid.status == StatusCode::OK, "{}", still_valid.body);

        // 新密码不符合强度要求 → 400。
        let weak = app
            .admin_put(
                "/api/admin/profile/password",
                serde_json::json!({
                    "current_password": OWNER_PASSWORD,
                    "new_password": "short"
                }),
                &second_cookie,
            )
            .await?;
        ensure!(weak.status == StatusCode::BAD_REQUEST, "{}", weak.body);

        // 未认证请求 → 401。
        let anonymous = app
            .admin_put(
                "/api/admin/profile/password",
                serde_json::json!({
                    "current_password": OWNER_PASSWORD,
                    "new_password": "new-password-2026"
                }),
                "aries_admin_session=missing-token",
            )
            .await?;
        ensure!(anonymous.status == StatusCode::UNAUTHORIZED);

        // 正确修改 → 204，响应通过 Set-Cookie 清除当前会话 Cookie。
        let changed = app
            .admin_put(
                "/api/admin/profile/password",
                serde_json::json!({
                    "current_password": OWNER_PASSWORD,
                    "new_password": "new-password-2026"
                }),
                &second_cookie,
            )
            .await?;
        ensure!(changed.status == StatusCode::NO_CONTENT, "{}", changed.body);
        let cleared = changed
            .set_cookie
            .context("password change must clear session cookie")?;
        ensure!(cleared.starts_with("aries_admin_session="), "{cleared}");
        ensure!(cleared.contains("Max-Age=0"), "{cleared}");

        // 安全语义：改密撤销该用户全部 Session，两个旧 Cookie 再请求均 401。
        let revoked_second = app.admin_get("/api/admin/profile", &second_cookie).await?;
        ensure!(
            revoked_second.status == StatusCode::UNAUTHORIZED,
            "{}",
            revoked_second.body
        );
        let revoked_bootstrap = app
            .admin_get("/api/admin/profile", &bootstrap_cookie)
            .await?;
        ensure!(
            revoked_bootstrap.status == StatusCode::UNAUTHORIZED,
            "{}",
            revoked_bootstrap.body
        );

        // 旧密码登录 → 401；新密码登录 → 200。
        let old_login = app
            .admin_post(
                "/api/admin/auth/login",
                serde_json::json!({ "login": "owner", "password": OWNER_PASSWORD }),
                None,
            )
            .await?;
        ensure!(
            old_login.status == StatusCode::UNAUTHORIZED,
            "{}",
            old_login.body
        );
        let _new_cookie = login_cookie(&app, "owner", "new-password-2026").await?;

        // 审计：改密成功写入一条 profile.password_changed。
        let (audit_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs WHERE action = 'profile.password_changed'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(
            audit_count == 1,
            "expected 1 password audit, got {audit_count}"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
