//! `/api/public/articles` Contract Test：匿名只读、仅暴露 Published、字段裁剪与 Sanitization。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

async fn create_article(
    app: &TestApp,
    cookie: &str,
    body: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let response = app
        .admin_post("/api/admin/articles", body, Some(cookie))
        .await?;
    ensure!(
        response.status == StatusCode::CREATED,
        "create article failed: {}",
        response.body
    );
    Ok(response.body)
}

async fn publish_article(
    app: &TestApp,
    cookie: &str,
    article: &serde_json::Value,
) -> anyhow::Result<()> {
    let id = article["id"].as_i64().context("missing id")?;
    let version = article["version"].as_i64().context("missing version")?;
    let response = app
        .admin_patch(
            &format!("/api/admin/articles/{id}/status"),
            serde_json::json!({ "command": "publish", "expected_version": version }),
            cookie,
        )
        .await?;
    ensure!(
        response.status == StatusCode::OK,
        "publish failed: {}",
        response.body
    );
    Ok(())
}

#[tokio::test]
async fn public_list_only_exposes_published_articles() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario_result = list_visibility_scenario(&app).await;
    let cleanup_result = app.cleanup().await;
    scenario_result?;
    cleanup_result
}

async fn list_visibility_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let draft = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Draft Post", "markdown_source": "# Draft" }),
    )
    .await?;
    let published = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Published Post", "markdown_source": "# Published" }),
    )
    .await?;
    publish_article(app, &cookie, &published).await?;

    // Public GET 不带 Origin 与 Cookie，验证无 CSRF 层、无 Auth 要求。
    let list = app.get("/api/public/articles").await?;
    ensure!(list.status == StatusCode::OK, "list failed: {}", list.body);
    ensure!(list.body["total"] == 1, "unexpected total: {}", list.body);
    let items = list.body["items"].as_array().context("items not array")?;
    ensure!(items.len() == 1);
    ensure!(items[0]["slug"] == published["slug"]);
    ensure!(items[0]["slug"] != draft["slug"]);

    // 非法 sort/order 静默回退，不报错。
    let fallback = app
        .get("/api/public/articles?sort=bogus&order=sideways")
        .await?;
    ensure!(fallback.status == StatusCode::OK);
    ensure!(fallback.body["total"] == 1);

    // keyword 过滤命中 title。
    let hit = app.get("/api/public/articles?keyword=Published").await?;
    ensure!(hit.body["total"] == 1);
    let miss = app
        .get("/api/public/articles?keyword=nonexistent-term")
        .await?;
    ensure!(miss.body["total"] == 0);

    // LIKE 通配符字面量化（ILIKE ESCAPE '\'）：`%` 不再作为通配符匹配全部，
    // 只命中标题中确实含字面量 % 的文章。
    let percent_post = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Sale 50% Off", "markdown_source": "# Sale" }),
    )
    .await?;
    publish_article(app, &cookie, &percent_post).await?;
    let percent = app.get("/api/public/articles?keyword=%25").await?;
    ensure!(
        percent.body["total"] == 1,
        "keyword '%' must match literally: {}",
        percent.body
    );
    ensure!(percent.body["items"][0]["slug"] == percent_post["slug"]);
    let percent_phrase = app.get("/api/public/articles?keyword=50%25%20Off").await?;
    ensure!(
        percent_phrase.body["total"] == 1,
        "keyword with '%' inside must match literally: {}",
        percent_phrase.body
    );
    Ok(())
}

#[tokio::test]
async fn admin_article_list_keyword_escapes_like_wildcards() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario_result = admin_list_keyword_scenario(&app).await;
    let cleanup_result = app.cleanup().await;
    scenario_result?;
    cleanup_result
}

async fn admin_list_keyword_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Normal Post", "markdown_source": "# Normal" }),
    )
    .await?;
    let percent = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Progress 100% Done", "markdown_source": "# Progress" }),
    )
    .await?;

    // 普通子串匹配不受转义影响。
    let phrase = app
        .admin_get("/api/admin/articles?keyword=Progress", &cookie)
        .await?;
    ensure!(phrase.body["total"] == 1, "{}", phrase.body);
    ensure!(phrase.body["items"][0]["id"] == percent["id"]);

    // LIKE 通配符字面量化（ILIKE ESCAPE '\'）：`%` 不再作为通配符匹配全部，
    // 只命中标题中确实含字面量 % 的文章。
    let percent_only = app
        .admin_get("/api/admin/articles?keyword=%25", &cookie)
        .await?;
    ensure!(
        percent_only.body["total"] == 1,
        "keyword '%' must match literally: {}",
        percent_only.body
    );
    ensure!(percent_only.body["items"][0]["id"] == percent["id"]);
    let percent_phrase = app
        .admin_get("/api/admin/articles?keyword=100%25%20Done", &cookie)
        .await?;
    ensure!(
        percent_phrase.body["total"] == 1,
        "keyword with '%' inside must match literally: {}",
        percent_phrase.body
    );
    Ok(())
}

#[tokio::test]
async fn public_list_filters_by_category_and_tag() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario_result = taxonomy_filter_scenario(&app).await;
    let cleanup_result = app.cleanup().await;
    scenario_result?;
    cleanup_result
}

async fn taxonomy_filter_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let category = app
        .admin_post(
            "/api/admin/categories",
            serde_json::json!({ "name": "Engineering" }),
            Some(&cookie),
        )
        .await?;
    ensure!(category.status == StatusCode::CREATED, "{}", category.body);
    let category_id = category.body["id"]
        .as_i64()
        .context("missing category id")?;
    let tag = app
        .admin_post(
            "/api/admin/tags",
            serde_json::json!({ "name": "Rust" }),
            Some(&cookie),
        )
        .await?;
    ensure!(tag.status == StatusCode::CREATED, "{}", tag.body);
    let tag_id = tag.body["id"].as_i64().context("missing tag id")?;

    let tagged = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "Tagged Post",
            "markdown_source": "# Tagged",
            "category_id": category_id,
            "tag_ids": [tag_id],
        }),
    )
    .await?;
    publish_article(app, &cookie, &tagged).await?;
    let plain = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Plain Post", "markdown_source": "# Plain" }),
    )
    .await?;
    publish_article(app, &cookie, &plain).await?;

    let by_category = app
        .get(&format!("/api/public/articles?category_id={category_id}"))
        .await?;
    ensure!(by_category.body["total"] == 1, "{}", by_category.body);
    ensure!(by_category.body["items"][0]["slug"] == tagged["slug"]);
    let by_tag = app
        .get(&format!("/api/public/articles?tag_id={tag_id}"))
        .await?;
    ensure!(by_tag.body["total"] == 1, "{}", by_tag.body);
    ensure!(by_tag.body["items"][0]["slug"] == tagged["slug"]);
    let by_unknown_category = app.get("/api/public/articles?category_id=999999").await?;
    ensure!(by_unknown_category.body["total"] == 0);
    Ok(())
}

#[tokio::test]
async fn public_detail_contract_and_not_found_semantics() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario_result = detail_scenario(&app).await;
    let cleanup_result = app.cleanup().await;
    scenario_result?;
    cleanup_result
}

async fn detail_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let draft = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Draft Detail", "markdown_source": "# Draft Detail" }),
    )
    .await?;
    let article = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Detail Post", "markdown_source": "# Detail" }),
    )
    .await?;
    publish_article(app, &cookie, &article).await?;
    let slug = article["slug"].as_str().context("missing slug")?;

    let detail = app.get(&format!("/api/public/articles/{slug}")).await?;
    ensure!(detail.status == StatusCode::OK, "{}", detail.body);
    ensure!(detail.body["slug"] == slug);
    ensure!(detail.body["rendered_html"].is_string());
    // Public Detail 不得泄露内部字段。
    for key in ["markdown_source", "author_id", "version"] {
        ensure!(
            detail.body.get(key).is_none(),
            "public detail must not contain {key}"
        );
    }
    // slug 为 citext，大小写不敏感。
    let upper = app
        .get(&format!("/api/public/articles/{}", slug.to_uppercase()))
        .await?;
    ensure!(upper.status == StatusCode::OK);

    // Draft、不存在、Recycled 的 Slug 一律 404 ARTICLE_NOT_FOUND。
    let draft_slug = draft["slug"].as_str().context("missing draft slug")?;
    for target in [draft_slug.to_owned(), "no-such-article".to_owned()] {
        let missing = app.get(&format!("/api/public/articles/{target}")).await?;
        ensure!(missing.status == StatusCode::NOT_FOUND, "{}", missing.body);
        ensure!(missing.body["error"]["code"] == "ARTICLE_NOT_FOUND");
    }
    let article_id = article["id"].as_i64().context("missing id")?;
    let recycled = app
        .admin_patch(
            &format!("/api/admin/articles/{article_id}/status"),
            serde_json::json!({ "command": "recycle", "expected_version": 2 }),
            &cookie,
        )
        .await?;
    ensure!(recycled.status == StatusCode::OK, "{}", recycled.body);
    let gone = app.get(&format!("/api/public/articles/{slug}")).await?;
    ensure!(gone.status == StatusCode::NOT_FOUND);
    ensure!(gone.body["error"]["code"] == "ARTICLE_NOT_FOUND");
    Ok(())
}

#[tokio::test]
async fn password_protected_article_hides_rendered_html() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario_result = password_scenario(&app).await;
    let cleanup_result = app.cleanup().await;
    scenario_result?;
    cleanup_result
}

async fn password_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let article = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "Secret Post",
            "markdown_source": "# Secret",
            "access_password": "reader-pass-1",
        }),
    )
    .await?;
    publish_article(app, &cookie, &article).await?;
    let slug = article["slug"].as_str().context("missing slug")?;

    let detail = app.get(&format!("/api/public/articles/{slug}")).await?;
    ensure!(detail.status == StatusCode::OK, "{}", detail.body);
    ensure!(detail.body["password_protected"] == true);
    ensure!(detail.body["rendered_html"].is_null());

    let list = app.get("/api/public/articles").await?;
    ensure!(list.body["items"][0]["password_protected"] == true);
    Ok(())
}

#[tokio::test]
async fn public_list_out_of_range_page_returns_404() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario_result = out_of_range_scenario(&app).await;
    let cleanup_result = app.cleanup().await;
    scenario_result?;
    cleanup_result
}

async fn out_of_range_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let article = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Paged Post", "markdown_source": "# Paged" }),
    )
    .await?;
    publish_article(app, &cookie, &article).await?;

    // page=1 始终 200；page > 1 且结果为空 → 404 PAGE_OUT_OF_RANGE。
    let first = app.get("/api/public/articles?page=1").await?;
    ensure!(first.status == StatusCode::OK, "{}", first.body);
    let overflow = app.get("/api/public/articles?page=999").await?;
    ensure!(
        overflow.status == StatusCode::NOT_FOUND,
        "{}",
        overflow.body
    );
    ensure!(overflow.body["error"]["code"] == "PAGE_OUT_OF_RANGE");

    // 搜索结果为空的 page=1 保持 200（不是越界页）。
    let search_miss = app.get("/api/public/search?q=nonexistent-term").await?;
    ensure!(search_miss.status == StatusCode::OK, "{}", search_miss.body);
    ensure!(search_miss.body["total"] == 0);
    let search_overflow = app.get("/api/public/search?q=Paged&page=999").await?;
    ensure!(
        search_overflow.status == StatusCode::NOT_FOUND,
        "{}",
        search_overflow.body
    );

    // 分类 / 标签文章列表同样适用。
    app.admin_post(
        "/api/admin/categories",
        serde_json::json!({ "name": "Paging", "slug": "paging" }),
        Some(&cookie),
    )
    .await?;
    let category_overflow = app
        .get("/api/public/categories/paging/articles?page=2")
        .await?;
    ensure!(
        category_overflow.status == StatusCode::NOT_FOUND,
        "{}",
        category_overflow.body
    );
    let tag_overflow = app.get("/api/public/tags/rust/articles?page=2").await?;
    ensure!(
        tag_overflow.status == StatusCode::NOT_FOUND,
        "{}",
        tag_overflow.body
    );

    // 评论列表：page=1 无评论保持 200，page=2 → 404。
    let slug = article["slug"].as_str().context("missing slug")?;
    let comments_first = app
        .get(&format!("/api/public/articles/{slug}/comments"))
        .await?;
    ensure!(
        comments_first.status == StatusCode::OK,
        "{}",
        comments_first.body
    );
    let comments_overflow = app
        .get(&format!("/api/public/articles/{slug}/comments?page=2"))
        .await?;
    ensure!(
        comments_overflow.status == StatusCode::NOT_FOUND,
        "{}",
        comments_overflow.body
    );
    Ok(())
}

#[tokio::test]
async fn public_detail_rendered_html_is_sanitized() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario_result = sanitization_scenario(&app).await;
    let cleanup_result = app.cleanup().await;
    scenario_result?;
    cleanup_result
}

async fn sanitization_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let article = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "XSS Post",
            "markdown_source": "# XSS\n\n<script>alert(1)</script>",
        }),
    )
    .await?;
    publish_article(app, &cookie, &article).await?;
    let slug = article["slug"].as_str().context("missing slug")?;

    let detail = app.get(&format!("/api/public/articles/{slug}")).await?;
    ensure!(detail.status == StatusCode::OK, "{}", detail.body);
    let html = detail.body["rendered_html"]
        .as_str()
        .context("rendered_html missing")?;
    ensure!(!html.contains("<script"), "script tag leaked: {html}");
    ensure!(!html.contains("alert(1)"), "script body leaked: {html}");
    Ok(())
}
