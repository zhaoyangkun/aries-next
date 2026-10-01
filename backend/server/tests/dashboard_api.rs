//! Dashboard Contract Test：Admin 聚合端点（文章/评论统计 + 最近待审核评论 + 最近失败 Job）+ 权限。
//! 数据全部直接 SQL 插入固定 Fixture，先造数再断言计数字段。
//! 通过 `ARIES_RUN_DATABASE_TESTS=1` 门控，随机 Schema 隔离。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

/// 直接 SQL 插入一篇文章（published 状态按表约束补 published_at）。
async fn insert_article(
    app: &TestApp,
    author_id: i64,
    slug: &str,
    status: &str,
) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO articles (author_id, status, slug, title, markdown_source, rendered_html, published_at) \
         VALUES ($1, $2, $3, $4, '# md', '<p>md</p>', CASE WHEN $2 = 'published' THEN now() ELSE NULL END) \
         RETURNING id",
    )
    .bind(author_id)
    .bind(status)
    .bind(slug)
    .bind(format!("title-{slug}"))
    .fetch_one(&app.state.database)
    .await
    .context("insert article")?;
    Ok(id)
}

/// 直接 SQL 插入一条评论（created_at 可控，便于 today 统计断言）。
async fn insert_comment(
    app: &TestApp,
    article_id: i64,
    author_name: &str,
    status: &str,
    created_at: time::OffsetDateTime,
    deleted: bool,
) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO comments (target_type, target_id, author_name, author_email, content_markdown, status, created_at, deleted_at) \
         VALUES ('article', $1, $2, $3, $4, $5, $6, CASE WHEN $7 THEN now() ELSE NULL END) \
         RETURNING id",
    )
    .bind(article_id)
    .bind(author_name)
    .bind(format!("{author_name}@example.com"))
    .bind(format!("content-{author_name}"))
    .bind(status)
    .bind(created_at)
    .bind(deleted)
    .fetch_one(&app.state.database)
    .await
    .context("insert comment")?;
    Ok(id)
}

/// 直接 SQL 插入一条后台任务（updated_at 可控，便于失败列表排序断言）。
async fn insert_job(
    app: &TestApp,
    kind: &str,
    status: &str,
    attempts: i32,
    last_error: Option<&str>,
    updated_at: time::OffsetDateTime,
) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO background_jobs (kind, status, attempts, max_attempts, last_error, updated_at) \
         VALUES ($1, $2, $3, 3, $4, $5) \
         RETURNING id",
    )
    .bind(kind)
    .bind(status)
    .bind(attempts)
    .bind(last_error)
    .bind(updated_at)
    .fetch_one(&app.state.database)
    .await
    .context("insert background job")?;
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
async fn dashboard_empty_site_returns_zero_stats() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        let dashboard = app.admin_get("/api/admin/dashboard", &owner_cookie).await?;
        ensure!(dashboard.status == StatusCode::OK, "{}", dashboard.body);
        // 四个顶级字段齐全，计数全为 0，列表为空。
        ensure!(dashboard.body["articles"]["total"].as_i64() == Some(0));
        ensure!(dashboard.body["articles"]["draft"].as_i64() == Some(0));
        ensure!(dashboard.body["articles"]["published"].as_i64() == Some(0));
        ensure!(dashboard.body["articles"]["recycled"].as_i64() == Some(0));
        ensure!(dashboard.body["comments"]["total"].as_i64() == Some(0));
        ensure!(dashboard.body["comments"]["pending"].as_i64() == Some(0));
        ensure!(dashboard.body["comments"]["today"].as_i64() == Some(0));
        ensure!(
            dashboard.body["recent_pending_comments"]
                .as_array()
                .map(Vec::is_empty)
                == Some(true)
        );
        ensure!(
            dashboard.body["recent_failed_jobs"]
                .as_array()
                .map(Vec::is_empty)
                == Some(true)
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn dashboard_aggregates_article_comment_and_job_stats() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let (owner_id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE username = 'owner'")
            .fetch_one(&app.state.database)
            .await?;

        // 文章：draft 2 + published 3 + recycled 1，合计 6。
        let target = insert_article(&app, owner_id, "dash-draft-1", "draft").await?;
        insert_article(&app, owner_id, "dash-draft-2", "draft").await?;
        for slug in ["dash-pub-1", "dash-pub-2", "dash-pub-3"] {
            insert_article(&app, owner_id, slug, "published").await?;
        }
        insert_article(&app, owner_id, "dash-recycled-1", "recycled").await?;

        // 评论：pending 2（一条今天一条昨天）+ approved 1（今天）+ 已删除 pending 1。
        // 期望：total 3（不含已删除）、pending 2（不含已删除）、today 2（今天创建且未删除）。
        let now = time::OffsetDateTime::now_utc();
        insert_comment(&app, target, "alice", "pending", now, false).await?;
        insert_comment(
            &app,
            target,
            "bob",
            "pending",
            now - time::Duration::days(1),
            false,
        )
        .await?;
        insert_comment(&app, target, "carol", "approved", now, false).await?;
        insert_comment(&app, target, "dave", "pending", now, true).await?;

        // 后台任务：failed 2（更新时间一旧一新）+ done 1（不应出现在失败列表）。
        insert_job(
            &app,
            "media_cleanup",
            "failed",
            3,
            Some("disk full"),
            now - time::Duration::hours(1),
        )
        .await?;
        insert_job(&app, "comment_notification", "failed", 1, None, now).await?;
        insert_job(&app, "media_cleanup", "done", 1, None, now).await?;

        let dashboard = app.admin_get("/api/admin/dashboard", &owner_cookie).await?;
        ensure!(dashboard.status == StatusCode::OK, "{}", dashboard.body);

        // 文章聚合计数。
        let articles = &dashboard.body["articles"];
        ensure!(articles["total"].as_i64() == Some(6), "{articles}");
        ensure!(articles["draft"].as_i64() == Some(2), "{articles}");
        ensure!(articles["published"].as_i64() == Some(3), "{articles}");
        ensure!(articles["recycled"].as_i64() == Some(1), "{articles}");

        // 评论聚合计数。
        let comments = &dashboard.body["comments"];
        ensure!(comments["total"].as_i64() == Some(3), "{comments}");
        ensure!(comments["pending"].as_i64() == Some(2), "{comments}");
        ensure!(comments["today"].as_i64() == Some(2), "{comments}");

        // 最近待审核评论：按 created_at desc，今天的 alice 在前；不含已删除的 dave。
        let pending = dashboard.body["recent_pending_comments"]
            .as_array()
            .context("missing recent_pending_comments")?;
        ensure!(pending.len() == 2, "{pending:?}");
        ensure!(pending[0]["author_name"] == "alice");
        ensure!(pending[0]["status"] == "pending");
        ensure!(pending[0]["target_type"] == "article");
        ensure!(pending[0]["target_id"].as_i64() == Some(target));
        ensure!(pending[1]["author_name"] == "bob");
        // 隐私字段不下发。
        ensure!(pending[0].get("ip_hash").is_none());
        ensure!(pending[0].get("user_agent_digest").is_none());

        // 最近失败任务：按 updated_at desc，最新的在前；字段齐全。
        let failed = dashboard.body["recent_failed_jobs"]
            .as_array()
            .context("missing recent_failed_jobs")?;
        ensure!(failed.len() == 2, "{failed:?}");
        ensure!(failed[0]["kind"] == "comment_notification");
        ensure!(failed[0]["attempts"].as_i64() == Some(1));
        ensure!(failed[0]["max_attempts"].as_i64() == Some(3));
        ensure!(failed[0]["last_error"].is_null());
        ensure!(failed[1]["kind"] == "media_cleanup");
        ensure!(failed[1]["attempts"].as_i64() == Some(3));
        ensure!(failed[1]["last_error"] == "disk full");
        // updated_at 为 time 默认 serde 数组形式 [year, ordinal, hour, …]。
        let updated_at = failed[0]["updated_at"]
            .as_array()
            .context("updated_at is not an array")?;
        ensure!(updated_at[0].as_i64() >= Some(2000), "{failed:?}");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn dashboard_requires_authentication_and_is_open_to_all_roles() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        app.bootstrap_owner().await?;

        // 未认证 401。
        let unauthenticated = app.get("/api/admin/dashboard").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);

        // 403 不可达：core 的 Role::allows 把 ViewDashboard 授予全部三个角色
        // （owner/editor/moderator），已认证用户不会被该端点拒绝。
        let editor_cookie =
            create_user_and_login(&app, "editor-dash", "editor", "editor-pass-1").await?;
        let editor_view = app
            .admin_get("/api/admin/dashboard", &editor_cookie)
            .await?;
        ensure!(editor_view.status == StatusCode::OK, "{}", editor_view.body);
        let moderator_cookie =
            create_user_and_login(&app, "moderator-dash", "moderator", "moderator-pass-1").await?;
        let moderator_view = app
            .admin_get("/api/admin/dashboard", &moderator_cookie)
            .await?;
        ensure!(
            moderator_view.status == StatusCode::OK,
            "{}",
            moderator_view.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
