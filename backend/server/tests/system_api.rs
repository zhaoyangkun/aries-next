//! System Metrics Contract Test：系统监控端点（快照 / SSE 实时流 / 历史趋势）+ 权限。
//! 通过 `ARIES_RUN_DATABASE_TESTS=1` 门控，随机 Schema 隔离。

mod common;

use anyhow::{Context, ensure};
use axum::http::{StatusCode, header};
use http_body_util::BodyExt;

use common::TestApp;

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
async fn metrics_snapshot_contains_host_process_and_pg_metrics() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        let response = app
            .admin_get("/api/admin/system/metrics", &owner_cookie)
            .await?;
        ensure!(response.status == StatusCode::OK, "{}", response.body);
        let body = &response.body;

        // 时间戳为 RFC 3339 字符串。
        ensure!(
            body["ts"]
                .as_str()
                .is_some_and(|ts| ts.contains('T') && ts.ends_with('Z')),
            "{}",
            body["ts"]
        );

        // CPU：全局与每核使用率均在 0–100；容器与桌面核数不同，只断非空。
        let cpu_percent = body["cpu"]["percent"].as_f64().context("cpu.percent")?;
        ensure!(
            (0.0..=100.0).contains(&cpu_percent),
            "cpu percent out of range: {cpu_percent}"
        );
        let per_core = body["cpu"]["per_core"].as_array().context("cpu.per_core")?;
        ensure!(!per_core.is_empty(), "per_core must not be empty");
        ensure!(
            per_core
                .iter()
                .all(|value| value.as_f64().is_some_and(|v| (0.0..=100.0).contains(&v))),
            "{per_core:?}"
        );

        // 内存：总量为正，使用率与字节数自洽（percent 由服务端预计算）。
        let memory_total = body["memory"]["total_bytes"]
            .as_u64()
            .context("memory.total")?;
        ensure!(memory_total > 0, "memory total must be positive");
        let memory_used = body["memory"]["used_bytes"]
            .as_u64()
            .context("memory.used")?;
        ensure!(memory_used <= memory_total, "memory used > total");
        let memory_percent = body["memory"]["percent"]
            .as_f64()
            .context("memory.percent")?;
        ensure!(
            (0.0..=100.0).contains(&memory_percent),
            "memory percent out of range: {memory_percent}"
        );

        // Swap：字段恒在（未配置 Swap 时 total 为 0）。
        ensure!(body["swap"]["total_bytes"].is_u64());
        ensure!(body["swap"]["used_bytes"].is_u64());

        // 磁盘：挂载点非空，used ≤ total。
        let disks = body["disks"].as_array().context("disks")?;
        ensure!(!disks.is_empty(), "disks must not be empty");
        for disk in disks {
            ensure!(disk["mount_point"].as_str().is_some_and(|m| !m.is_empty()));
            ensure!(disk["total_bytes"].as_u64().unwrap_or(0) > 0);
            ensure!(
                disk["used_bytes"].as_u64().unwrap_or(1)
                    <= disk["total_bytes"].as_u64().unwrap_or(0),
                "{disk}"
            );
        }

        // 网络与负载：字段恒在（速率在无流量时可为 0）。
        ensure!(body["network"]["rx_bytes_per_sec"].is_u64());
        ensure!(body["network"]["tx_bytes_per_sec"].is_u64());
        ensure!(body["network"]["total_received_bytes"].is_u64());
        ensure!(body["network"]["total_transmitted_bytes"].is_u64());
        ensure!(body["load"]["one"].is_f64());
        ensure!(body["load"]["five"].is_f64());
        ensure!(body["load"]["fifteen"].is_f64());

        // 进程数与主机信息。
        ensure!(body["process_count"].as_u64().unwrap_or(0) > 0);
        ensure!(body["uptime_seconds"].as_u64().unwrap_or(0) > 0);
        ensure!(body["host_name"].is_string());

        // Server 自身：RSS 为正（进程存活）。
        let rss = body["server"]["rss_bytes"].as_u64().context("server.rss")?;
        ensure!(rss > 0, "server rss must be positive");

        // PostgreSQL：测试库真实存在，库大小与连接数必须可用。
        let pg_size = body["pg"]["database_size_bytes"]
            .as_i64()
            .context("pg.database_size_bytes")?;
        ensure!(pg_size > 0, "pg database size must be positive");
        let connections = body["pg"]["active_connections"]
            .as_i64()
            .context("pg.active_connections")?;
        ensure!(connections >= 1, "at least the querying connection");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn metrics_history_returns_samples_and_clamps_window() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // Harness 不跑采样循环：直接推一个当前采样点，验证查询链路。
        let snapshot = app.state.metrics.collect().await;
        app.state
            .metrics_history
            .push(aries_server::metrics::sample_point(&snapshot));

        let response = app
            .admin_get("/api/admin/system/metrics/history", &owner_cookie)
            .await?;
        ensure!(response.status == StatusCode::OK, "{}", response.body);
        ensure!(response.body["hours"].as_u64() == Some(1));
        let buckets = response.body["buckets"].as_array().context("buckets")?;
        ensure!(buckets.len() == 1, "{buckets:?}");
        let bucket = &buckets[0];
        ensure!(
            bucket["bucket"]
                .as_str()
                .is_some_and(|ts| ts.contains('T') && ts.ends_with('Z')),
            "{}",
            bucket["bucket"]
        );
        ensure!(bucket["memory_total_bytes"].as_u64().unwrap_or(0) > 0);
        ensure!(bucket["cpu_percent"].as_f64().is_some());

        // hours 越界钳位到 24（与运行日志 stats 端点惯例一致）。
        let clamped = app
            .admin_get("/api/admin/system/metrics/history?hours=99", &owner_cookie)
            .await?;
        ensure!(clamped.status == StatusCode::OK, "{}", clamped.body);
        ensure!(clamped.body["hours"].as_u64() == Some(24));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn metrics_endpoints_require_authentication_and_owner() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        app.bootstrap_owner().await?;

        // 未认证 401。
        for path in [
            "/api/admin/system/metrics",
            "/api/admin/system/metrics/history",
        ] {
            let unauthenticated = app.get(path).await?;
            ensure!(
                unauthenticated.status == StatusCode::UNAUTHORIZED,
                "{path}: {}",
                unauthenticated.status
            );
        }

        // Editor 无 settings:manage 权限，403。
        let editor_cookie =
            create_user_and_login(&app, "editor-metrics", "editor", "editor-pass-1").await?;
        let denied = app
            .admin_get("/api/admin/system/metrics", &editor_cookie)
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
async fn metrics_stream_pushes_snapshot_events_via_sse() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 建立流式连接（Body 不收集，由调用方逐帧读取）。
        let (status, headers, mut body) = app
            .admin_get_streaming("/api/admin/system/metrics/stream", &owner_cookie)
            .await?;
        ensure!(status == StatusCode::OK, "{status}");
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        ensure!(
            content_type.starts_with("text/event-stream"),
            "stream response is not SSE: {content_type}"
        );

        // 首个 tick 立即触发：第一帧应很快到达。
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), body.frame()).await;
        let Ok(Some(Ok(frame))) = frame else {
            anyhow::bail!("stream did not deliver the first metrics frame");
        };
        let data = frame
            .data_ref()
            .map(|data| String::from_utf8_lossy(data).into_owned())
            .unwrap_or_default();
        ensure!(data.contains("event: metrics"), "{data}");
        ensure!(data.contains("\"cpu\""), "{data}");
        ensure!(data.contains("\"memory\""), "{data}");
        // 主动 drop body：SSE 长连接随测试结束关闭。
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
