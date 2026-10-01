//! Phase 06 Contract Test：友情链接 Admin CRUD + 分类管理（kind = link）+ Public 端点。

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
async fn link_crud_url_validation_and_category_reference() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 友链分类（kind = link）。
        let category = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Blogs", "slug": "blogs" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(category.status == StatusCode::CREATED, "{}", category.body);
        ensure!(category.body["kind"] == "link");
        let category_id = category.body["id"]
            .as_i64()
            .context("missing category id")?;

        // 分类列表只含 link kind，不回 article 分类。
        let categories = app
            .admin_get("/api/admin/links/categories", &owner_cookie)
            .await?;
        ensure!(categories.status == StatusCode::OK, "{}", categories.body);
        ensure!(categories.body.as_array().map(Vec::len) == Some(1));

        // 创建友链。
        let created = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({
                    "category_id": category_id,
                    "title": "Example",
                    "url": "https://example.com",
                    "description": "An example site",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        ensure!(created.body["status"] == "active");
        let link_id = created.body["id"].as_i64().context("missing link id")?;

        // 危险 scheme → 400。
        let bad_url = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({ "title": "Evil", "url": "javascript:alert(1)" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            bad_url.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_url.body
        );
        ensure!(bad_url.body["error"]["code"] == "INVALID_LINK_URL");

        // Public 端点（无 Cookie）：active 可见，status 字段不外泄，分类名已解析。
        let public = app.get("/api/public/links").await?;
        ensure!(public.status == StatusCode::OK, "{}", public.body);
        ensure!(public.body.as_array().map(Vec::len) == Some(1));
        ensure!(public.body[0]["url"] == "https://example.com");
        ensure!(public.body[0].get("status").is_none());
        ensure!(public.body[0]["category_name"] == "Blogs");

        // 无分类友链：category_name 为 null 而非请求失败。
        let uncategorized = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({ "title": "Solo", "url": "https://solo.example.com" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            uncategorized.status == StatusCode::CREATED,
            "{}",
            uncategorized.body
        );
        let public_two = app.get("/api/public/links").await?;
        ensure!(public_two.body.as_array().map(Vec::len) == Some(2));
        let solo = public_two
            .body
            .as_array()
            .context("missing links")?
            .iter()
            .find(|link| link["title"] == "Solo")
            .context("missing uncategorized link")?;
        ensure!(solo["category_id"].is_null());
        ensure!(solo["category_name"].is_null());

        // 置为 inactive 后 Public 不再返回。
        let updated = app
            .admin_put(
                &format!("/api/admin/links/{link_id}"),
                serde_json::json!({
                    "category_id": category_id,
                    "title": "Example",
                    "url": "https://example.com",
                    "status": "inactive",
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        let public_after = app.get("/api/public/links").await?;
        // Example 置为 inactive 后不再出现，只剩无分类的 Solo。
        let remaining = public_after.body.as_array().context("missing links")?;
        ensure!(remaining.len() == 1, "{}", public_after.body);
        ensure!(remaining[0]["title"] == "Solo");

        // Admin 列表仍可按 status 过滤找到。
        let inactive = app
            .admin_get("/api/admin/links?status=inactive", &owner_cookie)
            .await?;
        ensure!(inactive.body["total"].as_i64() == Some(1));

        // 分类仍被引用 → 删除 409（引用保护）。
        let referenced = app
            .admin_delete(
                &format!("/api/admin/links/categories/{category_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(
            referenced.status == StatusCode::CONFLICT,
            "{}",
            referenced.body
        );
        ensure!(referenced.body["error"]["code"] == "TAXONOMY_IN_USE");

        // 删除友链（软删）后分类可删除。
        let deleted = app
            .admin_delete(&format!("/api/admin/links/{link_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let category_deleted = app
            .admin_delete(
                &format!("/api/admin/links/categories/{category_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(
            category_deleted.status == StatusCode::NO_CONTENT,
            "{}",
            category_deleted.body
        );

        // 审计：link.created ×2 + link.updated + link.deleted + link_category.created/deleted。
        let (audit_count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE target_type = 'link'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(
            audit_count == 6,
            "expected 6 link audit events, got {audit_count}"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn link_permission_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-l", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-l", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-l", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-l", "moderator-pass-1").await?;

        let created = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({ "title": "Editor Link", "url": "https://editor.example.com" }),
                Some(&editor_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);

        let denied = app.admin_get("/api/admin/links", &moderator_cookie).await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);

        let unauthenticated = app.get("/api/admin/links").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn link_create_and_update_input_validation() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 标题校验：空串、纯空白、超长（101 字符）均 → 400。
        for title in ["".to_owned(), "   ".to_owned(), "t".repeat(101)] {
            let response = app
                .admin_post(
                    "/api/admin/links",
                    serde_json::json!({ "title": title, "url": "https://example.com" }),
                    Some(&owner_cookie),
                )
                .await?;
            ensure!(
                response.status == StatusCode::BAD_REQUEST,
                "title={title:?} {}",
                response.body
            );
            ensure!(response.body["error"]["code"] == "INVALID_LINK_TITLE");
        }

        // URL scheme 白名单：javascript:/data:/ftp:/协议相对/空串/超长均 → 400，大小写不敏感。
        let invalid_urls = [
            "javascript:alert(1)".to_owned(),
            "JavaScript:alert(1)".to_owned(),
            "data:text/html,<script>alert(1)</script>".to_owned(),
            "ftp://example.com".to_owned(),
            "//example.com".to_owned(),
            String::new(),
            format!("https://{}", "a".repeat(2041)), // 2049 字符，超过 2048 上限
        ];
        for url in invalid_urls {
            let response = app
                .admin_post(
                    "/api/admin/links",
                    serde_json::json!({ "title": "Url Case", "url": url }),
                    Some(&owner_cookie),
                )
                .await?;
            ensure!(
                response.status == StatusCode::BAD_REQUEST,
                "{}",
                response.body
            );
            ensure!(response.body["error"]["code"] == "INVALID_LINK_URL");
        }

        // 未知 status → 400。
        let bad_status = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({
                    "title": "Status Case",
                    "url": "https://example.com",
                    "status": "enabled",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            bad_status.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_status.body
        );
        ensure!(bad_status.body["error"]["code"] == "INVALID_LINK_STATUS");

        // 引用不存在的分类：外键违规映射为 404。
        let missing_category = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({
                    "category_id": 999_999_i64,
                    "title": "Orphan",
                    "url": "https://example.com",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            missing_category.status == StatusCode::NOT_FOUND,
            "{}",
            missing_category.body
        );
        ensure!(missing_category.body["error"]["code"] == "LINK_NOT_FOUND");

        // 合法边界值均可创建：100 字符标题、2048 字符 URL、大写 scheme。
        let boundary = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({
                    "title": "t".repeat(100),
                    "url": format!("HTTPS://{}", "a".repeat(2040)),
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(boundary.status == StatusCode::CREATED, "{}", boundary.body);
        let link_id = boundary.body["id"].as_i64().context("missing link id")?;

        // 更新路径执行同样的 URL 与 status 校验。
        let bad_update = app
            .admin_put(
                &format!("/api/admin/links/{link_id}"),
                serde_json::json!({ "title": "Updated", "url": "javascript:alert(1)" }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            bad_update.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_update.body
        );
        ensure!(bad_update.body["error"]["code"] == "INVALID_LINK_URL");

        let bad_status_update = app
            .admin_put(
                &format!("/api/admin/links/{link_id}"),
                serde_json::json!({
                    "title": "Updated",
                    "url": "https://example.com",
                    "status": "ENABLED",
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            bad_status_update.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_status_update.body
        );
        ensure!(bad_status_update.body["error"]["code"] == "INVALID_LINK_STATUS");

        // 更新引用不存在的分类 → 404。
        let orphan_update = app
            .admin_put(
                &format!("/api/admin/links/{link_id}"),
                serde_json::json!({
                    "category_id": 999_999_i64,
                    "title": "Updated",
                    "url": "https://example.com",
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            orphan_update.status == StatusCode::NOT_FOUND,
            "{}",
            orphan_update.body
        );
        ensure!(orphan_update.body["error"]["code"] == "LINK_NOT_FOUND");

        // 全部非法输入均未落库：列表仅剩 1 条合法友链。
        let list = app.admin_get("/api/admin/links", &owner_cookie).await?;
        ensure!(list.body["total"].as_i64() == Some(1), "{}", list.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn link_get_update_delete_not_found() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 不存在的 ID：GET / PUT / DELETE 均 → 404。
        let missing = app
            .admin_get("/api/admin/links/999999", &owner_cookie)
            .await?;
        ensure!(missing.status == StatusCode::NOT_FOUND, "{}", missing.body);
        ensure!(missing.body["error"]["code"] == "LINK_NOT_FOUND");

        let update_missing = app
            .admin_put(
                "/api/admin/links/999999",
                serde_json::json!({ "title": "Ghost", "url": "https://ghost.example.com" }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            update_missing.status == StatusCode::NOT_FOUND,
            "{}",
            update_missing.body
        );

        let delete_missing = app
            .admin_delete("/api/admin/links/999999", &owner_cookie)
            .await?;
        ensure!(
            delete_missing.status == StatusCode::NOT_FOUND,
            "{}",
            delete_missing.body
        );

        // 软删除后所有读写路径均视为不存在，重复删除也不是幂等 204。
        let created = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({ "title": "Temp", "url": "https://temp.example.com" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        let link_id = created.body["id"].as_i64().context("missing link id")?;

        let deleted = app
            .admin_delete(&format!("/api/admin/links/{link_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);

        let get_deleted = app
            .admin_get(&format!("/api/admin/links/{link_id}"), &owner_cookie)
            .await?;
        ensure!(get_deleted.status == StatusCode::NOT_FOUND);

        let update_deleted = app
            .admin_put(
                &format!("/api/admin/links/{link_id}"),
                serde_json::json!({ "title": "Temp", "url": "https://temp.example.com" }),
                &owner_cookie,
            )
            .await?;
        ensure!(update_deleted.status == StatusCode::NOT_FOUND);

        let delete_again = app
            .admin_delete(&format!("/api/admin/links/{link_id}"), &owner_cookie)
            .await?;
        ensure!(delete_again.status == StatusCode::NOT_FOUND);

        // 软删除友链不再出现在 Admin 列表与 Public 端点。
        let list = app.admin_get("/api/admin/links", &owner_cookie).await?;
        ensure!(list.body["total"].as_i64() == Some(0), "{}", list.body);
        let public = app.get("/api/public/links").await?;
        ensure!(public.body.as_array().map(Vec::len) == Some(0));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn link_list_pagination_and_filters() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 两个分类 + 五条友链，覆盖排序、状态、分类、关键字维度。
        let category_one = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Cat One", "slug": "cat-one" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            category_one.status == StatusCode::CREATED,
            "{}",
            category_one.body
        );
        let category_one_id = category_one.body["id"]
            .as_i64()
            .context("missing category id")?;
        let category_two = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Cat Two", "slug": "cat-two" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            category_two.status == StatusCode::CREATED,
            "{}",
            category_two.body
        );
        let category_two_id = category_two.body["id"]
            .as_i64()
            .context("missing category id")?;

        // 按创建顺序 id 递增；排序规则为 sort_order, id 升序。
        let fixtures = [
            serde_json::json!({
                "category_id": category_one_id,
                "title": "Alpha Blog",
                "url": "https://alpha.example.com",
                "sort_order": 2,
            }),
            serde_json::json!({
                "category_id": category_one_id,
                "title": "Beta Site",
                "url": "https://beta.example.com",
                "sort_order": 1,
            }),
            serde_json::json!({
                "category_id": category_two_id,
                "title": "Gamma",
                "url": "https://gamma.example.org",
                "status": "inactive",
                "sort_order": 3,
            }),
            serde_json::json!({
                "title": "Delta",
                "url": "https://delta.example.com",
                "sort_order": 1,
            }),
            serde_json::json!({
                "title": "Epsilon Alpha",
                "url": "https://epsilon.example.com",
                "sort_order": 2,
            }),
        ];
        for fixture in fixtures {
            let created = app
                .admin_post("/api/admin/links", fixture, Some(&owner_cookie))
                .await?;
            ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        }

        // 期望排序：sort_order 升序，并列时按 id 升序。
        let titles_of = |body: &serde_json::Value| -> Vec<String> {
            body["items"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item["title"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };

        // 默认分页：page=1，page_size=20。
        let all = app.admin_get("/api/admin/links", &owner_cookie).await?;
        ensure!(all.status == StatusCode::OK, "{}", all.body);
        ensure!(all.body["total"].as_i64() == Some(5), "{}", all.body);
        ensure!(all.body["page"].as_u64() == Some(1));
        ensure!(all.body["page_size"].as_u64() == Some(20));
        ensure!(
            titles_of(&all.body) == ["Beta Site", "Delta", "Alpha Blog", "Epsilon Alpha", "Gamma"],
            "{}",
            all.body
        );

        // 分页切片：第二页 2 条、第三页 1 条，total 不受分页影响。
        let page_two = app
            .admin_get("/api/admin/links?page=2&page_size=2", &owner_cookie)
            .await?;
        ensure!(page_two.body["page"].as_u64() == Some(2));
        ensure!(page_two.body["page_size"].as_u64() == Some(2));
        ensure!(page_two.body["total"].as_i64() == Some(5));
        ensure!(titles_of(&page_two.body) == ["Alpha Blog", "Epsilon Alpha"]);
        let page_three = app
            .admin_get("/api/admin/links?page=3&page_size=2", &owner_cookie)
            .await?;
        ensure!(titles_of(&page_three.body) == ["Gamma"]);

        // page/page_size 越界钳制：page 最小 1，page_size 最大 100。
        let clamped = app
            .admin_get("/api/admin/links?page=0&page_size=1000", &owner_cookie)
            .await?;
        ensure!(clamped.body["page"].as_u64() == Some(1));
        ensure!(clamped.body["page_size"].as_u64() == Some(100));
        ensure!(clamped.body["total"].as_i64() == Some(5));

        // status 过滤；非法值 → 400。
        let active = app
            .admin_get("/api/admin/links?status=active", &owner_cookie)
            .await?;
        ensure!(active.body["total"].as_i64() == Some(4));
        let inactive = app
            .admin_get("/api/admin/links?status=inactive", &owner_cookie)
            .await?;
        ensure!(inactive.body["total"].as_i64() == Some(1));
        ensure!(titles_of(&inactive.body) == ["Gamma"]);
        let bogus_status = app
            .admin_get("/api/admin/links?status=bogus", &owner_cookie)
            .await?;
        ensure!(
            bogus_status.status == StatusCode::BAD_REQUEST,
            "{}",
            bogus_status.body
        );
        ensure!(bogus_status.body["error"]["code"] == "INVALID_LINK_STATUS");

        // category_id 过滤。
        let by_category = app
            .admin_get(
                &format!("/api/admin/links?category_id={category_one_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(by_category.body["total"].as_i64() == Some(2));
        ensure!(titles_of(&by_category.body) == ["Beta Site", "Alpha Blog"]);

        // keyword：ILIKE 匹配 title 与 url，大小写不敏感。
        let by_title = app
            .admin_get("/api/admin/links?keyword=alpha", &owner_cookie)
            .await?;
        ensure!(by_title.body["total"].as_i64() == Some(2));
        ensure!(titles_of(&by_title.body) == ["Alpha Blog", "Epsilon Alpha"]);
        // inactive 友链仍可在 Admin 列表中按关键字检索到。
        let by_url = app
            .admin_get("/api/admin/links?keyword=EXAMPLE.ORG", &owner_cookie)
            .await?;
        ensure!(by_url.body["total"].as_i64() == Some(1));
        ensure!(titles_of(&by_url.body) == ["Gamma"]);
        // 纯空白 keyword 视为未提供。
        let blank_keyword = app
            .admin_get("/api/admin/links?keyword=%20%20%20", &owner_cookie)
            .await?;
        ensure!(blank_keyword.body["total"].as_i64() == Some(5));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn link_category_crud_validation_and_kind_isolation() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // slug 省略时由 name 派生并规范化。
        let created = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "My Blogs" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        ensure!(created.body["slug"] == "my-blogs");
        ensure!(created.body["kind"] == "link");
        let blogs_id = created.body["id"].as_i64().context("missing category id")?;

        // 同 kind 重复 slug → 409；citext 列大小写不敏感。
        let duplicate = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "My Blogs" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            duplicate.status == StatusCode::CONFLICT,
            "{}",
            duplicate.body
        );
        ensure!(duplicate.body["error"]["code"] == "TAXONOMY_CONFLICT");
        let duplicate_case = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Other", "slug": "MY-BLOGS" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            duplicate_case.status == StatusCode::CONFLICT,
            "{}",
            duplicate_case.body
        );

        // 非法名称（空、纯空白、超长）→ 400。
        for name in ["".to_owned(), "   ".to_owned(), "n".repeat(101)] {
            let response = app
                .admin_post(
                    "/api/admin/links/categories",
                    serde_json::json!({ "name": name }),
                    Some(&owner_cookie),
                )
                .await?;
            ensure!(
                response.status == StatusCode::BAD_REQUEST,
                "name={name:?} {}",
                response.body
            );
            ensure!(response.body["error"]["code"] == "INVALID_TAXONOMY_NAME");
        }

        // 更新：改名改 slug；与不存在的 ID → 404；占用他人 slug → 409。
        let tools = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Tools", "slug": "tools" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(tools.status == StatusCode::CREATED, "{}", tools.body);
        let tools_id = tools.body["id"].as_i64().context("missing category id")?;

        let updated = app
            .admin_put(
                &format!("/api/admin/links/categories/{blogs_id}"),
                serde_json::json!({ "name": "Renamed Blogs", "slug": "renamed-blogs" }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["name"] == "Renamed Blogs");
        ensure!(updated.body["slug"] == "renamed-blogs");

        let update_conflict = app
            .admin_put(
                &format!("/api/admin/links/categories/{blogs_id}"),
                serde_json::json!({ "name": "Copycat", "slug": "tools" }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            update_conflict.status == StatusCode::CONFLICT,
            "{}",
            update_conflict.body
        );
        ensure!(update_conflict.body["error"]["code"] == "TAXONOMY_CONFLICT");

        let update_missing = app
            .admin_put(
                "/api/admin/links/categories/999999",
                serde_json::json!({ "name": "Ghost" }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            update_missing.status == StatusCode::NOT_FOUND,
            "{}",
            update_missing.body
        );
        ensure!(update_missing.body["error"]["code"] == "TAXONOMY_NOT_FOUND");
        let delete_missing = app
            .admin_delete("/api/admin/links/categories/999999", &owner_cookie)
            .await?;
        ensure!(delete_missing.status == StatusCode::NOT_FOUND);

        // 引用保护：details 携带引用计数。
        let link = app
            .admin_post(
                "/api/admin/links",
                serde_json::json!({
                    "category_id": tools_id,
                    "title": "Tool Link",
                    "url": "https://tool.example.com",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(link.status == StatusCode::CREATED, "{}", link.body);
        let link_id = link.body["id"].as_i64().context("missing link id")?;
        let referenced = app
            .admin_delete(
                &format!("/api/admin/links/categories/{tools_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(
            referenced.status == StatusCode::CONFLICT,
            "{}",
            referenced.body
        );
        ensure!(referenced.body["error"]["code"] == "TAXONOMY_IN_USE");
        ensure!(
            referenced.body["error"]["details"]["reference_count"].as_i64() == Some(1),
            "{}",
            referenced.body
        );
        let link_deleted = app
            .admin_delete(&format!("/api/admin/links/{link_id}"), &owner_cookie)
            .await?;
        ensure!(link_deleted.status == StatusCode::NO_CONTENT);
        let tools_deleted = app
            .admin_delete(
                &format!("/api/admin/links/categories/{tools_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(tools_deleted.status == StatusCode::NO_CONTENT);

        // Kind 隔离：article 分类与 link 分类可同 slug（UNIQUE (kind, slug)）。
        let article_category = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Article Shared", "slug": "shared" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            article_category.status == StatusCode::CREATED,
            "{}",
            article_category.body
        );
        ensure!(article_category.body["kind"] == "article");
        let article_category_id = article_category.body["id"]
            .as_i64()
            .context("missing category id")?;
        let link_shared = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Link Shared", "slug": "shared" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            link_shared.status == StatusCode::CREATED,
            "{}",
            link_shared.body
        );

        // link 分类列表不回 article 分类，反之亦然。
        let link_categories = app
            .admin_get("/api/admin/links/categories", &owner_cookie)
            .await?;
        let link_category_list = link_categories
            .body
            .as_array()
            .context("missing categories")?;
        ensure!(link_category_list.iter().all(|item| item["kind"] == "link"));
        ensure!(
            !link_category_list
                .iter()
                .any(|item| item["id"].as_i64() == Some(article_category_id))
        );
        let article_categories = app
            .admin_get("/api/admin/categories", &owner_cookie)
            .await?;
        let article_category_list = article_categories
            .body
            .as_array()
            .context("missing categories")?;
        ensure!(
            article_category_list
                .iter()
                .all(|item| item["kind"] == "article")
        );

        // 用 link 分类端点操作 article 分类 → 404（kind 不匹配视为不存在），且原分类不受影响。
        let wrong_kind_update = app
            .admin_put(
                &format!("/api/admin/links/categories/{article_category_id}"),
                serde_json::json!({ "name": "Hijack" }),
                &owner_cookie,
            )
            .await?;
        ensure!(wrong_kind_update.status == StatusCode::NOT_FOUND);
        let wrong_kind_delete = app
            .admin_delete(
                &format!("/api/admin/links/categories/{article_category_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(wrong_kind_delete.status == StatusCode::NOT_FOUND);
        let article_after = app
            .admin_get("/api/admin/categories", &owner_cookie)
            .await?;
        ensure!(article_after.body.as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["id"].as_i64() == Some(article_category_id) && item["name"] == "Article Shared"
            })
        }));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn link_category_and_write_permission_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-c", "editor", "editor-pass-2").await?;
        create_user(&app, "moderator-c", "moderator", "moderator-pass-2").await?;
        let editor_cookie = login_cookie(&app, "editor-c", "editor-pass-2").await?;
        let moderator_cookie = login_cookie(&app, "moderator-c", "moderator-pass-2").await?;

        // 未认证：读端点与写端点均 → 401。
        let anon_list = app.get("/api/admin/links/categories").await?;
        ensure!(anon_list.status == StatusCode::UNAUTHORIZED);
        let anon_create = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Anon" }),
                None,
            )
            .await?;
        ensure!(anon_create.status == StatusCode::UNAUTHORIZED);
        let anon_update = app
            .admin_put(
                "/api/admin/links/1",
                serde_json::json!({ "title": "Anon", "url": "https://anon.example.com" }),
                "",
            )
            .await?;
        ensure!(anon_update.status == StatusCode::UNAUTHORIZED);
        let anon_delete = app.admin_delete("/api/admin/links/1", "").await?;
        ensure!(anon_delete.status == StatusCode::UNAUTHORIZED);

        // moderator 无 ManageContent：分类读写与友链写入均 → 403。
        let moderator_list = app
            .admin_get("/api/admin/links/categories", &moderator_cookie)
            .await?;
        ensure!(moderator_list.status == StatusCode::FORBIDDEN);
        let moderator_create = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Moderator Cat" }),
                Some(&moderator_cookie),
            )
            .await?;
        ensure!(moderator_create.status == StatusCode::FORBIDDEN);
        let moderator_update = app
            .admin_put(
                "/api/admin/links/1",
                serde_json::json!({ "title": "Moderator", "url": "https://mod.example.com" }),
                &moderator_cookie,
            )
            .await?;
        ensure!(moderator_update.status == StatusCode::FORBIDDEN);
        let moderator_delete = app
            .admin_delete("/api/admin/links/categories/1", &moderator_cookie)
            .await?;
        ensure!(moderator_delete.status == StatusCode::FORBIDDEN);

        // editor 具备 ManageContent：分类 CRUD 可用。
        let created = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Editor Cat", "slug": "editor-cat" }),
                Some(&editor_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        let category_id = created.body["id"].as_i64().context("missing category id")?;
        let updated = app
            .admin_put(
                &format!("/api/admin/links/categories/{category_id}"),
                serde_json::json!({ "name": "Editor Cat V2", "slug": "editor-cat-v2" }),
                &editor_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        let deleted = app
            .admin_delete(
                &format!("/api/admin/links/categories/{category_id}"),
                &editor_cookie,
            )
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
