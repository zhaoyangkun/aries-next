//! Phase 06 Contract Test：导航菜单 Admin 管理 + 两级限制 + 原子排序 + Public 树。

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

async fn create_nav_item(
    app: &TestApp,
    cookie: &str,
    body: serde_json::Value,
) -> anyhow::Result<common::TestResponse> {
    app.admin_post("/api/admin/navigation", body, Some(cookie))
        .await
}

#[tokio::test]
async fn navigation_hierarchy_target_check_reorder_and_public_tree() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 前置数据：article 分类、一篇已发布文章、一个已发布页面、一个草稿页面（死目标）。
        let category = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Rust", "slug": "rust" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(category.status == StatusCode::CREATED, "{}", category.body);
        let category_id = category.body["id"].as_i64().context("missing category id")?;

        let article = app
            .admin_post(
                "/api/admin/articles",
                serde_json::json!({ "title": "Rust Post", "markdown_source": "# Rust" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(article.status == StatusCode::CREATED, "{}", article.body);
        let article_id = article.body["id"].as_i64().context("missing article id")?;
        let article_version = article.body["version"].as_i64().context("missing version")?;
        let article_slug = article.body["slug"].as_str().context("missing slug")?.to_owned();
        let published = app
            .admin_patch(
                &format!("/api/admin/articles/{article_id}/status"),
                serde_json::json!({ "command": "publish", "expected_version": article_version }),
                &owner_cookie,
            )
            .await?;
        ensure!(published.status == StatusCode::OK, "{}", published.body);

        let page = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "about", "title": "About", "status": "published" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(page.status == StatusCode::CREATED, "{}", page.body);
        let page_id = page.body["id"].as_i64().context("missing page id")?;
        let draft_page = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "wip", "title": "WIP" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(draft_page.status == StatusCode::CREATED, "{}", draft_page.body);
        let draft_page_id = draft_page.body["id"].as_i64().context("missing draft page id")?;

        // 一级：url 目标。
        let home = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "Home", "target_type": "url", "url": "https://blog.example.com/",
            }),
        )
        .await?;
        ensure!(home.status == StatusCode::CREATED, "{}", home.body);
        let home_id = home.body["id"].as_i64().context("missing id")?;

        // 一级：article 目标（指向已发布文章）。
        let posts = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Posts", "target_type": "article", "target_id": article_id }),
        )
        .await?;
        ensure!(posts.status == StatusCode::CREATED, "{}", posts.body);
        let posts_id = posts.body["id"].as_i64().context("missing id")?;

        // 二级：挂在 Posts 下（指向真实分类）。
        let child = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "Rust", "target_type": "category", "target_id": category_id,
                "parent_id": posts_id,
            }),
        )
        .await?;
        ensure!(child.status == StatusCode::CREATED, "{}", child.body);
        let child_id = child.body["id"].as_i64().context("missing id")?;

        // 一级：page 目标（已发布页面）；以及指向草稿页面的死目标节点。
        let about = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "About", "target_type": "page", "target_id": page_id }),
        )
        .await?;
        ensure!(about.status == StatusCode::CREATED, "{}", about.body);
        let about_id = about.body["id"].as_i64().context("missing id")?;
        let dead = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Dead", "target_type": "page", "target_id": draft_page_id }),
        )
        .await?;
        ensure!(dead.status == StatusCode::CREATED, "{}", dead.body);
        let dead_id = dead.body["id"].as_i64().context("missing id")?;

        // 三级：挂在二级节点下 → 400 层级限制。
        let third_level = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "Too Deep", "target_type": "url", "url": "https://x.com",
                "parent_id": child_id,
            }),
        )
        .await?;
        ensure!(third_level.status == StatusCode::BAD_REQUEST, "{}", third_level.body);
        ensure!(third_level.body["error"]["code"] == "INVALID_NAVIGATION_HIERARCHY");

        // target_type 与字段不匹配：url 类型缺 url → 400。
        let mismatch = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Broken", "target_type": "url" }),
        )
        .await?;
        ensure!(mismatch.status == StatusCode::BAD_REQUEST, "{}", mismatch.body);
        ensure!(mismatch.body["error"]["code"] == "INVALID_NAVIGATION_TARGET");

        // article 类型缺 target_id → 400；危险 URL → 400。
        let missing_id = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Broken", "target_type": "article" }),
        )
        .await?;
        ensure!(missing_id.status == StatusCode::BAD_REQUEST);
        let evil_url = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Evil", "target_type": "url", "url": "javascript:alert(1)" }),
        )
        .await?;
        ensure!(evil_url.status == StatusCode::BAD_REQUEST);
        ensure!(evil_url.body["error"]["code"] == "INVALID_NAVIGATION_URL");

        // 隐藏项：不出现在 Public 树。
        let hidden = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "Secret", "target_type": "url", "url": "https://secret.example.com",
                "visible": false,
            }),
        )
        .await?;
        ensure!(hidden.status == StatusCode::CREATED, "{}", hidden.body);

        // 原子批量排序：Posts 排到 Home 前。
        let reordered = app
            .admin_put(
                "/api/admin/navigation/order",
                serde_json::json!({ "item_ids": [posts_id, home_id, child_id, about_id, dead_id] }),
                &owner_cookie,
            )
            .await?;
        ensure!(reordered.status == StatusCode::NO_CONTENT, "{}", reordered.body);
        let list = app.admin_get("/api/admin/navigation", &owner_cookie).await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        let items = list.body.as_array().context("missing navigation list")?;
        let sort_of = |id: i64| {
            items
                .iter()
                .find(|item| item["id"].as_i64() == Some(id))
                .and_then(|item| item["sort_order"].as_i64())
        };
        ensure!(sort_of(posts_id) == Some(0));
        ensure!(sort_of(home_id) == Some(1));
        ensure!(sort_of(child_id) == Some(2));

        // 排序包含不存在的 id → 404，且已有顺序不被破坏（原子性）。
        let bad_reorder = app
            .admin_put(
                "/api/admin/navigation/order",
                serde_json::json!({ "item_ids": [home_id, 999999] }),
                &owner_cookie,
            )
            .await?;
        ensure!(bad_reorder.status == StatusCode::NOT_FOUND, "{}", bad_reorder.body);
        let list_after = app.admin_get("/api/admin/navigation", &owner_cookie).await?;
        let items_after = list_after.body.as_array().context("missing list")?;
        let sort_after = |id: i64| {
            items_after
                .iter()
                .find(|item| item["id"].as_i64() == Some(id))
                .and_then(|item| item["sort_order"].as_i64())
        };
        ensure!(sort_after(posts_id) == Some(0), "reorder must roll back atomically");

        // Public 树（无 Cookie）：href 由服务端解析；死目标（草稿页面）与隐藏项被剔除。
        let public = app.get("/api/public/navigation").await?;
        ensure!(public.status == StatusCode::OK, "{}", public.body);
        let tree = public.body.as_array().context("missing tree")?;
        ensure!(
            tree.len() == 3,
            "dead/hidden items must be excluded: {}",
            public.body
        );
        // Posts（sort 0）：article 目标解析为 /articles/{slug}，含子节点。
        ensure!(tree[0]["label"] == "Posts");
        let expected_article_href = format!("/articles/{article_slug}");
        ensure!(tree[0]["href"].as_str() == Some(expected_article_href.as_str()));
        ensure!(tree[0]["children"].as_array().map(Vec::len) == Some(1));
        ensure!(tree[0]["children"][0]["label"] == "Rust");
        ensure!(tree[0]["children"][0]["href"] == "/categories/rust");
        // Home（sort 1）：url 原样保留。
        ensure!(tree[1]["label"] == "Home");
        ensure!(tree[1]["href"] == "https://blog.example.com/");
        ensure!(tree[1]["url"] == "https://blog.example.com/");
        // About（sort 3）：page 目标解析为 /custom/{slug}（对齐旧版 xue 路由）。
        ensure!(tree[2]["label"] == "About");
        ensure!(tree[2]["href"] == "/custom/about");
        // 公开节点不回 visible/sort_order 等管理字段；保留 target_type/target_id 向后兼容。
        ensure!(tree[0].get("visible").is_none());
        ensure!(tree[0]["target_type"] == "article");

        // 有子节点的节点不可直接删除 → 409。
        let blocked = app
            .admin_delete(&format!("/api/admin/navigation/{posts_id}"), &owner_cookie)
            .await?;
        ensure!(blocked.status == StatusCode::CONFLICT, "{}", blocked.body);
        ensure!(blocked.body["error"]["code"] == "NAVIGATION_CONFLICT");

        // 先删子再删父。
        let child_deleted = app
            .admin_delete(&format!("/api/admin/navigation/{child_id}"), &owner_cookie)
            .await?;
        ensure!(child_deleted.status == StatusCode::NO_CONTENT);
        let parent_deleted = app
            .admin_delete(&format!("/api/admin/navigation/{posts_id}"), &owner_cookie)
            .await?;
        ensure!(parent_deleted.status == StatusCode::NO_CONTENT);

        // 审计：created ×6 + reordered + deleted ×2。
        let (audit_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs WHERE target_type = 'navigation'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(audit_count == 9, "expected 9 navigation audit events, got {audit_count}");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn navigation_permission_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-n", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-n", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-n", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-n", "moderator-pass-1").await?;

        let created = create_nav_item(
            &app,
            &editor_cookie,
            serde_json::json!({ "label": "Editor Nav", "target_type": "url", "url": "https://e.com" }),
        )
        .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);

        let denied = app.admin_get("/api/admin/navigation", &moderator_cookie).await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);

        let unauthenticated = app.get("/api/admin/navigation").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn navigation_create_validation_edges() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "moderator-c", "moderator", "moderator-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-c", "moderator-pass-1").await?;

        // 前置：一个一级节点与挂在它下面的二级节点，用于层级边界。
        let parent = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Parent", "target_type": "url", "url": "https://p.example.com" }),
        )
        .await?;
        ensure!(parent.status == StatusCode::CREATED, "{}", parent.body);
        let parent_id = parent.body["id"].as_i64().context("missing id")?;
        let child = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "Child", "target_type": "url", "url": "https://c.example.com",
                "parent_id": parent_id,
            }),
        )
        .await?;
        ensure!(child.status == StatusCode::CREATED, "{}", child.body);
        let child_id = child.body["id"].as_i64().context("missing id")?;

        // 父节点不存在 → 400 层级错误（两级限制要求父节点必须存在）。
        let orphan = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "Orphan", "target_type": "url", "url": "https://o.example.com",
                "parent_id": 999999,
            }),
        )
        .await?;
        ensure!(orphan.status == StatusCode::BAD_REQUEST, "{}", orphan.body);
        ensure!(orphan.body["error"]["code"] == "INVALID_NAVIGATION_HIERARCHY");

        // 父节点自身已有 parent（二级节点不能再当父节点）→ 400。
        let third_level = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "Third", "target_type": "url", "url": "https://t.example.com",
                "parent_id": child_id,
            }),
        )
        .await?;
        ensure!(third_level.status == StatusCode::BAD_REQUEST, "{}", third_level.body);
        ensure!(third_level.body["error"]["code"] == "INVALID_NAVIGATION_HIERARCHY");

        // 未知 target_type → 400。
        let bad_type = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Bad Type", "target_type": "external", "url": "https://x.com" }),
        )
        .await?;
        ensure!(bad_type.status == StatusCode::BAD_REQUEST, "{}", bad_type.body);
        ensure!(bad_type.body["error"]["code"] == "INVALID_NAVIGATION_TARGET_TYPE");

        // 空标签与超长标签（> 60 字符）→ 400。
        let empty_label = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "   ", "target_type": "url", "url": "https://x.com" }),
        )
        .await?;
        ensure!(empty_label.status == StatusCode::BAD_REQUEST, "{}", empty_label.body);
        ensure!(empty_label.body["error"]["code"] == "INVALID_NAVIGATION_LABEL");
        let long_label = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "a".repeat(61), "target_type": "url", "url": "https://x.com",
            }),
        )
        .await?;
        ensure!(long_label.status == StatusCode::BAD_REQUEST, "{}", long_label.body);
        ensure!(long_label.body["error"]["code"] == "INVALID_NAVIGATION_LABEL");

        // 未认证创建 → 401；moderator 无 ManageContent 权限 → 403。
        let unauthenticated = app
            .admin_post(
                "/api/admin/navigation",
                serde_json::json!({ "label": "Anon", "target_type": "url", "url": "https://x.com" }),
                None,
            )
            .await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        let forbidden = create_nav_item(
            &app,
            &moderator_cookie,
            serde_json::json!({ "label": "Mod", "target_type": "url", "url": "https://x.com" }),
        )
        .await?;
        ensure!(forbidden.status == StatusCode::FORBIDDEN, "{}", forbidden.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn navigation_update_crud_and_hierarchy() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "moderator-u", "moderator", "moderator-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-u", "moderator-pass-1").await?;

        // 正常更新：全量字段改写。
        let item = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Old", "target_type": "url", "url": "https://old.example.com" }),
        )
        .await?;
        ensure!(item.status == StatusCode::CREATED, "{}", item.body);
        let item_id = item.body["id"].as_i64().context("missing id")?;
        let updated = app
            .admin_put(
                &format!("/api/admin/navigation/{item_id}"),
                serde_json::json!({
                    "label": "Updated", "target_type": "url", "url": "https://new.example.com",
                    "open_in_new_tab": true, "visible": false, "sort_order": 7,
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["label"] == "Updated");
        ensure!(updated.body["url"] == "https://new.example.com");
        ensure!(updated.body["open_in_new_tab"] == true);
        ensure!(updated.body["visible"] == false);
        ensure!(updated.body["sort_order"] == 7);

        // 列表回读确认持久化。
        let list = app.admin_get("/api/admin/navigation", &owner_cookie).await?;
        let persisted = list
            .body
            .as_array()
            .context("missing list")?
            .iter()
            .find(|entry| entry["id"].as_i64() == Some(item_id))
            .context("updated item missing from list")?;
        ensure!(persisted["label"] == "Updated");
        ensure!(persisted["sort_order"] == 7);

        // 层级前置：parent_a、parent_b 两个一级节点，child 挂在 parent_a 下。
        let parent_a = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "A", "target_type": "url", "url": "https://a.example.com" }),
        )
        .await?;
        let parent_a_id = parent_a.body["id"].as_i64().context("missing id")?;
        let parent_b = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "B", "target_type": "url", "url": "https://b.example.com" }),
        )
        .await?;
        let parent_b_id = parent_b.body["id"].as_i64().context("missing id")?;
        let child = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({
                "label": "C", "target_type": "url", "url": "https://c.example.com",
                "parent_id": parent_a_id,
            }),
        )
        .await?;
        let child_id = child.body["id"].as_i64().context("missing id")?;

        // 正常换父：child 从 parent_a 移到 parent_b。
        let reparented = app
            .admin_put(
                &format!("/api/admin/navigation/{child_id}"),
                serde_json::json!({
                    "label": "C", "target_type": "url", "url": "https://c.example.com",
                    "parent_id": parent_b_id,
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(reparented.status == StatusCode::OK, "{}", reparented.body);
        ensure!(reparented.body["parent_id"] == parent_b_id);

        // 节点挂到自身 → 400。
        let self_parent = app
            .admin_put(
                &format!("/api/admin/navigation/{parent_a_id}"),
                serde_json::json!({
                    "label": "A", "target_type": "url", "url": "https://a.example.com",
                    "parent_id": parent_a_id,
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(self_parent.status == StatusCode::BAD_REQUEST, "{}", self_parent.body);
        ensure!(self_parent.body["error"]["code"] == "INVALID_NAVIGATION_HIERARCHY");

        // 父节点不存在 → 400。
        let missing_parent = app
            .admin_put(
                &format!("/api/admin/navigation/{child_id}"),
                serde_json::json!({
                    "label": "C", "target_type": "url", "url": "https://c.example.com",
                    "parent_id": 999999,
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(missing_parent.status == StatusCode::BAD_REQUEST, "{}", missing_parent.body);
        ensure!(missing_parent.body["error"]["code"] == "INVALID_NAVIGATION_HIERARCHY");

        // 有子节点的节点（parent_b 下有 child）再挂到别人下面 → 400（会形成第三级）。
        let move_parent = app
            .admin_put(
                &format!("/api/admin/navigation/{parent_b_id}"),
                serde_json::json!({
                    "label": "B", "target_type": "url", "url": "https://b.example.com",
                    "parent_id": parent_a_id,
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(move_parent.status == StatusCode::BAD_REQUEST, "{}", move_parent.body);
        ensure!(move_parent.body["error"]["code"] == "INVALID_NAVIGATION_HIERARCHY");

        // 挂到二级节点（child 自身有 parent）→ 400。
        let under_second_level = app
            .admin_put(
                &format!("/api/admin/navigation/{item_id}"),
                serde_json::json!({
                    "label": "Updated", "target_type": "url", "url": "https://new.example.com",
                    "parent_id": child_id,
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            under_second_level.status == StatusCode::BAD_REQUEST,
            "{}",
            under_second_level.body
        );
        ensure!(under_second_level.body["error"]["code"] == "INVALID_NAVIGATION_HIERARCHY");

        // 更新不存在的节点 → 404。
        let not_found = app
            .admin_put(
                "/api/admin/navigation/999999",
                serde_json::json!({ "label": "Ghost", "target_type": "url", "url": "https://g.com" }),
                &owner_cookie,
            )
            .await?;
        ensure!(not_found.status == StatusCode::NOT_FOUND, "{}", not_found.body);
        ensure!(not_found.body["error"]["code"] == "NAVIGATION_NOT_FOUND");

        // 更新入参校验：空标签、未知 target_type、目标字段不匹配、危险 URL → 400。
        let bad_label = app
            .admin_put(
                &format!("/api/admin/navigation/{item_id}"),
                serde_json::json!({ "label": "", "target_type": "url", "url": "https://x.com" }),
                &owner_cookie,
            )
            .await?;
        ensure!(bad_label.status == StatusCode::BAD_REQUEST, "{}", bad_label.body);
        ensure!(bad_label.body["error"]["code"] == "INVALID_NAVIGATION_LABEL");
        let bad_type = app
            .admin_put(
                &format!("/api/admin/navigation/{item_id}"),
                serde_json::json!({ "label": "X", "target_type": "external", "url": "https://x.com" }),
                &owner_cookie,
            )
            .await?;
        ensure!(bad_type.status == StatusCode::BAD_REQUEST, "{}", bad_type.body);
        ensure!(bad_type.body["error"]["code"] == "INVALID_NAVIGATION_TARGET_TYPE");
        let bad_target = app
            .admin_put(
                &format!("/api/admin/navigation/{item_id}"),
                serde_json::json!({ "label": "X", "target_type": "article" }),
                &owner_cookie,
            )
            .await?;
        ensure!(bad_target.status == StatusCode::BAD_REQUEST, "{}", bad_target.body);
        ensure!(bad_target.body["error"]["code"] == "INVALID_NAVIGATION_TARGET");
        let bad_url = app
            .admin_put(
                &format!("/api/admin/navigation/{item_id}"),
                serde_json::json!({ "label": "X", "target_type": "url", "url": "ftp://x.com" }),
                &owner_cookie,
            )
            .await?;
        ensure!(bad_url.status == StatusCode::BAD_REQUEST, "{}", bad_url.body);
        ensure!(bad_url.body["error"]["code"] == "INVALID_NAVIGATION_URL");

        // 未认证更新 → 401；moderator → 403。
        let unauthenticated = app
            .admin_put(
                &format!("/api/admin/navigation/{item_id}"),
                serde_json::json!({ "label": "X", "target_type": "url", "url": "https://x.com" }),
                "aries_admin_session=invalid-token",
            )
            .await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        let forbidden = app
            .admin_put(
                &format!("/api/admin/navigation/{item_id}"),
                serde_json::json!({ "label": "X", "target_type": "url", "url": "https://x.com" }),
                &moderator_cookie,
            )
            .await?;
        ensure!(forbidden.status == StatusCode::FORBIDDEN, "{}", forbidden.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn navigation_delete_and_reorder_guards() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "moderator-d", "moderator", "moderator-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-d", "moderator-pass-1").await?;

        // 删除不存在的节点 → 404。
        let not_found = app
            .admin_delete("/api/admin/navigation/999999", &owner_cookie)
            .await?;
        ensure!(
            not_found.status == StatusCode::NOT_FOUND,
            "{}",
            not_found.body
        );
        ensure!(not_found.body["error"]["code"] == "NAVIGATION_NOT_FOUND");

        // 未认证删除 → 401；moderator 删除 → 403。
        let victim = create_nav_item(
            &app,
            &owner_cookie,
            serde_json::json!({ "label": "Victim", "target_type": "url", "url": "https://v.com" }),
        )
        .await?;
        let victim_id = victim.body["id"].as_i64().context("missing id")?;
        let unauthenticated = app
            .admin_delete(
                &format!("/api/admin/navigation/{victim_id}"),
                "aries_admin_session=invalid-token",
            )
            .await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        let forbidden = app
            .admin_delete(
                &format!("/api/admin/navigation/{victim_id}"),
                &moderator_cookie,
            )
            .await?;
        ensure!(
            forbidden.status == StatusCode::FORBIDDEN,
            "{}",
            forbidden.body
        );

        // 正常删除 → 204；物理删除后再删一次 → 404。
        let deleted = app
            .admin_delete(&format!("/api/admin/navigation/{victim_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT);
        let deleted_again = app
            .admin_delete(&format!("/api/admin/navigation/{victim_id}"), &owner_cookie)
            .await?;
        ensure!(deleted_again.status == StatusCode::NOT_FOUND);

        // 排序未认证 → 401；moderator → 403。
        let reorder_anon = app
            .admin_put(
                "/api/admin/navigation/order",
                serde_json::json!({ "item_ids": [] }),
                "aries_admin_session=invalid-token",
            )
            .await?;
        ensure!(reorder_anon.status == StatusCode::UNAUTHORIZED);
        let reorder_forbidden = app
            .admin_put(
                "/api/admin/navigation/order",
                serde_json::json!({ "item_ids": [] }),
                &moderator_cookie,
            )
            .await?;
        ensure!(reorder_forbidden.status == StatusCode::FORBIDDEN);

        // 空数组排序 → 204 无操作。
        let empty_reorder = app
            .admin_put(
                "/api/admin/navigation/order",
                serde_json::json!({ "item_ids": [] }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            empty_reorder.status == StatusCode::NO_CONTENT,
            "{}",
            empty_reorder.body
        );

        // 部分排序：只重写入参 id 的 sort_order（从 0 起），未列入的保持原值。
        let mut ids = Vec::new();
        for (index, label) in ["S1", "S2", "S3"].iter().enumerate() {
            let created = create_nav_item(
                &app,
                &owner_cookie,
                serde_json::json!({
                    "label": label, "target_type": "url",
                    "url": format!("https://s{index}.example.com"),
                    "sort_order": (index as i32 + 1) * 10,
                }),
            )
            .await?;
            ensure!(created.status == StatusCode::CREATED, "{}", created.body);
            ids.push(created.body["id"].as_i64().context("missing id")?);
        }
        let partial = app
            .admin_put(
                "/api/admin/navigation/order",
                serde_json::json!({ "item_ids": [ids[2], ids[0]] }),
                &owner_cookie,
            )
            .await?;
        ensure!(partial.status == StatusCode::NO_CONTENT, "{}", partial.body);
        let list = app
            .admin_get("/api/admin/navigation", &owner_cookie)
            .await?;
        let items = list.body.as_array().context("missing list")?;
        let sort_of = |id: i64| {
            items
                .iter()
                .find(|item| item["id"].as_i64() == Some(id))
                .and_then(|item| item["sort_order"].as_i64())
        };
        ensure!(sort_of(ids[2]) == Some(0));
        ensure!(sort_of(ids[0]) == Some(1));
        ensure!(
            sort_of(ids[1]) == Some(20),
            "unlisted item must keep its sort_order"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
