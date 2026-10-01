//! Audit Log Contract Test：Admin 查询端点（action/actor/target/时间范围筛选 + 分页）+ 权限（仅 Owner）。
//! 审计数据一部分由真实 Bootstrap/Login 操作端到端产生，一部分直接 SQL 插入固定 Fixture。
//! 通过 `ARIES_RUN_DATABASE_TESTS=1` 门控，随机 Schema 隔离。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

/// 直接 SQL 插入一条 audit_logs 前置数据（created_at 可控，便于时间范围断言）。
async fn insert_audit(
    app: &TestApp,
    actor_user_id: Option<i64>,
    action: &str,
    target_type: &str,
    target_id: Option<&str>,
    metadata: serde_json::Value,
    created_at: &str,
) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO audit_logs (actor_user_id, action, target_type, target_id, metadata, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6::timestamptz) \
         RETURNING id",
    )
    .bind(actor_user_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(metadata)
    .bind(created_at)
    .fetch_one(&app.state.database)
    .await
    .context("insert audit log")?;
    Ok(id)
}

/// 创建指定角色的用户并走真实登录端点，返回 Session Cookie 串。
async fn create_user_and_login(
    app: &TestApp,
    username: &str,
    role: &str,
    password: &str,
) -> anyhow::Result<String> {
    let password_hash = app.state.passwords.hash(password)?;
    sqlx::query(
        "INSERT INTO users (username, email, password_hash, display_name, role, status) \
         VALUES ($1, $2, $3, $4, $5, 'active')",
    )
    .bind(username)
    .bind(format!("{username}@example.com"))
    .bind(&password_hash)
    .bind(username)
    .bind(role)
    .execute(&app.state.database)
    .await?;
    let login = app
        .admin_post(
            "/api/admin/auth/login",
            serde_json::json!({ "login": username, "password": password }),
            None,
        )
        .await?;
    ensure!(login.status == StatusCode::OK, "{}", login.body);
    login
        .set_cookie
        .context("login did not set cookie")?
        .split(';')
        .next()
        .map(str::to_owned)
        .context("failed to parse session cookie")
}

#[tokio::test]
async fn audit_logs_filters_pagination_and_permissions() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // Bootstrap 自身写入 auth.bootstrap（端到端验证审计写入链路，含 actor_username 联查）。
        let bootstrapped = app
            .admin_get("/api/admin/audit-logs?action=auth.bootstrap", &owner_cookie)
            .await?;
        ensure!(
            bootstrapped.status == StatusCode::OK,
            "{}",
            bootstrapped.body
        );
        ensure!(bootstrapped.body["total"].as_i64() == Some(1));
        ensure!(bootstrapped.body["items"][0]["actor_username"] == "owner");
        ensure!(bootstrapped.body["items"][0]["target_type"] == "user");

        let (owner_id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE username = 'owner'")
            .fetch_one(&app.state.database)
            .await?;

        // 清空端到端产生的事件，换固定 Fixture，保证计数断言确定。
        sqlx::query("DELETE FROM audit_logs")
            .execute(&app.state.database)
            .await?;

        // 前置数据：不同 action/target/actor/时间。
        insert_audit(
            &app,
            Some(owner_id),
            "article.created",
            "article",
            Some("10"),
            serde_json::json!({ "title": "hello" }),
            "2026-09-05T10:00:00Z",
        )
        .await?;
        insert_audit(
            &app,
            Some(owner_id),
            "article.updated",
            "article",
            Some("10"),
            serde_json::json!({}),
            "2026-09-05T11:00:00Z",
        )
        .await?;
        insert_audit(
            &app,
            Some(owner_id),
            "article.created",
            "article",
            Some("11"),
            serde_json::json!({}),
            "2026-09-05T12:00:00Z",
        )
        .await?;
        // 匿名 actor（登录失败场景）：actor_user_id / actor_username 均为 null。
        insert_audit(
            &app,
            None,
            "auth.login_failed",
            "user",
            None,
            serde_json::json!({ "login": "ghost" }),
            "2026-09-05T13:00:00Z",
        )
        .await?;

        // 全量：created_at desc, id desc；响应字段齐全。
        let list = app
            .admin_get("/api/admin/audit-logs", &owner_cookie)
            .await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body["total"].as_i64() == Some(4));
        ensure!(list.body["page"].as_u64() == Some(1));
        ensure!(list.body["page_size"].as_u64() == Some(20));
        let items = list.body["items"].as_array().context("missing items")?;
        ensure!(items.len() == 4);
        let first = &items[0];
        // 最新一条在前；匿名 actor 两个字段均为 null。
        ensure!(first["action"] == "auth.login_failed");
        ensure!(first["actor_user_id"].is_null());
        ensure!(first["actor_username"].is_null());
        ensure!(first["metadata"]["login"] == "ghost");
        // created_at 为 time 默认 serde 数组形式 [year, ordinal, hour, …]
        // （logs 端点显式格式化为 RFC 3339，此处未做，按实际契约断言）。
        let created_at = first["created_at"]
            .as_array()
            .context("created_at is not an array")?;
        ensure!(created_at[0].as_i64() == Some(2026), "{first}");
        // actor_username 联查：有 actor 的行带回用户名。
        ensure!(items[1]["actor_username"] == "owner");

        // action 精确匹配；空串按不过滤处理。
        let by_action = app
            .admin_get(
                "/api/admin/audit-logs?action=article.created",
                &owner_cookie,
            )
            .await?;
        ensure!(by_action.body["total"].as_i64() == Some(2));
        let empty_action = app
            .admin_get("/api/admin/audit-logs?action=", &owner_cookie)
            .await?;
        ensure!(empty_action.body["total"].as_i64() == Some(4));

        // actor 过滤：匿名行被排除；不存在的 actor 命中 0。
        let by_actor = app
            .admin_get(
                &format!("/api/admin/audit-logs?actor_user_id={owner_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(by_actor.body["total"].as_i64() == Some(3));
        let items = by_actor.body["items"].as_array().context("missing items")?;
        ensure!(items.iter().all(|item| item["actor_username"] == "owner"));
        let unknown_actor = app
            .admin_get("/api/admin/audit-logs?actor_user_id=999999", &owner_cookie)
            .await?;
        ensure!(unknown_actor.body["total"].as_i64() == Some(0));

        // target 过滤：target_type 与 target_id 可独立或组合。
        let by_type = app
            .admin_get("/api/admin/audit-logs?target_type=article", &owner_cookie)
            .await?;
        ensure!(by_type.body["total"].as_i64() == Some(3));
        let by_target = app
            .admin_get(
                "/api/admin/audit-logs?target_type=article&target_id=10",
                &owner_cookie,
            )
            .await?;
        ensure!(by_target.body["total"].as_i64() == Some(2));
        let by_target_id_only = app
            .admin_get("/api/admin/audit-logs?target_id=11", &owner_cookie)
            .await?;
        ensure!(by_target_id_only.body["total"].as_i64() == Some(1));

        // 时间范围：按 created_at 左闭右开。
        let ranged = app
            .admin_get(
                "/api/admin/audit-logs?start=2026-09-05T11:00:00Z&end=2026-09-05T13:00:00Z",
                &owner_cookie,
            )
            .await?;
        ensure!(ranged.body["total"].as_i64() == Some(2), "{}", ranged.body);
        let boundary = app
            .admin_get(
                "/api/admin/audit-logs?start=2026-09-05T10:00:00Z&end=2026-09-05T11:00:00Z",
                &owner_cookie,
            )
            .await?;
        ensure!(
            boundary.body["total"].as_i64() == Some(1),
            "{}",
            boundary.body
        );
        // 非法日期 → 400 INVALID_DATE_RANGE。
        let bad_start = app
            .admin_get("/api/admin/audit-logs?start=not-a-date", &owner_cookie)
            .await?;
        ensure!(bad_start.status == StatusCode::BAD_REQUEST);
        ensure!(bad_start.body["error"]["code"] == "INVALID_DATE_RANGE");
        let bad_end = app
            .admin_get("/api/admin/audit-logs?end=not-a-date", &owner_cookie)
            .await?;
        ensure!(bad_end.status == StatusCode::BAD_REQUEST);
        ensure!(bad_end.body["error"]["code"] == "INVALID_DATE_RANGE");

        // 分页：页语义正确，page_size clamp 到 100。
        let paged = app
            .admin_get("/api/admin/audit-logs?page=2&page_size=2", &owner_cookie)
            .await?;
        ensure!(paged.body["page"].as_u64() == Some(2));
        ensure!(paged.body["page_size"].as_u64() == Some(2));
        ensure!(paged.body["total"].as_i64() == Some(4));
        let items = paged.body["items"].as_array().context("missing items")?;
        ensure!(items.len() == 2);
        // 时间线第 3、4 条（倒序）：11:00 article.updated、10:00 article.created。
        ensure!(items[0]["action"] == "article.updated");
        ensure!(items[1]["action"] == "article.created");
        let clamped = app
            .admin_get("/api/admin/audit-logs?page_size=500", &owner_cookie)
            .await?;
        ensure!(clamped.body["page_size"].as_u64() == Some(100));
        ensure!(clamped.body["items"].as_array().map(Vec::len) == Some(4));

        // 权限：Audit Log 仅 Owner（ManageSettings）可访问，editor 403。
        let editor_cookie =
            create_user_and_login(&app, "editor-audit", "editor", "editor-pass-1").await?;
        let denied = app
            .admin_get("/api/admin/audit-logs", &editor_cookie)
            .await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);
        ensure!(denied.body["error"]["code"] == "PERMISSION_DENIED");
        // 未认证 401。
        let unauthenticated = app.get("/api/admin/audit-logs").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn audit_login_events_recorded_end_to_end() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        app.bootstrap_owner().await?;

        // 失败登录：用户存在，actor_user_id 有值，metadata 记录尝试的 login 名。
        let failed = app
            .admin_post(
                "/api/admin/auth/login",
                serde_json::json!({ "login": "owner", "password": "wrong-password-1" }),
                None,
            )
            .await?;
        ensure!(failed.status == StatusCode::UNAUTHORIZED, "{}", failed.body);

        // 成功登录：写 auth.login_succeeded（同时撤销 Bootstrap 签发的旧 Session）。
        let login = app
            .admin_post(
                "/api/admin/auth/login",
                serde_json::json!({ "login": "owner", "password": common::OWNER_PASSWORD }),
                None,
            )
            .await?;
        ensure!(login.status == StatusCode::OK, "{}", login.body);
        let owner_cookie = login
            .set_cookie
            .context("login did not set cookie")?
            .split(';')
            .next()
            .context("failed to parse session cookie")?
            .to_owned();

        let (owner_id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE username = 'owner'")
            .fetch_one(&app.state.database)
            .await?;

        let failed_events = app
            .admin_get(
                "/api/admin/audit-logs?action=auth.login_failed",
                &owner_cookie,
            )
            .await?;
        ensure!(failed_events.body["total"].as_i64() == Some(1));
        let entry = &failed_events.body["items"][0];
        ensure!(entry["actor_user_id"].as_i64() == Some(owner_id));
        ensure!(entry["target_type"] == "user");
        ensure!(entry["target_id"].as_str() == Some(owner_id.to_string().as_str()));
        ensure!(entry["metadata"]["login"] == "owner");

        let succeeded_events = app
            .admin_get(
                "/api/admin/audit-logs?action=auth.login_succeeded",
                &owner_cookie,
            )
            .await?;
        ensure!(succeeded_events.body["total"].as_i64() == Some(1));
        ensure!(succeeded_events.body["items"][0]["target_type"] == "session");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
