//! Phase 06 Contract Test：日志（短内容）Admin CRUD + Public 可见性边界。
//! Private 日志不得出现在 Public 端点（Phase 06 §5）。

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

/// 便捷创建一条日志并返回 id，失败直接报错。
async fn create_journal(app: &TestApp, cookie: &str, markdown: &str) -> anyhow::Result<i64> {
    let response = app
        .admin_post(
            "/api/admin/journals",
            serde_json::json!({ "content_markdown": markdown }),
            Some(cookie),
        )
        .await?;
    ensure!(response.status == StatusCode::CREATED, "{}", response.body);
    response.body["id"].as_i64().context("missing journal id")
}

/// journal 目标的审计事件数，用于验证失败请求不落审计。
async fn journal_audit_count(app: &TestApp) -> anyhow::Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE target_type = 'journal'")
            .fetch_one(&app.state.database)
            .await?;
    Ok(count)
}

#[tokio::test]
async fn journal_crud_and_visibility_boundary() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 一公一私两条日志。
        let public = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "A **public** note" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(public.status == StatusCode::CREATED, "{}", public.body);
        ensure!(public.body["visibility"] == "public");
        ensure!(
            public.body["content_html"]
                .as_str()
                .context("missing html")?
                .contains("<strong>public</strong>")
        );
        let public_id = public.body["id"].as_i64().context("missing id")?;

        let private = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "a private note", "visibility": "private" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(private.status == StatusCode::CREATED, "{}", private.body);

        // 空内容 → 400。
        let empty = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "   " }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(empty.status == StatusCode::BAD_REQUEST, "{}", empty.body);
        ensure!(empty.body["error"]["code"] == "INVALID_JOURNAL_CONTENT");

        // 非法 visibility → 400。
        let bad_visibility = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "x", "visibility": "hidden" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(bad_visibility.status == StatusCode::BAD_REQUEST);

        // Admin 列表：visibility 过滤 + 倒序。
        let all = app.admin_get("/api/admin/journals", &owner_cookie).await?;
        ensure!(all.status == StatusCode::OK, "{}", all.body);
        ensure!(all.body["total"].as_i64() == Some(2));
        let privates = app
            .admin_get("/api/admin/journals?visibility=private", &owner_cookie)
            .await?;
        ensure!(privates.body["total"].as_i64() == Some(1));

        // Public 端点（无 Cookie）：只见 public 那条。
        let public_list = app.get("/api/public/journals").await?;
        ensure!(public_list.status == StatusCode::OK, "{}", public_list.body);
        ensure!(public_list.body["total"].as_i64() == Some(1));
        ensure!(public_list.body["items"][0]["id"].as_i64() == Some(public_id));

        // 更新为 private 后 Public 列表归零。
        let updated = app
            .admin_put(
                &format!("/api/admin/journals/{public_id}"),
                serde_json::json!({ "content_markdown": "now private", "visibility": "private" }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        let public_after = app.get("/api/public/journals").await?;
        ensure!(public_after.body["total"].as_i64() == Some(0));

        // 软删除 → 404。
        let deleted = app
            .admin_delete(&format!("/api/admin/journals/{public_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let gone = app
            .admin_get(&format!("/api/admin/journals/{public_id}"), &owner_cookie)
            .await?;
        ensure!(gone.status == StatusCode::NOT_FOUND);

        // 审计：created ×2 + updated + deleted。
        let audit_count = journal_audit_count(&app).await?;
        ensure!(audit_count == 4, "expected 4 journal audit events, got {audit_count}");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn journal_permission_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-j", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-j", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-j", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-j", "moderator-pass-1").await?;

        // editor 拥有 ManageContent，可创建日志。
        let created = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "editor note" }),
                Some(&editor_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        let journal_id = created.body["id"].as_i64().context("missing id")?;

        // moderator 无 ManageContent：列表/详情/创建/更新/删除全部 403。
        let denied = app
            .admin_get("/api/admin/journals", &moderator_cookie)
            .await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);
        ensure!(denied.body["error"]["code"] == "PERMISSION_DENIED");

        let denied_get = app
            .admin_get(
                &format!("/api/admin/journals/{journal_id}"),
                &moderator_cookie,
            )
            .await?;
        ensure!(denied_get.status == StatusCode::FORBIDDEN);

        let denied_create = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "moderator note" }),
                Some(&moderator_cookie),
            )
            .await?;
        ensure!(denied_create.status == StatusCode::FORBIDDEN);

        let denied_update = app
            .admin_put(
                &format!("/api/admin/journals/{journal_id}"),
                serde_json::json!({ "content_markdown": "moderator edit" }),
                &moderator_cookie,
            )
            .await?;
        ensure!(denied_update.status == StatusCode::FORBIDDEN);

        let denied_delete = app
            .admin_delete(
                &format!("/api/admin/journals/{journal_id}"),
                &moderator_cookie,
            )
            .await?;
        ensure!(denied_delete.status == StatusCode::FORBIDDEN);

        // 未认证 → 401：GET 无 Cookie；写请求带合法 Origin 但无 Cookie。
        let unauthenticated = app.get("/api/admin/journals").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        let unauthenticated_create = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "anon note" }),
                None,
            )
            .await?;
        ensure!(
            unauthenticated_create.status == StatusCode::UNAUTHORIZED,
            "{}",
            unauthenticated_create.body
        );
        // 伪造 Session Token → 401。
        let bogus = "aries_admin_session=not-a-real-token";
        let bogus_update = app
            .admin_put(
                &format!("/api/admin/journals/{journal_id}"),
                serde_json::json!({ "content_markdown": "bogus edit" }),
                bogus,
            )
            .await?;
        ensure!(bogus_update.status == StatusCode::UNAUTHORIZED);
        let bogus_delete = app
            .admin_delete(&format!("/api/admin/journals/{journal_id}"), bogus)
            .await?;
        ensure!(bogus_delete.status == StatusCode::UNAUTHORIZED);

        // 403/401 均未写入数据，也不产生 journal 审计。
        ensure!(journal_audit_count(&app).await? == 1);
        let list = app.admin_get("/api/admin/journals", &owner_cookie).await?;
        ensure!(list.body["total"].as_i64() == Some(1));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn journal_validation_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let journal_id = create_journal(&app, &owner_cookie, "valid note").await?;

        // 长度边界：2000 字符通过，2001 字符 → 400。
        let max_ok = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "a".repeat(2000) }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(max_ok.status == StatusCode::CREATED, "{}", max_ok.body);

        let too_long = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "a".repeat(2001) }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            too_long.status == StatusCode::BAD_REQUEST,
            "{}",
            too_long.body
        );
        ensure!(too_long.body["error"]["code"] == "INVALID_JOURNAL_CONTENT");

        // 非法 visibility → 400，错误码精确匹配。
        let bad_visibility = app
            .admin_post(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "x", "visibility": "hidden" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(bad_visibility.status == StatusCode::BAD_REQUEST);
        ensure!(bad_visibility.body["error"]["code"] == "INVALID_JOURNAL_VISIBILITY");

        // 更新路径同样校验：空内容与超长内容 → 400。
        let empty_update = app
            .admin_put(
                &format!("/api/admin/journals/{journal_id}"),
                serde_json::json!({ "content_markdown": "  " }),
                &owner_cookie,
            )
            .await?;
        ensure!(empty_update.status == StatusCode::BAD_REQUEST);
        ensure!(empty_update.body["error"]["code"] == "INVALID_JOURNAL_CONTENT");

        let long_update = app
            .admin_put(
                &format!("/api/admin/journals/{journal_id}"),
                serde_json::json!({ "content_markdown": "b".repeat(2001) }),
                &owner_cookie,
            )
            .await?;
        ensure!(long_update.status == StatusCode::BAD_REQUEST);

        let bad_visibility_update = app
            .admin_put(
                &format!("/api/admin/journals/{journal_id}"),
                serde_json::json!({ "content_markdown": "ok", "visibility": "archived" }),
                &owner_cookie,
            )
            .await?;
        ensure!(bad_visibility_update.status == StatusCode::BAD_REQUEST);
        ensure!(bad_visibility_update.body["error"]["code"] == "INVALID_JOURNAL_VISIBILITY");

        // PUT 为全量更新：省略 visibility 时回落到 public（与 Handler 默认值一致）。
        let reset_visibility = app
            .admin_put(
                &format!("/api/admin/journals/{journal_id}"),
                serde_json::json!({ "content_markdown": "no visibility field" }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            reset_visibility.status == StatusCode::OK,
            "{}",
            reset_visibility.body
        );
        ensure!(reset_visibility.body["visibility"] == "public");

        // 400 请求不落数据、不写审计：总数仍是 2 条，审计仅 created ×2 + updated ×1。
        let list = app.admin_get("/api/admin/journals", &owner_cookie).await?;
        ensure!(list.body["total"].as_i64() == Some(2));
        ensure!(journal_audit_count(&app).await? == 3);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn journal_not_found_paths() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let journal_id = create_journal(&app, &owner_cookie, "about to vanish").await?;
        let missing_id = journal_id + 1000;

        // 详情 / 更新 / 删除不存在的 id → 404 JOURNAL_NOT_FOUND。
        let get_missing = app
            .admin_get(&format!("/api/admin/journals/{missing_id}"), &owner_cookie)
            .await?;
        ensure!(
            get_missing.status == StatusCode::NOT_FOUND,
            "{}",
            get_missing.body
        );
        ensure!(get_missing.body["error"]["code"] == "JOURNAL_NOT_FOUND");

        let update_missing = app
            .admin_put(
                &format!("/api/admin/journals/{missing_id}"),
                serde_json::json!({ "content_markdown": "ghost" }),
                &owner_cookie,
            )
            .await?;
        ensure!(update_missing.status == StatusCode::NOT_FOUND);
        ensure!(update_missing.body["error"]["code"] == "JOURNAL_NOT_FOUND");

        let delete_missing = app
            .admin_delete(&format!("/api/admin/journals/{missing_id}"), &owner_cookie)
            .await?;
        ensure!(delete_missing.status == StatusCode::NOT_FOUND);
        ensure!(delete_missing.body["error"]["code"] == "JOURNAL_NOT_FOUND");

        // 软删除是幂等边界：二次删除同样 404，且不再产生审计。
        let deleted = app
            .admin_delete(&format!("/api/admin/journals/{journal_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let deleted_again = app
            .admin_delete(&format!("/api/admin/journals/{journal_id}"), &owner_cookie)
            .await?;
        ensure!(deleted_again.status == StatusCode::NOT_FOUND);
        // 已软删除的记录更新也返回 404。
        let update_deleted = app
            .admin_put(
                &format!("/api/admin/journals/{journal_id}"),
                serde_json::json!({ "content_markdown": "revive" }),
                &owner_cookie,
            )
            .await?;
        ensure!(update_deleted.status == StatusCode::NOT_FOUND);

        // 审计仅 created + deleted，404 路径不写审计。
        ensure!(journal_audit_count(&app).await? == 2);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn journal_list_pagination_and_filters() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let first_id = create_journal(&app, &owner_cookie, "note one").await?;
        create_journal(&app, &owner_cookie, "note two").await?;
        let third_id = create_journal(&app, &owner_cookie, "note three").await?;
        app.admin_post(
            "/api/admin/journals",
            serde_json::json!({ "content_markdown": "secret note", "visibility": "private" }),
            Some(&owner_cookie),
        )
        .await?;

        // 第一页 page_size=2：total=4，按创建时间倒序（新记录在前）。
        let page_one = app
            .admin_get("/api/admin/journals?page=1&page_size=2", &owner_cookie)
            .await?;
        ensure!(page_one.status == StatusCode::OK, "{}", page_one.body);
        ensure!(page_one.body["total"].as_i64() == Some(4));
        ensure!(page_one.body["page"].as_u64() == Some(1));
        ensure!(page_one.body["page_size"].as_u64() == Some(2));
        let items = page_one.body["items"].as_array().context("missing items")?;
        ensure!(items.len() == 2);
        ensure!(items[0]["id"].as_i64() > items[1]["id"].as_i64());
        ensure!(items[0]["id"].as_i64() > Some(third_id));

        // 第二页：剩余 2 条。
        let page_two = app
            .admin_get("/api/admin/journals?page=2&page_size=2", &owner_cookie)
            .await?;
        ensure!(page_two.status == StatusCode::OK, "{}", page_two.body);
        let items_two = page_two.body["items"].as_array().context("missing items")?;
        ensure!(items_two.len() == 2);
        ensure!(items_two[1]["id"].as_i64() == Some(first_id));

        // page_size 超过上限时钳制到 100。
        let clamped = app
            .admin_get("/api/admin/journals?page_size=1000", &owner_cookie)
            .await?;
        ensure!(clamped.status == StatusCode::OK, "{}", clamped.body);
        ensure!(clamped.body["page_size"].as_u64() == Some(100));
        ensure!(
            clamped.body["items"]
                .as_array()
                .context("missing items")?
                .len()
                == 4
        );

        // visibility 过滤；非法过滤值 → 400。
        let public_only = app
            .admin_get("/api/admin/journals?visibility=public", &owner_cookie)
            .await?;
        ensure!(public_only.body["total"].as_i64() == Some(3));
        let bad_filter = app
            .admin_get("/api/admin/journals?visibility=hidden", &owner_cookie)
            .await?;
        ensure!(
            bad_filter.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_filter.body
        );
        ensure!(bad_filter.body["error"]["code"] == "INVALID_JOURNAL_VISIBILITY");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn journal_origin_guard_on_writes() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 写请求 Origin 不在白名单 → 403 INVALID_ORIGIN（先于认证与权限）。
        let bad_origin = app
            .post_with_headers(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "csrf attempt" }),
                &[
                    ("Origin", "http://evil.example"),
                    ("Cookie", owner_cookie.as_str()),
                ],
            )
            .await?;
        ensure!(
            bad_origin.status == StatusCode::FORBIDDEN,
            "{}",
            bad_origin.body
        );
        ensure!(bad_origin.body["error"]["code"] == "INVALID_ORIGIN");

        // 缺少 Origin 的写请求同样被拒。
        let no_origin = app
            .post_with_headers(
                "/api/admin/journals",
                serde_json::json!({ "content_markdown": "no origin" }),
                &[("Cookie", owner_cookie.as_str())],
            )
            .await?;
        ensure!(no_origin.status == StatusCode::FORBIDDEN);
        ensure!(no_origin.body["error"]["code"] == "INVALID_ORIGIN");

        // 被 Origin 拦截的请求不落数据。
        let list = app.admin_get("/api/admin/journals", &owner_cookie).await?;
        ensure!(list.body["total"].as_i64() == Some(0));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn public_journal_list_dto_and_pagination() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let first_id = create_journal(&app, &owner_cookie, "public **one**").await?;
        create_journal(&app, &owner_cookie, "public two").await?;
        create_journal(&app, &owner_cookie, "public three").await?;
        app.admin_post(
            "/api/admin/journals",
            serde_json::json!({ "content_markdown": "private note", "visibility": "private" }),
            Some(&owner_cookie),
        )
        .await?;

        // Public 列表无需 Cookie，且 Private 不计入 total。
        let list = app.get("/api/public/journals").await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body["total"].as_i64() == Some(3));

        // 公开 DTO 只暴露 id / content_html / created_at，不回 Markdown 源与可见性。
        let item = &list.body["items"][0];
        ensure!(item.get("id").is_some());
        ensure!(item.get("content_html").is_some());
        ensure!(item.get("created_at").is_some());
        ensure!(item.get("content_markdown").is_none());
        ensure!(item.get("visibility").is_none());
        ensure!(item.get("created_by").is_none());

        // 分页语义与 Admin 一致：page_size=2 时第二页只剩 1 条（最早的记录）。
        let page_two = app.get("/api/public/journals?page=2&page_size=2").await?;
        ensure!(page_two.status == StatusCode::OK, "{}", page_two.body);
        let items = page_two.body["items"].as_array().context("missing items")?;
        ensure!(items.len() == 1);
        ensure!(items[0]["id"].as_i64() == Some(first_id));

        // 倒序：第一条是最新创建。
        let latest = &list.body["items"][0];
        ensure!(latest["id"].as_i64() > Some(first_id));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
