//! Phase 02 Contract Test：Taxonomy（分类/标签）Admin CRUD + kind 隔离 + 删除引用保护。
//!
//! 管理端 `/api/admin/categories` 固定 kind = article；link/gallery 分类由各自模块管理，
//! 本文件通过跨模块创建验证 kind 过滤与跨 kind 同 slug 共存。

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

/// 创建一篇引用指定分类/标签的 Draft 文章，返回 (id, version)。
async fn create_article(
    app: &TestApp,
    cookie: &str,
    title: &str,
    category_id: Option<i64>,
    tag_ids: &[i64],
) -> anyhow::Result<(i64, i64)> {
    let created = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({
                "title": title,
                "markdown_source": "# Reference",
                "category_id": category_id,
                "tag_ids": tag_ids,
            }),
            Some(cookie),
        )
        .await?;
    ensure!(created.status == StatusCode::CREATED, "{}", created.body);
    let id = created.body["id"].as_i64().context("missing article id")?;
    let version = created.body["version"]
        .as_i64()
        .context("missing article version")?;
    Ok((id, version))
}

/// 回收并删除文章，使 Taxonomy 引用计数归零（引用统计排除 deleted_at 非空的行）。
async fn recycle_and_delete_article(
    app: &TestApp,
    cookie: &str,
    id: i64,
    version: i64,
) -> anyhow::Result<()> {
    let recycled = app
        .admin_patch(
            &format!("/api/admin/articles/{id}/status"),
            serde_json::json!({ "command": "recycle", "expected_version": version }),
            cookie,
        )
        .await?;
    ensure!(
        recycled.status == StatusCode::OK,
        "recycle failed: {}",
        recycled.body
    );
    let deleted = app
        .admin_delete(&format!("/api/admin/articles/{id}"), cookie)
        .await?;
    ensure!(
        deleted.status == StatusCode::NO_CONTENT,
        "delete failed: {}",
        deleted.body
    );
    Ok(())
}

#[tokio::test]
async fn category_crud_conflict_and_validation() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 正常创建：响应携带 kind = article 与全部字段。
        let created = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Tech", "slug": "tech" }),
                Some(&cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        ensure!(created.body["kind"] == "article");
        ensure!(created.body["name"] == "Tech");
        ensure!(created.body["slug"] == "tech");
        ensure!(created.body["description"] == "");
        let category_id = created.body["id"].as_i64().context("missing id")?;

        // 省略 slug 时回退为 name 的归一化结果。
        let fallback = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "No Slug" }),
                Some(&cookie),
            )
            .await?;
        ensure!(fallback.status == StatusCode::CREATED, "{}", fallback.body);
        ensure!(fallback.body["slug"] == "no-slug");
        let fallback_id = fallback.body["id"].as_i64().context("missing id")?;

        // slug 冲突（同 kind 唯一）→ 409。
        let conflict = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Tech Again", "slug": "tech" }),
                Some(&cookie),
            )
            .await?;
        ensure!(conflict.status == StatusCode::CONFLICT, "{}", conflict.body);
        ensure!(conflict.body["error"]["code"] == "TAXONOMY_CONFLICT");

        // 非法 name（trim 后为空）→ 400。
        let bad_name = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "   ", "slug": "blank-name" }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            bad_name.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_name.body
        );
        ensure!(bad_name.body["error"]["code"] == "INVALID_TAXONOMY_NAME");

        // 非法 slug（归一化后为空）→ 400。
        let bad_slug = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Bad Slug", "slug": "!!!" }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            bad_slug.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_slug.body
        );
        ensure!(bad_slug.body["error"]["code"] == "INVALID_ARTICLE_SLUG");

        // 列表包含两个已创建分类。
        let list = app.admin_get("/api/admin/categories", &cookie).await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body.as_array().map(Vec::len) == Some(2));

        // 正常更新。
        let updated = app
            .admin_put(
                &format!("/api/admin/categories/{category_id}"),
                serde_json::json!({ "name": "Tech Renamed", "slug": "tech-renamed" }),
                &cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["name"] == "Tech Renamed");
        ensure!(updated.body["slug"] == "tech-renamed");

        // 更新为已占用的 slug → 409。
        let update_conflict = app
            .admin_put(
                &format!("/api/admin/categories/{category_id}"),
                serde_json::json!({ "name": "Tech Renamed", "slug": "no-slug" }),
                &cookie,
            )
            .await?;
        ensure!(
            update_conflict.status == StatusCode::CONFLICT,
            "{}",
            update_conflict.body
        );
        ensure!(update_conflict.body["error"]["code"] == "TAXONOMY_CONFLICT");

        // 更新/删除不存在的分类 → 404。
        let missing_update = app
            .admin_put(
                "/api/admin/categories/999999",
                serde_json::json!({ "name": "Ghost", "slug": "ghost" }),
                &cookie,
            )
            .await?;
        ensure!(
            missing_update.status == StatusCode::NOT_FOUND,
            "{}",
            missing_update.body
        );
        ensure!(missing_update.body["error"]["code"] == "TAXONOMY_NOT_FOUND");
        let missing_delete = app
            .admin_delete("/api/admin/categories/999999", &cookie)
            .await?;
        ensure!(
            missing_delete.status == StatusCode::NOT_FOUND,
            "{}",
            missing_delete.body
        );

        // 正常删除；重复删除 → 404（物理删除，行已不存在）。
        let deleted = app
            .admin_delete(&format!("/api/admin/categories/{category_id}"), &cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let deleted_again = app
            .admin_delete(&format!("/api/admin/categories/{category_id}"), &cookie)
            .await?;
        ensure!(
            deleted_again.status == StatusCode::NOT_FOUND,
            "{}",
            deleted_again.body
        );
        let deleted_fallback = app
            .admin_delete(&format!("/api/admin/categories/{fallback_id}"), &cookie)
            .await?;
        ensure!(
            deleted_fallback.status == StatusCode::NO_CONTENT,
            "{}",
            deleted_fallback.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn tag_crud_conflict_and_validation() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 正常创建。
        let created = app
            .admin_post(
                "/api/admin/tags",
                serde_json::json!({ "name": "Rust", "slug": "rust" }),
                Some(&cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        ensure!(created.body["name"] == "Rust");
        ensure!(created.body["slug"] == "rust");
        let tag_id = created.body["id"].as_i64().context("missing id")?;

        // slug 冲突 → 409。
        let conflict = app
            .admin_post(
                "/api/admin/tags",
                serde_json::json!({ "name": "Rust Again", "slug": "rust" }),
                Some(&cookie),
            )
            .await?;
        ensure!(conflict.status == StatusCode::CONFLICT, "{}", conflict.body);
        ensure!(conflict.body["error"]["code"] == "TAXONOMY_CONFLICT");

        // 非法 name → 400。
        let bad_name = app
            .admin_post(
                "/api/admin/tags",
                serde_json::json!({ "name": "", "slug": "blank-name" }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            bad_name.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_name.body
        );
        ensure!(bad_name.body["error"]["code"] == "INVALID_TAXONOMY_NAME");

        // 列表包含已创建标签。
        let list = app.admin_get("/api/admin/tags", &cookie).await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body.as_array().map(Vec::len) == Some(1));

        // 正常更新。
        let updated = app
            .admin_put(
                &format!("/api/admin/tags/{tag_id}"),
                serde_json::json!({ "name": "Rust Lang", "slug": "rust-lang" }),
                &cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["name"] == "Rust Lang");
        ensure!(updated.body["slug"] == "rust-lang");

        // 更新/删除不存在的标签 → 404。
        let missing_update = app
            .admin_put(
                "/api/admin/tags/999999",
                serde_json::json!({ "name": "Ghost", "slug": "ghost" }),
                &cookie,
            )
            .await?;
        ensure!(
            missing_update.status == StatusCode::NOT_FOUND,
            "{}",
            missing_update.body
        );
        ensure!(missing_update.body["error"]["code"] == "TAXONOMY_NOT_FOUND");
        let missing_delete = app.admin_delete("/api/admin/tags/999999", &cookie).await?;
        ensure!(
            missing_delete.status == StatusCode::NOT_FOUND,
            "{}",
            missing_delete.body
        );

        // 正常删除；重复删除 → 404。
        let deleted = app
            .admin_delete(&format!("/api/admin/tags/{tag_id}"), &cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let deleted_again = app
            .admin_delete(&format!("/api/admin/tags/{tag_id}"), &cookie)
            .await?;
        ensure!(
            deleted_again.status == StatusCode::NOT_FOUND,
            "{}",
            deleted_again.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn taxonomy_permission_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-t", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-t", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-t", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-t", "moderator-pass-1").await?;

        // 未认证 → 401（读与写一致）。
        let unauthenticated_list = app.get("/api/admin/categories").await?;
        ensure!(unauthenticated_list.status == StatusCode::UNAUTHORIZED);
        let unauthenticated_create = app
            .admin_post(
                "/api/admin/tags",
                serde_json::json!({ "name": "Anon", "slug": "anon" }),
                None,
            )
            .await?;
        ensure!(unauthenticated_create.status == StatusCode::UNAUTHORIZED);

        // moderator 无 ManageContent 权限 → 403（读与写一致）。
        let denied_list = app
            .admin_get("/api/admin/categories", &moderator_cookie)
            .await?;
        ensure!(
            denied_list.status == StatusCode::FORBIDDEN,
            "{}",
            denied_list.body
        );
        let denied_tags = app.admin_get("/api/admin/tags", &moderator_cookie).await?;
        ensure!(
            denied_tags.status == StatusCode::FORBIDDEN,
            "{}",
            denied_tags.body
        );
        let denied_create = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Denied", "slug": "denied" }),
                Some(&moderator_cookie),
            )
            .await?;
        ensure!(
            denied_create.status == StatusCode::FORBIDDEN,
            "{}",
            denied_create.body
        );

        // editor 拥有 ManageContent → 可创建。
        let editor_created = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Editor Category", "slug": "editor-category" }),
                Some(&editor_cookie),
            )
            .await?;
        ensure!(
            editor_created.status == StatusCode::CREATED,
            "{}",
            editor_created.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn category_kind_isolation() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 三套分类体系共用 (kind, slug) 唯一约束：跨 kind 允许同名 slug。
        let article_category = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Shared", "slug": "shared-kind" }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            article_category.status == StatusCode::CREATED,
            "{}",
            article_category.body
        );
        ensure!(article_category.body["kind"] == "article");
        let article_category_id = article_category.body["id"].as_i64().context("missing id")?;

        let link_category = app
            .admin_post(
                "/api/admin/links/categories",
                serde_json::json!({ "name": "Shared Link", "slug": "shared-kind" }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            link_category.status == StatusCode::CREATED,
            "{}",
            link_category.body
        );
        ensure!(link_category.body["kind"] == "link");
        let link_category_id = link_category.body["id"].as_i64().context("missing id")?;

        let gallery_category = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "Shared Gallery", "slug": "shared-kind" }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            gallery_category.status == StatusCode::CREATED,
            "{}",
            gallery_category.body
        );
        ensure!(gallery_category.body["kind"] == "gallery");

        // 管理端分类列表固定 article kind，不回 link/gallery 分类。
        let list = app.admin_get("/api/admin/categories", &cookie).await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        let categories = list.body.as_array().context("missing categories")?;
        ensure!(categories.len() == 1, "{}", list.body);
        ensure!(categories[0]["id"] == article_category_id);
        ensure!(categories[0]["kind"] == "article");

        // 更新/删除按 kind 隔离：link 分类 id 在 article 端点下不可见 → 404。
        let wrong_kind_update = app
            .admin_put(
                &format!("/api/admin/categories/{link_category_id}"),
                serde_json::json!({ "name": "Hijack", "slug": "hijack" }),
                &cookie,
            )
            .await?;
        ensure!(
            wrong_kind_update.status == StatusCode::NOT_FOUND,
            "{}",
            wrong_kind_update.body
        );
        let wrong_kind_delete = app
            .admin_delete(
                &format!("/api/admin/categories/{link_category_id}"),
                &cookie,
            )
            .await?;
        ensure!(
            wrong_kind_delete.status == StatusCode::NOT_FOUND,
            "{}",
            wrong_kind_delete.body
        );

        // 跨 kind 误操作未影响 link 分类本体。
        let link_categories = app
            .admin_get("/api/admin/links/categories", &cookie)
            .await?;
        let links = link_categories.body.as_array().context("missing links")?;
        ensure!(links.len() == 1, "{}", link_categories.body);
        ensure!(links[0]["name"] == "Shared Link");
        ensure!(links[0]["slug"] == "shared-kind");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn category_delete_reference_protection() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        let category = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Referenced", "slug": "referenced" }),
                Some(&cookie),
            )
            .await?;
        ensure!(category.status == StatusCode::CREATED, "{}", category.body);
        let category_id = category.body["id"].as_i64().context("missing id")?;

        // 一篇文章引用 → 删除 409，details 携带引用数 1。
        let (article_one, version_one) =
            create_article(&app, &cookie, "Post One", Some(category_id), &[]).await?;
        let referenced = app
            .admin_delete(&format!("/api/admin/categories/{category_id}"), &cookie)
            .await?;
        ensure!(
            referenced.status == StatusCode::CONFLICT,
            "{}",
            referenced.body
        );
        ensure!(referenced.body["error"]["code"] == "TAXONOMY_IN_USE");
        ensure!(
            referenced.body["error"]["details"]["reference_count"] == 1,
            "{}",
            referenced.body
        );

        // 两篇引用 → 引用数随之增长为 2。
        let (article_two, version_two) =
            create_article(&app, &cookie, "Post Two", Some(category_id), &[]).await?;
        let referenced_two = app
            .admin_delete(&format!("/api/admin/categories/{category_id}"), &cookie)
            .await?;
        ensure!(
            referenced_two.body["error"]["details"]["reference_count"] == 2,
            "{}",
            referenced_two.body
        );

        // 回收并删除全部引用文章后分类可删除。
        recycle_and_delete_article(&app, &cookie, article_one, version_one).await?;
        recycle_and_delete_article(&app, &cookie, article_two, version_two).await?;
        let deleted = app
            .admin_delete(&format!("/api/admin/categories/{category_id}"), &cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn tag_delete_reference_protection() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        let tag = app
            .admin_post(
                "/api/admin/tags",
                serde_json::json!({ "name": "Referenced Tag", "slug": "referenced-tag" }),
                Some(&cookie),
            )
            .await?;
        ensure!(tag.status == StatusCode::CREATED, "{}", tag.body);
        let tag_id = tag.body["id"].as_i64().context("missing id")?;

        // 文章关联标签 → 删除 409，details 携带引用数 1。
        let (article_id, version) =
            create_article(&app, &cookie, "Tagged Post", None, &[tag_id]).await?;
        let referenced = app
            .admin_delete(&format!("/api/admin/tags/{tag_id}"), &cookie)
            .await?;
        ensure!(
            referenced.status == StatusCode::CONFLICT,
            "{}",
            referenced.body
        );
        ensure!(referenced.body["error"]["code"] == "TAXONOMY_IN_USE");
        ensure!(
            referenced.body["error"]["details"]["reference_count"] == 1,
            "{}",
            referenced.body
        );

        // 回收并删除引用文章后标签可删除。
        recycle_and_delete_article(&app, &cookie, article_id, version).await?;
        let deleted = app
            .admin_delete(&format!("/api/admin/tags/{tag_id}"), &cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
