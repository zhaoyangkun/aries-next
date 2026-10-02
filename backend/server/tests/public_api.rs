//! Public API 集成测试：可见性、排序与导航、密码解锁、浏览去重、聚合端点、搜索与缓存头。

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

fn slug_of(article: &serde_json::Value) -> anyhow::Result<&str> {
    article["slug"].as_str().context("missing slug")
}

#[tokio::test]
async fn public_site_projects_only_safe_fields_with_cache_header() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = site_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn site_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let updated = app
        .admin_put(
            "/api/admin/site-settings",
            serde_json::json!({
                "site_name": "Aries 博客",
                "site_description": "记录 Rust 与生活",
                "site_url": "https://blog.example.com",
                "logo_url": "",
                "icp_text": "京ICP备00000000号",
                "default_cover_url": "",
                "page_size_index": 10,
                "page_size_archive": 10,
                "page_size_search": 10,
            }),
            &cookie,
        )
        .await?;
    ensure!(updated.status == StatusCode::OK, "{}", updated.body);

    let (status, headers, bytes) = app.get_raw("/api/public/site").await?;
    ensure!(status == StatusCode::OK);
    let cache = headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .context("missing cache-control")?;
    ensure!(
        cache.contains("stale-while-revalidate"),
        "unexpected: {cache}"
    );

    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(body["site_name"] == "Aries 博客");
    ensure!(body["icp_text"] == "京ICP备00000000号");
    // 公开投影不得出现分页设置或任何额外字段。
    let object = body.as_object().context("site must be an object")?;
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    ensure!(
        keys == [
            "created_at",
            "default_cover_url",
            "icp_text",
            "logo_url",
            "site_description",
            "site_name",
            "site_url"
        ],
        "unexpected public site fields: {keys:?}"
    );
    Ok(())
}

#[tokio::test]
async fn public_articles_visibility_ordering_and_neighbors() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = ordering_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn ordering_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let category = app
        .admin_post(
            "/api/admin/categories",
            serde_json::json!({ "name": "Engineering" }),
            Some(&cookie),
        )
        .await?;
    ensure!(category.status == StatusCode::CREATED, "{}", category.body);
    let category_id = category.body["id"].as_i64().context("category id")?;
    let category_slug = category.body["slug"]
        .as_str()
        .context("category slug")?
        .to_owned();
    let tag = app
        .admin_post(
            "/api/admin/tags",
            serde_json::json!({ "name": "Rust" }),
            Some(&cookie),
        )
        .await?;
    ensure!(tag.status == StatusCode::CREATED, "{}", tag.body);
    let tag_id = tag.body["id"].as_i64().context("tag id")?;
    let tag_slug = tag.body["slug"].as_str().context("tag slug")?.to_owned();

    let oldest = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "Oldest",
            "markdown_source": "# Oldest",
            "category_id": category_id,
            "tag_ids": [tag_id],
        }),
    )
    .await?;
    publish_article(app, &cookie, &oldest).await?;
    let middle = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Middle", "markdown_source": "# Middle" }),
    )
    .await?;
    publish_article(app, &cookie, &middle).await?;
    let pinned = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Pinned", "markdown_source": "# Pinned", "is_pinned": true }),
    )
    .await?;
    publish_article(app, &cookie, &pinned).await?;
    // Draft 与 Recycled 不得出现在任何公开端点。
    let draft = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "Hidden Draft",
            "markdown_source": "# Hidden",
            "category_id": category_id,
            "tag_ids": [tag_id],
        }),
    )
    .await?;

    // 列表：置顶优先，其余按发布时间倒序。
    let list = app.get("/api/public/articles").await?;
    ensure!(list.status == StatusCode::OK, "{}", list.body);
    ensure!(
        list.body["total"] == 3,
        "draft must be invisible: {}",
        list.body
    );
    let items = list.body["items"].as_array().context("items")?;
    let order: Vec<&str> = items
        .iter()
        .filter_map(|item| item["slug"].as_str())
        .collect();
    ensure!(
        order == [slug_of(&pinned)?, slug_of(&middle)?, slug_of(&oldest)?],
        "unexpected order: {order:?}"
    );

    // Slug 过滤：category/tag 参数与 ID 参数行为一致。
    let by_slug = app
        .get(&format!("/api/public/articles?category={category_slug}"))
        .await?;
    ensure!(by_slug.body["total"] == 1, "{}", by_slug.body);
    ensure!(by_slug.body["items"][0]["slug"] == slug_of(&oldest)?);
    let by_tag = app
        .get(&format!("/api/public/articles?tag={tag_slug}"))
        .await?;
    ensure!(by_tag.body["total"] == 1, "{}", by_tag.body);

    // 分页总数语义：page_size=1 时 total 仍为 3。
    let paged = app.get("/api/public/articles?page=2&page_size=1").await?;
    ensure!(
        paged.body["total"] == 3 && paged.body["items"].as_array().is_some_and(|i| i.len() == 1)
    );
    ensure!(paged.body["items"][0]["slug"] == slug_of(&middle)?);

    // 详情：category/tags 投影、visit/comment 计数、无内部字段。
    let (status, headers, bytes) = app
        .get_raw(&format!("/api/public/articles/{}", slug_of(&middle)?))
        .await?;
    ensure!(status == StatusCode::OK);
    let cache = headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .context("missing cache-control")?;
    ensure!(cache == "public, max-age=60", "unexpected: {cache}");
    let detail: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(detail["visit_count"] == 0 && detail["comment_count"] == 0);
    ensure!(detail["previous"]["slug"] == slug_of(&pinned)?);
    ensure!(detail["next"]["slug"] == slug_of(&oldest)?);
    for key in ["markdown_source", "author_id", "version"] {
        ensure!(detail.get(key).is_none(), "detail must not contain {key}");
    }

    // 邻居边界：第一篇 previous 为 null，最后一篇 next 为 null。
    let first = app
        .get(&format!("/api/public/articles/{}", slug_of(&pinned)?))
        .await?;
    ensure!(first.body["previous"].is_null());
    ensure!(first.body["next"]["slug"] == slug_of(&middle)?);
    let last = app
        .get(&format!("/api/public/articles/{}", slug_of(&oldest)?))
        .await?;
    ensure!(last.body["previous"]["slug"] == slug_of(&middle)?);
    ensure!(last.body["next"].is_null());
    // 分类与标签投影。
    ensure!(last.body["category"]["slug"] == category_slug);
    ensure!(
        last.body["tags"]
            .as_array()
            .is_some_and(|tags| tags.iter().any(|t| t["slug"] == tag_slug))
    );

    // 分类/标签聚合：Draft 不计数。
    let categories = app.get("/api/public/categories").await?;
    let engineering = categories
        .body
        .as_array()
        .and_then(|list| list.iter().find(|c| c["slug"] == category_slug))
        .context("category missing")?;
    ensure!(
        engineering["article_count"] == 1,
        "draft must not be counted: {engineering}"
    );
    let tags = app.get("/api/public/tags").await?;
    let rust = tags
        .body
        .as_array()
        .and_then(|list| list.iter().find(|t| t["slug"] == tag_slug))
        .context("tag missing")?;
    ensure!(rust["article_count"] == 1);

    // 分类/标签下的文章分页与 404。
    let category_articles = app
        .get(&format!("/api/public/categories/{category_slug}/articles"))
        .await?;
    ensure!(category_articles.body["total"] == 1);
    ensure!(category_articles.body["items"][0]["slug"] == slug_of(&oldest)?);
    let unknown = app.get("/api/public/categories/no-such/articles").await?;
    ensure!(unknown.status == StatusCode::NOT_FOUND);
    let tag_articles = app
        .get(&format!("/api/public/tags/{tag_slug}/articles"))
        .await?;
    ensure!(tag_articles.body["total"] == 1);
    let unknown_tag = app.get("/api/public/tags/no-such/articles").await?;
    ensure!(unknown_tag.status == StatusCode::NOT_FOUND);

    // 归档：当前 year/month 分组含 3 篇 Published，不含 Draft。
    let archives = app.get("/api/public/archives").await?;
    ensure!(archives.status == StatusCode::OK, "{}", archives.body);
    let groups = archives.body.as_array().context("archives must be array")?;
    let current = groups.first().context("archive group missing")?;
    ensure!(
        current["count"] == 3,
        "draft leaked into archives: {current}"
    );
    ensure!(current["articles"].as_array().is_some_and(|a| a.len() == 3));

    // Draft 详情 404（可见性兜底）。
    let hidden = app
        .get(&format!("/api/public/articles/{}", slug_of(&draft)?))
        .await?;
    ensure!(hidden.status == StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn article_access_unlock_flow_and_rate_limit() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = access_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn access_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let protected = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "Secret Post",
            "markdown_source": "# Secret Body",
            "access_password": "reader-pass-1",
        }),
    )
    .await?;
    publish_article(app, &cookie, &protected).await?;
    let protected_slug = slug_of(&protected)?.to_owned();
    let protected_id = protected["id"].as_i64().context("id")?;

    // 未解锁：正文为 null，响应 private/no-store。
    let (status, headers, _) = app
        .get_raw(&format!("/api/public/articles/{protected_slug}"))
        .await?;
    ensure!(status == StatusCode::OK);
    let cache = headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .context("missing cache-control")?;
    ensure!(
        cache.contains("no-store"),
        "protected detail must be no-store: {cache}"
    );

    // 无密码文章调用 access 返回 400。
    let plain = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Plain", "markdown_source": "# Plain" }),
    )
    .await?;
    publish_article(app, &cookie, &plain).await?;
    let not_protected = app
        .post_with_headers(
            &format!("/api/public/articles/{}/access", slug_of(&plain)?),
            serde_json::json!({ "password": "whatever" }),
            &[],
        )
        .await?;
    ensure!(
        not_protected.status == StatusCode::BAD_REQUEST,
        "{}",
        not_protected.body
    );
    ensure!(not_protected.body["error"]["code"] == "ARTICLE_NOT_PROTECTED");

    // 正确密码：下发 HttpOnly 解锁 Cookie，带 Cookie 访问详情可读正文。
    let unlocked = app
        .post_with_headers(
            &format!("/api/public/articles/{protected_slug}/access"),
            serde_json::json!({ "password": "reader-pass-1" }),
            &[("x-forwarded-for", "203.0.113.10")],
        )
        .await?;
    ensure!(unlocked.status == StatusCode::OK, "{}", unlocked.body);
    ensure!(unlocked.body["unlocked"] == true);
    let set_cookie = unlocked.set_cookie.context("missing unlock cookie")?;
    ensure!(set_cookie.contains(&format!("aries_article_access_{protected_id}=")));
    ensure!(set_cookie.to_lowercase().contains("httponly"));
    let access_cookie = set_cookie
        .split(';')
        .next()
        .context("cookie pair")?
        .to_owned();
    let detail = app
        .admin_get(
            &format!("/api/public/articles/{protected_slug}"),
            &access_cookie,
        )
        .await?;
    ensure!(detail.status == StatusCode::OK, "{}", detail.body);
    ensure!(
        detail.body["rendered_html"]
            .as_str()
            .is_some_and(|html| html.contains("Secret Body")),
        "unlocked detail must include rendered html: {}",
        detail.body
    );
    // 伪造 Cookie 值不得放行。
    let forged = app
        .admin_get(
            &format!("/api/public/articles/{protected_slug}"),
            &format!("aries_article_access_{protected_id}=forged"),
        )
        .await?;
    ensure!(
        forged.body["rendered_html"].is_null(),
        "forged credential must not unlock"
    );

    // 错误密码 401；同一客户端连续错误触发 429。
    for attempt in 0..5 {
        let wrong = app
            .post_with_headers(
                &format!("/api/public/articles/{protected_slug}/access"),
                serde_json::json!({ "password": format!("wrong-{attempt}") }),
                &[("x-forwarded-for", "203.0.113.20")],
            )
            .await?;
        ensure!(wrong.status == StatusCode::UNAUTHORIZED, "{}", wrong.body);
        ensure!(wrong.body["error"]["code"] == "INVALID_ARTICLE_PASSWORD");
    }
    let limited = app
        .post_with_headers(
            &format!("/api/public/articles/{protected_slug}/access"),
            serde_json::json!({ "password": "wrong-again" }),
            &[("x-forwarded-for", "203.0.113.20")],
        )
        .await?;
    ensure!(
        limited.status == StatusCode::TOO_MANY_REQUESTS,
        "{}",
        limited.body
    );
    Ok(())
}

#[tokio::test]
async fn article_views_dedup_within_sliding_window() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = views_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn views_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let article = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Counted", "markdown_source": "# Counted" }),
    )
    .await?;
    publish_article(app, &cookie, &article).await?;
    let slug = slug_of(&article)?.to_owned();
    let draft = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "Draft Views", "markdown_source": "# Draft" }),
    )
    .await?;

    let first = app
        .post_with_headers(
            &format!("/api/public/articles/{slug}/views"),
            serde_json::json!({}),
            &[("x-forwarded-for", "198.51.100.7")],
        )
        .await?;
    ensure!(first.status == StatusCode::OK, "{}", first.body);
    ensure!(first.body["visit_count"] == 1);

    // 窗口内同一客户端重复计数不增加；客户端 body 中的 count 字段被忽略。
    let repeat = app
        .post_with_headers(
            &format!("/api/public/articles/{slug}/views"),
            serde_json::json!({ "count": 100 }),
            &[("x-forwarded-for", "198.51.100.7")],
        )
        .await?;
    ensure!(
        repeat.body["visit_count"] == 1,
        "window dedup failed: {}",
        repeat.body
    );

    // 不同客户端正常计数。
    let other = app
        .post_with_headers(
            &format!("/api/public/articles/{slug}/views"),
            serde_json::json!({}),
            &[("x-forwarded-for", "198.51.100.8")],
        )
        .await?;
    ensure!(other.body["visit_count"] == 2);

    // Draft 不可见，计数端点 404。
    let hidden = app
        .post_with_headers(
            &format!("/api/public/articles/{}/views", slug_of(&draft)?),
            serde_json::json!({}),
            &[],
        )
        .await?;
    ensure!(hidden.status == StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn public_search_matches_fts_and_ilike_but_never_drafts() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = search_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn search_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let english = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "Systems Notes",
            "markdown_source": "The rustacean borrow checker explained in depth",
        }),
    )
    .await?;
    publish_article(app, &cookie, &english).await?;
    let chinese = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "中文标题测试",
            "summary": "一篇中文摘要",
            "markdown_source": "正文内容",
        }),
    )
    .await?;
    publish_article(app, &cookie, &chinese).await?;
    let draft = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "rustacean draft", "markdown_source": "rustacean" }),
    )
    .await?;

    // FTS 命中正文英文单词。
    let fts = app.get("/api/public/search?q=rustacean").await?;
    ensure!(fts.status == StatusCode::OK, "{}", fts.body);
    let slugs: Vec<&str> = fts.body["items"]
        .as_array()
        .context("items")?
        .iter()
        .filter_map(|item| item["slug"].as_str())
        .collect();
    ensure!(slugs.contains(&slug_of(&english)?), "fts miss: {slugs:?}");
    ensure!(
        !slugs.contains(&slug_of(&draft)?),
        "draft leaked: {slugs:?}"
    );

    // ILIKE 兜底：中文整串匹配标题。
    let ilike = app.get("/api/public/search?q=中文标题").await?;
    let hits: Vec<&str> = ilike.body["items"]
        .as_array()
        .context("items")?
        .iter()
        .filter_map(|item| item["slug"].as_str())
        .collect();
    ensure!(hits.contains(&slug_of(&chinese)?), "ilike miss: {hits:?}");
    // 搜索结果不含正文字段。
    ensure!(ilike.body["items"][0].get("rendered_html").is_none());
    ensure!(ilike.body["items"][0].get("markdown_source").is_none());

    // 空关键词 400；缺省参数同。
    for path in ["/api/public/search?q=", "/api/public/search"] {
        let empty = app.get(path).await?;
        ensure!(empty.status == StatusCode::BAD_REQUEST, "{path}");
        ensure!(empty.body["error"]["code"] == "INVALID_SEARCH_KEYWORD");
    }
    Ok(())
}

#[tokio::test]
async fn public_search_ranks_by_relevance_and_returns_body_snippet() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = search_relevance_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn search_relevance_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    // 标题命中
    let title_hit = create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "所有权 Ownership 指南", "markdown_source": "无关正文" }),
    )
    .await?;
    publish_article(app, &cookie, &title_hit).await?;
    // 仅正文命中：摘要不含关键词，期望返回 matched_excerpt 片段
    let body_hit = create_article(
        app,
        &cookie,
        serde_json::json!({
            "title": "内存管理随笔",
            "summary": "一段无关摘要",
            "markdown_source": "开头介绍背景。所有权（ownership）是 Rust 管理内存的核心机制。结尾总结。",
        }),
    )
    .await?;
    publish_article(app, &cookie, &body_hit).await?;

    let result = app.get("/api/public/search?q=所有权").await?;
    ensure!(result.status == StatusCode::OK, "{}", result.body);
    let items = result.body["items"].as_array().context("items")?;
    let slugs: Vec<&str> = items
        .iter()
        .filter_map(|item| item["slug"].as_str())
        .collect();
    // 相关度排序：标题命中排在仅正文命中之前
    let title_slug = slug_of(&title_hit)?;
    let body_slug = slug_of(&body_hit)?;
    let title_pos = slugs.iter().position(|s| *s == title_slug);
    let body_pos = slugs.iter().position(|s| *s == body_slug);
    ensure!(
        title_pos.is_some() && body_pos.is_some() && title_pos < body_pos,
        "relevance order broken: {slugs:?}"
    );

    // 摘要未覆盖的正文命中返回纯文本片段，且不含 Markdown 记号
    let body_item = items
        .iter()
        .find(|item| item["slug"] == body_slug)
        .context("body hit item")?;
    let excerpt = body_item["matched_excerpt"]
        .as_str()
        .context("matched_excerpt should be present")?;
    ensure!(
        excerpt.contains("所有权"),
        "excerpt miss keyword: {excerpt}"
    );
    ensure!(
        !excerpt.contains('`'),
        "excerpt should be plain text: {excerpt}"
    );
    // 标题命中的条目摘要不含关键词，但 snippet 来自正文（"无关正文"不含关键词）→ 应无片段；
    // 这里标题命中条目的正文也不含关键词，matched_excerpt 缺省（serde skip）
    ensure!(body_item.get("summary").is_some());

    // 搜索建议：前缀/包含匹配，draft 不泄露
    create_article(
        app,
        &cookie,
        serde_json::json!({ "title": "所有权草稿", "markdown_source": "draft" }),
    )
    .await?;
    let suggest = app.get("/api/public/search/suggest?q=所有权").await?;
    ensure!(suggest.status == StatusCode::OK, "{}", suggest.body);
    let titles: Vec<&str> = suggest
        .body
        .as_array()
        .context("suggest body")?
        .iter()
        .filter_map(|item| item["title"].as_str())
        .collect();
    ensure!(
        titles.contains(&"所有权 Ownership 指南"),
        "suggest miss: {titles:?}"
    );
    ensure!(
        !titles.contains(&"所有权草稿"),
        "draft leaked into suggest: {titles:?}"
    );
    // 建议项只含 slug/title
    ensure!(suggest.body[0].get("summary").is_none());

    // suggest 空关键词同样 400
    let empty = app.get("/api/public/search/suggest?q=").await?;
    ensure!(empty.status == StatusCode::BAD_REQUEST);
    ensure!(empty.body["error"]["code"] == "INVALID_SEARCH_KEYWORD");
    Ok(())
}
