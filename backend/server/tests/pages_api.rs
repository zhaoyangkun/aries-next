//! Phase 06 Contract Test：自定义页面 Admin CRUD + Public 只读端点。
//! 通过 `ARIES_RUN_DATABASE_TESTS=1` 门控，随机 Schema 隔离，走真实 Router + PostgreSQL。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

/// 直接造一个指定 Role 的用户，返回其 id。
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
async fn page_crud_flow_covers_slug_conflict_render_and_audit() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 创建草稿页面：Markdown 由 HTTP 层渲染，script 必须被 Sanitize。
        let created = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({
                    "slug": "about",
                    "title": "About",
                    "content_markdown": "# Hello <script>alert(1)</script>",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        ensure!(created.body["slug"] == "about");
        ensure!(created.body["status"] == "draft");
        let html = created.body["content_html"]
            .as_str()
            .context("missing html")?;
        ensure!(html.contains("Hello"), "html: {html}");
        ensure!(!html.contains("<script>"), "html: {html}");
        let page_id = created.body["id"].as_i64().context("missing page id")?;

        // slug 重复 → 409。
        let duplicate = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "about", "title": "Again" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            duplicate.status == StatusCode::CONFLICT,
            "{}",
            duplicate.body
        );
        ensure!(duplicate.body["error"]["code"] == "SLUG_CONFLICT");

        // 非法 slug（大写/空格）→ 400。
        let invalid = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "About Me", "title": "Bad" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            invalid.status == StatusCode::BAD_REQUEST,
            "{}",
            invalid.body
        );
        ensure!(invalid.body["error"]["code"] == "INVALID_PAGE_SLUG");

        // 草稿对 Public 不可见 → 404。
        let draft_public = app.get("/api/public/pages/about").await?;
        ensure!(draft_public.status == StatusCode::NOT_FOUND);

        // 发布后 Public 可读（无需 Cookie）。
        let updated = app
            .admin_put(
                &format!("/api/admin/pages/{page_id}"),
                serde_json::json!({
                    "slug": "about",
                    "title": "About",
                    "content_markdown": "# Hello",
                    "status": "published",
                    "sort_order": 1,
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["status"] == "published");

        let public = app.get("/api/public/pages/about").await?;
        ensure!(public.status == StatusCode::OK, "{}", public.body);
        ensure!(public.body["title"] == "About");
        // 公开 DTO 不回 Markdown 源。
        ensure!(public.body.get("content_markdown").is_none());

        // 列表 + status 过滤 + 分页语义。
        let list = app
            .admin_get(
                "/api/admin/pages?status=published&page=1&page_size=10",
                &owner_cookie,
            )
            .await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body["total"].as_i64() == Some(1));
        ensure!(list.body["page_size"].as_u64() == Some(10));
        let drafts = app
            .admin_get("/api/admin/pages?status=draft", &owner_cookie)
            .await?;
        ensure!(drafts.body["total"].as_i64() == Some(0));

        // 软删除后 Admin/Public 均 404。
        let deleted = app
            .admin_delete(&format!("/api/admin/pages/{page_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let gone = app
            .admin_get(&format!("/api/admin/pages/{page_id}"), &owner_cookie)
            .await?;
        ensure!(gone.status == StatusCode::NOT_FOUND);
        let public_gone = app.get("/api/public/pages/about").await?;
        ensure!(public_gone.status == StatusCode::NOT_FOUND);

        // 审计落库：created + updated + deleted。
        let (audit_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs \
             WHERE target_type = 'page' \
             AND action IN ('page.created', 'page.updated', 'page.deleted')",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(
            audit_count == 3,
            "expected 3 page audit events, got {audit_count}"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn page_permission_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-p", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-p", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-p", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-p", "moderator-pass-1").await?;

        // editor 拥有 ManageContent，可创建页面。
        let created = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "editor-page", "title": "By editor" }),
                Some(&editor_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);

        // moderator 无 ManageContent → 403。
        let denied = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "mod-page", "title": "By moderator" }),
                Some(&moderator_cookie),
            )
            .await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);
        ensure!(denied.body["error"]["code"] == "PERMISSION_DENIED");
        let denied_list = app.admin_get("/api/admin/pages", &moderator_cookie).await?;
        ensure!(denied_list.status == StatusCode::FORBIDDEN);

        // 未认证 → 401。
        let unauthenticated = app.get("/api/admin/pages").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn page_create_validation_rejects_bad_input() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 空 slug 与超长 slug（> 160 字符）→ 400 INVALID_PAGE_SLUG。
        for slug in ["", &"a".repeat(161)] {
            let response = app
                .admin_post(
                    "/api/admin/pages",
                    serde_json::json!({ "slug": slug, "title": "Valid" }),
                    Some(&owner_cookie),
                )
                .await?;
            ensure!(
                response.status == StatusCode::BAD_REQUEST,
                "slug={slug:?}: {}",
                response.body
            );
            ensure!(response.body["error"]["code"] == "INVALID_PAGE_SLUG");
        }

        // 空标题、纯空白标题、超长标题（> 200 字符）→ 400 INVALID_PAGE_TITLE。
        for title in ["", "   ", &"题".repeat(201)] {
            let response = app
                .admin_post(
                    "/api/admin/pages",
                    serde_json::json!({ "slug": "valid-slug", "title": title }),
                    Some(&owner_cookie),
                )
                .await?;
            ensure!(
                response.status == StatusCode::BAD_REQUEST,
                "title={title:?}: {}",
                response.body
            );
            ensure!(response.body["error"]["code"] == "INVALID_PAGE_TITLE");
        }

        // 未知 status → 400 INVALID_PAGE_STATUS。
        let bad_status = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({
                    "slug": "valid-slug",
                    "title": "Valid",
                    "status": "archived",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            bad_status.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_status.body
        );
        ensure!(bad_status.body["error"]["code"] == "INVALID_PAGE_STATUS");

        // 全部校验失败请求不得落库。
        let list = app.admin_get("/api/admin/pages", &owner_cookie).await?;
        ensure!(list.body["total"].as_i64() == Some(0));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn page_get_update_delete_by_id() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 创建后按 ID 读取：Admin DTO 含 Markdown 源与 created_by。
        let created = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({
                    "slug": "detail",
                    "title": "Detail",
                    "content_markdown": "first",
                    "status": "published",
                    "sort_order": 3,
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        let page_id = created.body["id"].as_i64().context("missing page id")?;
        ensure!(created.body["created_by"].as_i64().is_some());

        let fetched = app
            .admin_get(&format!("/api/admin/pages/{page_id}"), &owner_cookie)
            .await?;
        ensure!(fetched.status == StatusCode::OK, "{}", fetched.body);
        ensure!(fetched.body["content_markdown"] == "first");
        ensure!(fetched.body["sort_order"].as_i64() == Some(3));

        // 全量更新：HTML 由 HTTP 层重新渲染，updated_at 前进。
        let updated = app
            .admin_put(
                &format!("/api/admin/pages/{page_id}"),
                serde_json::json!({
                    "slug": "detail",
                    "title": "Detail v2",
                    "content_markdown": "**bold**",
                    "status": "published",
                    "sort_order": 7,
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["title"] == "Detail v2");
        ensure!(updated.body["content_markdown"] == "**bold**");
        let html = updated.body["content_html"]
            .as_str()
            .context("missing html")?;
        ensure!(html.contains("<strong>bold</strong>"), "html: {html}");
        ensure!(updated.body["sort_order"].as_i64() == Some(7));
        ensure!(
            updated.body["updated_at"].as_str() >= created.body["updated_at"].as_str(),
            "updated_at did not advance"
        );

        // PUT 省略 status 时回落为 draft（PagePayload.status 默认 None → Draft）。
        let no_status = app
            .admin_put(
                &format!("/api/admin/pages/{page_id}"),
                serde_json::json!({
                    "slug": "detail",
                    "title": "Detail v3",
                    "content_markdown": "x",
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(no_status.status == StatusCode::OK, "{}", no_status.body);
        ensure!(no_status.body["status"] == "draft");

        // 不存在的 ID：读 / 改 / 删均 404 PAGE_NOT_FOUND。
        let missing_get = app
            .admin_get("/api/admin/pages/999999", &owner_cookie)
            .await?;
        ensure!(missing_get.status == StatusCode::NOT_FOUND);
        ensure!(missing_get.body["error"]["code"] == "PAGE_NOT_FOUND");
        let missing_put = app
            .admin_put(
                "/api/admin/pages/999999",
                serde_json::json!({ "slug": "ghost", "title": "Ghost" }),
                &owner_cookie,
            )
            .await?;
        ensure!(missing_put.status == StatusCode::NOT_FOUND);
        let missing_delete = app
            .admin_delete("/api/admin/pages/999999", &owner_cookie)
            .await?;
        ensure!(missing_delete.status == StatusCode::NOT_FOUND);

        // 重复删除同一页面 → 404（软删除只命中 deleted_at IS NULL 的行）。
        let first_delete = app
            .admin_delete(&format!("/api/admin/pages/{page_id}"), &owner_cookie)
            .await?;
        ensure!(first_delete.status == StatusCode::NO_CONTENT);
        let second_delete = app
            .admin_delete(&format!("/api/admin/pages/{page_id}"), &owner_cookie)
            .await?;
        ensure!(second_delete.status == StatusCode::NOT_FOUND);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn page_slug_partial_unique_index_allows_reuse_after_delete() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 未删除的同 slug 冲突 → 409（部分唯一索引 WHERE deleted_at IS NULL 生效）。
        let first = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "reuse-me", "title": "First" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(first.status == StatusCode::CREATED, "{}", first.body);
        let first_id = first.body["id"].as_i64().context("missing page id")?;

        let conflict = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "reuse-me", "title": "Second" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(conflict.status == StatusCode::CONFLICT, "{}", conflict.body);
        ensure!(conflict.body["error"]["code"] == "SLUG_CONFLICT");

        // 更新到他人占用的 slug 同样 → 409。
        let other = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "other", "title": "Other" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(other.status == StatusCode::CREATED, "{}", other.body);
        let other_id = other.body["id"].as_i64().context("missing page id")?;
        let steal = app
            .admin_put(
                &format!("/api/admin/pages/{other_id}"),
                serde_json::json!({ "slug": "reuse-me", "title": "Other" }),
                &owner_cookie,
            )
            .await?;
        ensure!(steal.status == StatusCode::CONFLICT, "{}", steal.body);
        ensure!(steal.body["error"]["code"] == "SLUG_CONFLICT");

        // 软删除后 slug 释放：可重建同名页面，且拿到全新 ID。
        let deleted = app
            .admin_delete(&format!("/api/admin/pages/{first_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let recreated = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({
                    "slug": "reuse-me",
                    "title": "Reborn",
                    "status": "published",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            recreated.status == StatusCode::CREATED,
            "{}",
            recreated.body
        );
        let recreated_id = recreated.body["id"].as_i64().context("missing page id")?;
        ensure!(recreated_id != first_id);
        let public = app.get("/api/public/pages/reuse-me").await?;
        ensure!(public.status == StatusCode::OK, "{}", public.body);
        ensure!(public.body["title"] == "Reborn");

        // 已删除行的 slug 也可通过 UPDATE 复用：先删再改名不冲突。
        let third = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "third", "title": "Third" }),
                Some(&owner_cookie),
            )
            .await?;
        let third_id = third.body["id"].as_i64().context("missing page id")?;
        let deleted_again = app
            .admin_delete(&format!("/api/admin/pages/{recreated_id}"), &owner_cookie)
            .await?;
        ensure!(deleted_again.status == StatusCode::NO_CONTENT);
        let renamed = app
            .admin_put(
                &format!("/api/admin/pages/{third_id}"),
                serde_json::json!({ "slug": "reuse-me", "title": "Third" }),
                &owner_cookie,
            )
            .await?;
        ensure!(renamed.status == StatusCode::OK, "{}", renamed.body);
        ensure!(renamed.body["slug"] == "reuse-me");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn page_list_keyword_ordering_and_pagination() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 三个页面：排序键 2/1/1，标题与 slug 供关键字过滤。
        for (slug, title, sort_order) in [
            ("guestbook", "Guestbook", 2),
            ("about-site", "About Site", 1),
            ("friends-links", "Friends", 1),
        ] {
            let created = app
                .admin_post(
                    "/api/admin/pages",
                    serde_json::json!({
                        "slug": slug,
                        "title": title,
                        "sort_order": sort_order,
                    }),
                    Some(&owner_cookie),
                )
                .await?;
            ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        }

        // 默认列表按 (sort_order, id) 升序：两个 sort_order=1 的按创建先后在前。
        let list = app.admin_get("/api/admin/pages", &owner_cookie).await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body["total"].as_i64() == Some(3));
        let slugs: Vec<&str> = list.body["items"]
            .as_array()
            .context("items is not an array")?
            .iter()
            .filter_map(|item| item["slug"].as_str())
            .collect();
        ensure!(
            slugs == ["about-site", "friends-links", "guestbook"],
            "unexpected order: {slugs:?}"
        );

        // keyword 同时匹配 title 与 slug（ILIKE 子串，大小写不敏感）。
        let by_title = app
            .admin_get("/api/admin/pages?keyword=guest", &owner_cookie)
            .await?;
        ensure!(by_title.body["total"].as_i64() == Some(1));
        ensure!(by_title.body["items"][0]["slug"] == "guestbook");
        let by_slug = app
            .admin_get("/api/admin/pages?keyword=LINKS", &owner_cookie)
            .await?;
        ensure!(by_slug.body["total"].as_i64() == Some(1));
        ensure!(by_slug.body["items"][0]["slug"] == "friends-links");
        let no_hit = app
            .admin_get("/api/admin/pages?keyword=nothing-matches", &owner_cookie)
            .await?;
        ensure!(no_hit.body["total"].as_i64() == Some(0));
        // 纯空白 keyword 被 HTTP 层丢弃，不参与过滤。
        let blank = app
            .admin_get("/api/admin/pages?keyword=%20%20", &owner_cookie)
            .await?;
        ensure!(blank.body["total"].as_i64() == Some(3));

        // 分页：第二页只剩 1 条，total 仍为全集。
        let page_two = app
            .admin_get("/api/admin/pages?page=2&page_size=2", &owner_cookie)
            .await?;
        ensure!(page_two.body["total"].as_i64() == Some(3));
        ensure!(page_two.body["page"].as_u64() == Some(2));
        let items = page_two.body["items"].as_array().context("items")?;
        ensure!(items.len() == 1, "expected 1 item, got {}", items.len());
        ensure!(items[0]["slug"] == "guestbook");

        // page_size 超上限被钳制到 100；page=0 按第 1 页处理。
        let clamped = app
            .admin_get("/api/admin/pages?page_size=500", &owner_cookie)
            .await?;
        ensure!(clamped.body["page_size"].as_u64() == Some(100));
        let zero_page = app
            .admin_get("/api/admin/pages?page=0", &owner_cookie)
            .await?;
        ensure!(zero_page.body["page"].as_u64() == Some(1));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
