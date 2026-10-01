//! 运行日志 Contract Test：Admin 查询端点（筛选/分页/聚合）+ SQL 日志开关 + 落库写入链路。
//! 通过 `ARIES_RUN_DATABASE_TESTS=1` 门控，随机 Schema 隔离。

mod common;

use std::time::{Duration, Instant};

use anyhow::{Context, ensure};
use axum::http::{StatusCode, header};
use http_body_util::BodyExt;

use common::TestApp;

/// 直接 SQL 插入一条 server_logs 前置数据。
async fn insert_log(
    app: &TestApp,
    ts: &str,
    level: &str,
    target: &str,
    message: &str,
    request_id: Option<&str>,
) -> anyhow::Result<i64> {
    insert_log_with_fields(
        app,
        ts,
        level,
        target,
        message,
        request_id,
        serde_json::json!({}),
    )
    .await
}

/// 带 fields 的插入变体：keyword 搜索覆盖 fields JSON 文本的场景用。
async fn insert_log_with_fields(
    app: &TestApp,
    ts: &str,
    level: &str,
    target: &str,
    message: &str,
    request_id: Option<&str>,
    fields: serde_json::Value,
) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO server_logs (ts, level, target, message, request_id, fields) \
         VALUES ($1::timestamptz, $2, $3, $4, $5, $6) \
         RETURNING id",
    )
    .bind(ts)
    .bind(level)
    .bind(target)
    .bind(message)
    .bind(request_id)
    .bind(fields)
    .fetch_one(&app.state.database)
    .await
    .context("insert server log")?;
    Ok(id)
}

#[tokio::test]
async fn logs_api_filters_pagination_and_level_counts() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 前置数据：不同级别/目标/时间/关键字。
        insert_log(
            &app,
            "2026-09-05T10:00:00Z",
            "ERROR",
            "aries_server::http",
            "failed to save",
            Some("req-1"),
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T11:00:00Z",
            "WARN",
            "aries_server::http",
            "slow request",
            Some("req-2"),
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T12:00:00Z",
            "INFO",
            "aries_server::http",
            "request completed",
            Some("req-3"),
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T13:00:00Z",
            "DEBUG",
            "sqlx::query",
            "SELECT 1",
            None,
        )
        .await?;
        // 最近 24h 内的一条，计入 level_counts。
        insert_log(
            &app,
            "now",
            "ERROR",
            "aries_server::worker",
            "job failed",
            None,
        )
        .await?;

        // 全量：ts desc, id desc。
        let list = app.admin_get("/api/admin/logs", &owner_cookie).await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body["total"].as_i64() == Some(5));
        let items = list.body["items"].as_array().context("missing items")?;
        let first = &items[0];
        // 最新一条在前；ts 为 RFC3339 字符串。
        ensure!(first["level"] == "ERROR");
        ensure!(first["target"] == "aries_server::worker");
        ensure!(
            first["ts"].is_string(),
            "ts is not a string: {}",
            first["ts"]
        );
        ensure!(first["ts"].as_str().context("missing ts")?.contains('T'));
        ensure!(first["fields"].is_object());

        // level 语义为「最低级别」：WARN 含 WARN+ERROR。
        let warned = app
            .admin_get("/api/admin/logs?level=WARN", &owner_cookie)
            .await?;
        ensure!(warned.body["total"].as_i64() == Some(3), "{}", warned.body);
        // level=ERROR 只含 ERROR。
        let errors = app
            .admin_get("/api/admin/logs?level=ERROR", &owner_cookie)
            .await?;
        ensure!(errors.body["total"].as_i64() == Some(2));

        // level_counts：最近 24h 聚合，五个键齐全；只有 "now" 那条确定在窗口内。
        let counts = &list.body["level_counts"];
        for key in ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] {
            ensure!(counts[key].is_i64(), "missing key {key}");
        }
        ensure!(counts["ERROR"].as_i64() >= Some(1), "{counts}");

        // target 精确匹配。
        let by_target = app
            .admin_get("/api/admin/logs?target=sqlx::query", &owner_cookie)
            .await?;
        ensure!(by_target.body["total"].as_i64() == Some(1));

        // keyword ILIKE；% 按字面量转义，不当通配符。
        let by_keyword = app
            .admin_get("/api/admin/logs?keyword=request", &owner_cookie)
            .await?;
        ensure!(by_keyword.body["total"].as_i64() == Some(2));
        let wildcard = app
            .admin_get("/api/admin/logs?keyword=100%25", &owner_cookie)
            .await?;
        ensure!(
            wildcard.body["total"].as_i64() == Some(0),
            "{}",
            wildcard.body
        );

        // 时间范围：左闭右开。
        let ranged = app
            .admin_get(
                "/api/admin/logs?start=2026-09-05T10:00:00Z&end=2026-09-05T12:00:00Z",
                &owner_cookie,
            )
            .await?;
        ensure!(ranged.body["total"].as_i64() == Some(2), "{}", ranged.body);
        // 非法日期 → 400 INVALID_DATE_RANGE。
        let bad_date = app
            .admin_get("/api/admin/logs?start=not-a-date", &owner_cookie)
            .await?;
        ensure!(bad_date.status == StatusCode::BAD_REQUEST);
        ensure!(bad_date.body["error"]["code"] == "INVALID_DATE_RANGE");

        // 分页：page_size clamp 到 200，页语义正确。
        let paged = app
            .admin_get("/api/admin/logs?page=2&page_size=2", &owner_cookie)
            .await?;
        ensure!(paged.body["items"].as_array().map(Vec::len) == Some(2));
        ensure!(paged.body["page"].as_u64() == Some(2));

        // targets 端点：去重排序。
        let targets = app
            .admin_get("/api/admin/logs/targets", &owner_cookie)
            .await?;
        ensure!(targets.status == StatusCode::OK, "{}", targets.body);
        let items = targets.body["items"]
            .as_array()
            .context("missing targets")?;
        ensure!(items.iter().any(|t| t == "sqlx::query"));
        ensure!(items.iter().any(|t| t == "aries_server::http"));

        // 权限：editor 403，未认证 401。
        let password_hash = app.state.passwords.hash("editor-pass-1")?;
        sqlx::query(
            "INSERT INTO users (username, email, password_hash, display_name, role, status) \
             VALUES ('editor-logs', 'editor-logs@example.com', $1, 'Editor', 'editor', 'active')",
        )
        .bind(&password_hash)
        .execute(&app.state.database)
        .await?;
        let login = app
            .admin_post(
                "/api/admin/auth/login",
                serde_json::json!({ "login": "editor-logs", "password": "editor-pass-1" }),
                None,
            )
            .await?;
        let editor_cookie = login
            .set_cookie
            .context("login did not set cookie")?
            .split(';')
            .next()
            .context("failed to parse session cookie")?
            .to_owned();
        let denied = app.admin_get("/api/admin/logs", &editor_cookie).await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);
        let unauthenticated = app.get("/api/admin/logs").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn keyword_search_matches_fields_json_text() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // SQL 日志：message 为摘要占位，完整语句在 fields（贴近 sqlx 0.8 事件形状）。
        insert_log_with_fields(
            &app,
            "2026-09-05T10:00:00Z",
            "DEBUG",
            "sqlx::query",
            "",
            None,
            serde_json::json!({
                "summary": "SELECT id, title FROM articles WHERE …",
                "db.statement": "SELECT id, title FROM articles WHERE status = $1",
            }),
        )
        .await?;
        // 对照行：message 与 fields 都不含目标关键词。
        insert_log(
            &app,
            "2026-09-05T11:00:00Z",
            "INFO",
            "aries_server::http",
            "done",
            None,
        )
        .await?;

        // keyword 命中 fields 中的 SQL 语句（message 里没有该词）。
        let by_fields = app
            .admin_get("/api/admin/logs?keyword=articles", &owner_cookie)
            .await?;
        ensure!(
            by_fields.body["total"].as_i64() == Some(1),
            "{}",
            by_fields.body
        );
        ensure!(by_fields.body["items"][0]["target"] == "sqlx::query");

        // keyword 仍能命中 message。
        let by_message = app
            .admin_get("/api/admin/logs?keyword=done", &owner_cookie)
            .await?;
        ensure!(by_message.body["total"].as_i64() == Some(1));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn sql_logging_toggle_is_runtime_switchable() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 初始关闭；PUT 切换后 GET 回读（测试 Harness 用 noop 句柄，只验证 API 与状态位）。
        let initial = app.admin_get("/api/admin/logs/sql", &owner_cookie).await?;
        ensure!(initial.status == StatusCode::OK, "{}", initial.body);
        ensure!(initial.body["enabled"] == false);

        let toggled = app
            .admin_put(
                "/api/admin/logs/sql",
                serde_json::json!({ "enabled": true }),
                &owner_cookie,
            )
            .await?;
        ensure!(toggled.status == StatusCode::OK, "{}", toggled.body);
        ensure!(toggled.body["enabled"] == true);
        ensure!(app.state.log_handle.sql_enabled());

        let off = app
            .admin_put(
                "/api/admin/logs/sql",
                serde_json::json!({ "enabled": false }),
                &owner_cookie,
            )
            .await?;
        ensure!(off.body["enabled"] == false);
        ensure!(!app.state.log_handle.sql_enabled());
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn db_log_writer_persists_events_end_to_end() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        use tracing_subscriber::layer::SubscriberExt;

        // 用独立 Dispatch 驱动 DbLogLayer（测试进程不初始化全局 Subscriber）。
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let dispatch = tracing::Dispatch::new(
            tracing_subscriber::registry().with(aries_server::log_store::DbLogLayer::new(tx)),
        );
        tracing::dispatcher::with_default(&dispatch, || {
            let span = tracing::info_span!("http_request", request_id = "req-e2e");
            let _guard = span.enter();
            tracing::info!(path = "/api/test", "writer roundtrip");
        });
        // 释放发送端：writer 收到剩余记录后 flush 并退出。
        drop(dispatch);

        let handle = aries_server::log_store::spawn_writer(app.state.database.clone(), rx, 14);
        handle.await.context("writer task failed")?;

        // 事件已落库：内容、Span 名与 request_id 齐全。
        let row: (String, String, String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT level, target, message, span_name, request_id \
             FROM server_logs WHERE message = 'writer roundtrip'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(row.0 == "INFO");
        ensure!(row.2 == "writer roundtrip");
        ensure!(row.3.as_deref() == Some("http_request"));
        ensure!(row.4.as_deref() == Some("req-e2e"));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn logs_api_request_id_tracing_and_order() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 同一请求的三条链路日志 + 其他请求的噪音日志。
        insert_log(
            &app,
            "2026-09-05T10:00:01Z",
            "INFO",
            "aries_server::http",
            "request started",
            Some("trace-1"),
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T10:00:02Z",
            "DEBUG",
            "sqlx::query",
            "SELECT * FROM articles",
            Some("trace-1"),
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T10:00:03Z",
            "WARN",
            "aries_server::http",
            "request completed slow",
            Some("trace-1"),
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T10:00:02Z",
            "INFO",
            "aries_server::http",
            "other request",
            Some("trace-2"),
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T10:00:04Z",
            "ERROR",
            "aries_server::worker",
            "unrelated job",
            None,
        )
        .await?;

        // request_id 精确过滤：其他请求的日志不出现。
        let traced = app
            .admin_get("/api/admin/logs?request_id=trace-1", &owner_cookie)
            .await?;
        ensure!(traced.status == StatusCode::OK, "{}", traced.body);
        ensure!(traced.body["total"].as_i64() == Some(3), "{}", traced.body);
        let items = traced.body["items"].as_array().context("missing items")?;
        ensure!(items.iter().all(|item| item["request_id"] == "trace-1"));
        // 默认 desc：最新的在前。
        ensure!(items[0]["message"] == "request completed slow");

        // order=asc：按时间正序看链路流转。
        let ascending = app
            .admin_get(
                "/api/admin/logs?request_id=trace-1&order=asc",
                &owner_cookie,
            )
            .await?;
        let items = ascending.body["items"]
            .as_array()
            .context("missing items")?;
        let messages: Vec<&str> = items
            .iter()
            .filter_map(|item| item["message"].as_str())
            .collect();
        ensure!(
            messages
                == vec![
                    "request started",
                    "SELECT * FROM articles",
                    "request completed slow"
                ],
            "{messages:?}"
        );

        // 未知 order 值按 desc 宽松处理。
        let unknown_order = app
            .admin_get(
                "/api/admin/logs?request_id=trace-1&order=sideways",
                &owner_cookie,
            )
            .await?;
        ensure!(unknown_order.body["items"][0]["message"] == "request completed slow");

        // 组合过滤：request_id + level（最低级别）。
        let combined = app
            .admin_get(
                "/api/admin/logs?request_id=trace-1&level=WARN",
                &owner_cookie,
            )
            .await?;
        ensure!(
            combined.body["total"].as_i64() == Some(1),
            "{}",
            combined.body
        );
        ensure!(combined.body["items"][0]["level"] == "WARN");

        // 组合过滤：request_id + keyword。
        let combined = app
            .admin_get(
                "/api/admin/logs?request_id=trace-1&keyword=started",
                &owner_cookie,
            )
            .await?;
        ensure!(
            combined.body["total"].as_i64() == Some(1),
            "{}",
            combined.body
        );
        ensure!(combined.body["items"][0]["message"] == "request started");

        // 空串 request_id 忽略（等价于不带该参数）。
        let all = app
            .admin_get("/api/admin/logs?request_id=", &owner_cookie)
            .await?;
        ensure!(all.body["total"].as_i64() == Some(5), "{}", all.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn request_params_are_logged_redacted_and_linked_to_request() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        use tracing_subscriber::layer::SubscriberExt;

        // 测试进程不初始化全局 Subscriber：用线程默认 Dispatch 挂 DbLogLayer，
        // 让真实请求（TraceLayer Span + 参数中间件）的日志进入 channel。
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let dispatch = tracing::Dispatch::new(
            tracing_subscriber::registry().with(aries_server::log_store::DbLogLayer::new(tx)),
        );
        let guard = tracing::dispatcher::set_default(&dispatch);

        // 登录请求：password 是敏感键。
        let login = app
            .admin_post(
                "/api/admin/auth/login",
                serde_json::json!({ "login": "owner", "password": "plaintext-must-not-appear" }),
                None,
            )
            .await?;
        // 未 bootstrap，登录会 401，但参数日志已产生。
        ensure!(login.status == StatusCode::UNAUTHORIZED, "{}", login.body);

        // Bootstrap：bootstrap_secret 也是敏感键。
        let bootstrap = app
            .admin_post(
                "/api/admin/bootstrap",
                serde_json::json!({
                    "bootstrap_secret": common::BOOTSTRAP_SECRET,
                    "username": "owner",
                    "email": "owner@example.com",
                    "display_name": "Owner",
                    "password": common::OWNER_PASSWORD,
                }),
                None,
            )
            .await?;
        ensure!(bootstrap.status == StatusCode::OK, "{}", bootstrap.body);

        // GET 请求不产生参数日志。
        app.get("/api/health/live").await?;

        // 释放发送端：writer 收尾 flush 后退出。
        drop(guard);
        drop(dispatch);
        let handle = aries_server::log_store::spawn_writer(app.state.database.clone(), rx, 14);
        handle.await.context("writer task failed")?;

        // 拉取参数日志：request_id 关联、http.path 注入、敏感值脱敏。
        let rows: Vec<(String, Option<String>, serde_json::Value)> = sqlx::query_as(
            "SELECT message, request_id, fields FROM server_logs \
             WHERE message = 'http request params' ORDER BY id",
        )
        .fetch_all(&app.state.database)
        .await?;
        ensure!(rows.len() == 2, "expected 2 param logs, got {}", rows.len());

        for (_message, request_id, fields) in &rows {
            ensure!(request_id.is_some(), "param log missing request_id");
            // Span 上下文注入：http.path 直接在 fields 里可见。
            let path = fields["http.path"].as_str().context("missing http.path")?;
            ensure!(path.starts_with("/api/admin/"), "{path}");
            let params: serde_json::Value =
                serde_json::from_str(fields["params"].as_str().context("missing params")?)?;
            let text = params.to_string();
            ensure!(!text.contains("plaintext-must-not-appear"), "{text}");
            ensure!(!text.contains(common::OWNER_PASSWORD), "{text}");
            ensure!(!text.contains(common::BOOTSTRAP_SECRET), "{text}");
        }
        // 登录行的 password 被替换为 "***"。
        let login_params: serde_json::Value =
            serde_json::from_str(rows[0].2["params"].as_str().context("missing params")?)?;
        ensure!(login_params["password"] == "***", "{login_params}");
        ensure!(login_params["login"] == "owner", "{login_params}");
        ensure!(rows[0].2["http.path"] == "/api/admin/auth/login");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn sql_logging_never_persists_writer_own_inserts() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        use tracing_subscriber::layer::SubscriberExt;

        // 模拟 SQL 日志开启：DbLogLayer 不带级别过滤，sqlx::query 的 debug 事件全部通过。
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let dispatch = tracing::Dispatch::new(
            tracing_subscriber::registry().with(aries_server::log_store::DbLogLayer::new(tx)),
        );
        let guard = tracing::dispatcher::set_default(&dispatch);

        // 先起 writer：其批量 INSERT 产生的 sqlx 事件会回流到 DbLogLayer（同线程 runtime）。
        let handle = aries_server::log_store::spawn_writer(app.state.database.clone(), rx, 14);

        // 业务日志 + 业务 SQL（应正常入库）。
        tracing::info!("business event");
        sqlx::query("SELECT id FROM articles LIMIT 1")
            .fetch_all(&app.state.database)
            .await?;

        // 等 writer 至少完成一次 flush（500ms 间隔；并发全量测试下放宽 deadline）。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let (count,): (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM server_logs WHERE message = 'business event'")
                    .fetch_one(&app.state.database)
                    .await?;
            let (sql_count,): (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM server_logs \
                 WHERE target = 'sqlx::query' AND fields->>'summary' LIKE '%FROM articles%'",
            )
            .fetch_one(&app.state.database)
            .await?;
            if count == 1 && sql_count >= 1 {
                break;
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "business logs not persisted in time (event={count}, sql={sql_count})"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // 再等两个 flush 周期，捕获可能漏网的自身 INSERT 事件。
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

        drop(guard);
        drop(dispatch);
        handle.await.context("writer task failed")?;

        // 业务 SQL 日志正常入库（轮询循环已保证 ≥1，此处确认语义）。
        let (business_sql,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM server_logs \
             WHERE target = 'sqlx::query' AND fields->>'summary' LIKE '%FROM articles%'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(business_sql >= 1, "business sql log should be persisted");

        // 自身 INSERT 未入库：message / summary / db.statement 任一含 server_logs 都不得存在。
        let (self_inserts,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM server_logs \
             WHERE target = 'sqlx::query' AND (\
               message ILIKE '%server_logs%' \
               OR fields->>'summary' ILIKE '%server_logs%' \
               OR fields->>'db.statement' ILIKE '%server_logs%')",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(
            self_inserts == 0,
            "writer self-insert leaked into server_logs"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn logs_api_exclude_target_and_context_window() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 五条连续时间线：两条 sqlx 噪音 + 三条业务日志。
        insert_log(
            &app,
            "2026-09-05T10:00:00Z",
            "DEBUG",
            "sqlx::query",
            "SELECT 1",
            None,
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T10:01:00Z",
            "INFO",
            "aries_server::http",
            "first",
            None,
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T10:02:00Z",
            "WARN",
            "aries_server::http",
            "second",
            None,
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T10:03:00Z",
            "DEBUG",
            "sqlx::query",
            "SELECT 2",
            None,
        )
        .await?;
        insert_log(
            &app,
            "2026-09-05T10:04:00Z",
            "ERROR",
            "aries_server::http",
            "third",
            None,
        )
        .await?;

        // exclude_target：sqlx 噪音被排除。
        let excluded = app
            .admin_get("/api/admin/logs?exclude_target=sqlx::query", &owner_cookie)
            .await?;
        ensure!(excluded.status == StatusCode::OK, "{}", excluded.body);
        ensure!(
            excluded.body["total"].as_i64() == Some(3),
            "{}",
            excluded.body
        );
        let items = excluded.body["items"].as_array().context("missing items")?;
        ensure!(items.iter().all(|item| item["target"] != "sqlx::query"));

        // target 与 exclude_target 互斥 → 400 INVALID_FILTER。
        let conflict = app
            .admin_get(
                "/api/admin/logs?target=sqlx::query&exclude_target=sqlx::query",
                &owner_cookie,
            )
            .await?;
        ensure!(
            conflict.status == StatusCode::BAD_REQUEST,
            "{}",
            conflict.body
        );
        ensure!(conflict.body["error"]["code"] == "INVALID_FILTER");

        // 上下文模式：以 "second"（WARN）为锚点，前后各 1 条。
        let anchor_id: i64 = items
            .iter()
            .find(|item| item["message"] == "second")
            .and_then(|item| item["id"].as_i64())
            .context("missing anchor id")?;
        let around = app
            .admin_get(
                &format!("/api/admin/logs?around_id={anchor_id}&context=1"),
                &owner_cookie,
            )
            .await?;
        ensure!(around.status == StatusCode::OK, "{}", around.body);
        let items = around.body["items"].as_array().context("missing items")?;
        let messages: Vec<&str> = items
            .iter()
            .filter_map(|item| item["message"].as_str())
            .collect();
        // context=1：前一条 → 锚点 → 后一条，按 ts asc。
        ensure!(
            messages == vec!["first", "second", "SELECT 2"],
            "{messages:?}"
        );
        ensure!(around.body["page"].as_u64() == Some(1));
        ensure!(around.body["total"].as_i64() == Some(items.len() as i64));

        // 锚点行存在性：不存在的 id → 404。
        let missing = app
            .admin_get("/api/admin/logs?around_id=999999", &owner_cookie)
            .await?;
        ensure!(missing.status == StatusCode::NOT_FOUND, "{}", missing.body);
        ensure!(missing.body["error"]["code"] == "LOG_ENTRY_NOT_FOUND");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn logs_api_stats_buckets_by_hour_and_day() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 当前时间附近造三个小时的桶数据 + 三天前的一条。
        sqlx::query(
            "INSERT INTO server_logs (ts, level, target, message) VALUES \
             (now() - interval '30 minutes', 'ERROR', 't', 'recent-err'), \
             (now() - interval '40 minutes', 'WARN', 't', 'recent-warn'), \
             (now() - interval '40 minutes', 'ERROR', 't', 'recent-err-2'), \
             (now() - interval '90 minutes', 'INFO', 't', 'older-info'), \
             (now() - interval '3 days', 'ERROR', 't', 'day-err')",
        )
        .execute(&app.state.database)
        .await?;

        // 按小时分桶（≤48h）。
        let stats = app
            .admin_get("/api/admin/logs/stats?hours=24", &owner_cookie)
            .await?;
        ensure!(stats.status == StatusCode::OK, "{}", stats.body);
        ensure!(stats.body["hours"].as_u64() == Some(24));
        // 丢弃计数字段恒在（Harness 为 noop 句柄，计数为 0）。
        ensure!(
            stats.body["channel_dropped"].as_u64() == Some(0),
            "{}",
            stats.body
        );
        let buckets = stats.body["buckets"]
            .as_array()
            .context("missing buckets")?;
        ensure!(!buckets.is_empty(), "expected at least one bucket");
        // 找桶：每桶五键恒在。
        for bucket in buckets {
            for key in ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] {
                ensure!(bucket[key].is_i64(), "missing key {key} in {bucket}");
            }
            ensure!(
                bucket["bucket"]
                    .as_str()
                    .context("bucket not string")?
                    .contains('T')
            );
        }
        // 合计：窗口内 ERROR 2（两个 recent-err 同小时桶或相邻桶）+ WARN 1 + INFO 1。
        let total_error: i64 = buckets.iter().filter_map(|b| b["ERROR"].as_i64()).sum();
        let total_warn: i64 = buckets.iter().filter_map(|b| b["WARN"].as_i64()).sum();
        let total_info: i64 = buckets.iter().filter_map(|b| b["INFO"].as_i64()).sum();
        ensure!(total_error == 2, "{buckets:?}");
        ensure!(total_warn == 1, "{buckets:?}");
        ensure!(total_info == 1, "{buckets:?}");
        // 桶按时间升序。
        let bucket_ts: Vec<&str> = buckets
            .iter()
            .filter_map(|b| b["bucket"].as_str())
            .collect();
        let mut sorted = bucket_ts.clone();
        sorted.sort();
        ensure!(bucket_ts == sorted);

        // 按天分桶（>48h）：三天前那条进入统计。
        let daily = app
            .admin_get("/api/admin/logs/stats?hours=96", &owner_cookie)
            .await?;
        ensure!(daily.status == StatusCode::OK, "{}", daily.body);
        let buckets = daily.body["buckets"]
            .as_array()
            .context("missing buckets")?;
        let total_error: i64 = buckets.iter().filter_map(|b| b["ERROR"].as_i64()).sum();
        ensure!(total_error == 3, "{buckets:?}");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn filter_override_is_runtime_switchable() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 初始无覆盖。
        let initial = app
            .admin_get("/api/admin/logs/filter", &owner_cookie)
            .await?;
        ensure!(initial.status == StatusCode::OK, "{}", initial.body);
        ensure!(initial.body["directives"] == "");
        ensure!(initial.body["restore_seconds_remaining"].is_null());

        // 设置覆盖（无自动复位）。
        let set = app
            .admin_put(
                "/api/admin/logs/filter",
                serde_json::json!({ "directives": "aries_server=debug" }),
                &owner_cookie,
            )
            .await?;
        ensure!(set.status == StatusCode::OK, "{}", set.body);
        ensure!(set.body["directives"] == "aries_server=debug");
        ensure!(set.body["restore_seconds_remaining"].is_null());
        ensure!(app.state.log_handle.custom_directives() == "aries_server=debug");

        // 带自动复位的设置回读剩余秒数。
        let timed = app
            .admin_put(
                "/api/admin/logs/filter",
                serde_json::json!({ "directives": "tower_http=debug", "restore_minutes": 30 }),
                &owner_cookie,
            )
            .await?;
        ensure!(timed.status == StatusCode::OK, "{}", timed.body);
        let remaining = timed.body["restore_seconds_remaining"].as_u64();
        ensure!(
            remaining.is_some_and(|value| (1500..=1800).contains(&value)),
            "unexpected restore_seconds_remaining: {remaining:?}"
        );

        // 非法 directive → 400 INVALID_DIRECTIVES，状态不被破坏。
        let invalid = app
            .admin_put(
                "/api/admin/logs/filter",
                serde_json::json!({ "directives": "==nonsense" }),
                &owner_cookie,
            )
            .await?;
        ensure!(invalid.status == StatusCode::BAD_REQUEST, "{}", invalid.body);
        ensure!(invalid.body["error"]["code"] == "INVALID_DIRECTIVES");
        ensure!(app.state.log_handle.custom_directives() == "tower_http=debug");

        // 空 directives 清除覆盖（含未到期自动复位一并失效）。
        let cleared = app
            .admin_put(
                "/api/admin/logs/filter",
                serde_json::json!({ "directives": "" }),
                &owner_cookie,
            )
            .await?;
        ensure!(cleared.status == StatusCode::OK, "{}", cleared.body);
        ensure!(cleared.body["directives"] == "");
        ensure!(cleared.body["restore_seconds_remaining"].is_null());
        ensure!(app.state.log_handle.custom_directives().is_empty());

        // 权限：editor 403。
        let password_hash = app.state.passwords.hash("editor-filter-1")?;
        sqlx::query(
            "INSERT INTO users (username, email, password_hash, display_name, role, status) \
             VALUES ('editor-filter', 'editor-filter@example.com', $1, 'Editor', 'editor', 'active')",
        )
        .bind(&password_hash)
        .execute(&app.state.database)
        .await?;
        let login = app
            .admin_post(
                "/api/admin/auth/login",
                serde_json::json!({ "login": "editor-filter", "password": "editor-filter-1" }),
                None,
            )
            .await?;
        let editor_cookie = login
            .set_cookie
            .context("login did not set cookie")?
            .split(';')
            .next()
            .context("failed to parse session cookie")?
            .to_owned();
        let denied = app
            .admin_get("/api/admin/logs/filter", &editor_cookie)
            .await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn tail_streams_logs_inserted_after_connect_over_sse() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 鉴权与参数校验先于流建立：401 / 400 均为普通 JSON 响应。
        let unauthenticated = app.get("/api/admin/logs/tail").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        let conflict = app
            .admin_get(
                "/api/admin/logs/tail?target=sqlx::query&exclude_target=sqlx::query",
                &owner_cookie,
            )
            .await?;
        ensure!(
            conflict.status == StatusCode::BAD_REQUEST,
            "{}",
            conflict.body
        );
        ensure!(conflict.body["error"]["code"] == "INVALID_FILTER");

        // 建立流式连接（Body 不收集，由调用方逐帧读取）。
        let (status, headers, mut body) = app
            .admin_get_streaming("/api/admin/logs/tail", &owner_cookie)
            .await?;
        ensure!(status == StatusCode::OK, "{status}");
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        ensure!(
            content_type.starts_with("text/event-stream"),
            "tail response is not SSE: {content_type}"
        );

        // 等 anchor 查询完成（无直接观测点，留出裕量），随后插入连接后的新日志。
        tokio::time::sleep(Duration::from_millis(300)).await;
        insert_log(
            &app,
            "now",
            "WARN",
            "tail_test_target",
            "tail-probe-1",
            None,
        )
        .await?;

        // 逐帧读取，直到收到包含探针消息的 SSE 数据。
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut buffered = String::new();
        let mut found = false;
        while Instant::now() < deadline && !found {
            let frame = tokio::time::timeout(Duration::from_secs(5), body.frame()).await;
            let Ok(Some(Ok(frame))) = frame else {
                break;
            };
            if let Some(data) = frame.data_ref() {
                buffered.push_str(&String::from_utf8_lossy(data));
                if buffered.contains("tail-probe-1") {
                    found = true;
                }
            }
        }
        ensure!(
            found,
            "tail did not deliver the inserted log; buffered so far: {buffered}"
        );
        // 探针日志以 event: log 的数据帧推送。
        ensure!(buffered.contains("event: log"), "{buffered}");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn db_log_writer_redacts_sensitive_fields_before_persist() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        use tracing_subscriber::layer::SubscriberExt;

        // 用独立 Dispatch 驱动 DbLogLayer：敏感字段、query string、密码哈希形态的值
        // 应在落库前被替换，连接前日志不会出现在 tail 之外的常规查询里不影响此处断言。
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let dispatch = tracing::Dispatch::new(
            tracing_subscriber::registry().with(aries_server::log_store::DbLogLayer::new(tx)),
        );
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(
                password = "hunter2",
                http.query = "page=1&access_token=sekrit",
                note = "hash is $argon2id$v=19$m=64,t=4,p=8$fakefakefake",
                object_key = "2026/09/keep-me.png",
                "redaction roundtrip"
            );
        });
        drop(dispatch);

        let handle = aries_server::log_store::spawn_writer(app.state.database.clone(), rx, 14);
        handle.await.context("writer task failed")?;

        let (fields,): (serde_json::Value,) =
            sqlx::query_as("SELECT fields FROM server_logs WHERE message = 'redaction roundtrip'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(fields["password"] == "***", "{fields}");
        ensure!(
            fields["http.query"] == "page=1&access_token=***",
            "{fields}"
        );
        ensure!(fields["note"] == "***", "{fields}");
        ensure!(fields["object_key"] == "2026/09/keep-me.png", "{fields}");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
