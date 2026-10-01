//! Public 扩展内容端点（pages / journals / galleries / photos / links / navigation）集成测试：
//! 匿名可访问（无需 Cookie）、可见性边界（draft/private/inactive/hidden 绝不外泄）、
//! 404 语义、分页边界（page/page_size clamp）与公开 DTO 字段集合。
//! 通过 `ARIES_RUN_DATABASE_TESTS=1` 门控，随机 Schema 隔离，走真实 Router + PostgreSQL。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

/// 断言 JSON 对象的字段集合与期望完全一致（公开 DTO 不得多出内部字段）。
fn ensure_exact_keys(value: &serde_json::Value, expected: &[&str]) -> anyhow::Result<()> {
    let object = value.as_object().context("response must be an object")?;
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    ensure!(keys == expected, "unexpected keys: {keys:?}");
    Ok(())
}

/// 直接 SQL 插入媒体资产作为前置数据（绕过上传流程），同 galleries_api.rs 的模式。
async fn insert_media_asset(app: &TestApp, uploader_id: i64, key: &str) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO media_assets \
         (provider, object_key, url, original_name, mime, size_bytes, width, height, sha256, uploaded_by) \
         VALUES ('local', $1, $2, 'photo.jpg', 'image/jpeg', 1024, 800, 600, $3, $4) \
         RETURNING id",
    )
    .bind(key)
    .bind(format!("/api/media/files/{key}"))
    .bind(format!("sha256-{key}"))
    .bind(uploader_id)
    .fetch_one(&app.state.database)
    .await
    .context("insert media asset")?;
    Ok(id)
}

async fn create_page(
    app: &TestApp,
    cookie: &str,
    body: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let response = app
        .admin_post("/api/admin/pages", body, Some(cookie))
        .await?;
    ensure!(
        response.status == StatusCode::CREATED,
        "create page failed: {}",
        response.body
    );
    Ok(response.body)
}

async fn create_journal(
    app: &TestApp,
    cookie: &str,
    content_markdown: &str,
    visibility: &str,
) -> anyhow::Result<i64> {
    let response = app
        .admin_post(
            "/api/admin/journals",
            serde_json::json!({ "content_markdown": content_markdown, "visibility": visibility }),
            Some(cookie),
        )
        .await?;
    ensure!(
        response.status == StatusCode::CREATED,
        "create journal failed: {}",
        response.body
    );
    response.body["id"].as_i64().context("missing journal id")
}

#[tokio::test]
async fn public_pages_publish_visibility_exact_fields_and_404() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = page_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn page_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;

    // 草稿页面对 Public 不可见 → 404。
    let draft = create_page(
        app,
        &cookie,
        serde_json::json!({
            "slug": "about",
            "title": "About",
            "content_markdown": "# Hello Public",
        }),
    )
    .await?;
    let page_id = draft["id"].as_i64().context("missing page id")?;
    let draft_public = app.get("/api/public/pages/about").await?;
    ensure!(draft_public.status == StatusCode::NOT_FOUND);

    // 未知 slug → 404。
    let unknown = app.get("/api/public/pages/no-such-page").await?;
    ensure!(unknown.status == StatusCode::NOT_FOUND);

    // 发布后匿名可读，无需 Cookie。
    let published = app
        .admin_put(
            &format!("/api/admin/pages/{page_id}"),
            serde_json::json!({
                "slug": "about",
                "title": "About",
                "content_markdown": "# Hello Public",
                "status": "published",
            }),
            &cookie,
        )
        .await?;
    ensure!(published.status == StatusCode::OK, "{}", published.body);

    let (status, headers, bytes) = app.get_raw("/api/public/pages/about").await?;
    ensure!(status == StatusCode::OK);
    // 单内容缓存策略：CACHE_ARTICLE。
    let cache = headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .context("missing cache-control")?;
    ensure!(cache == "public, max-age=60", "unexpected: {cache}");

    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(body["slug"] == "about");
    ensure!(body["title"] == "About");
    ensure!(
        body["content_html"]
            .as_str()
            .is_some_and(|html| html.contains("Hello Public")),
        "{}",
        body
    );
    // 公开 DTO 只回展示字段，不回 Markdown 源与内部字段。
    ensure_exact_keys(&body, &["slug", "title", "content_html", "updated_at"])?;
    Ok(())
}

#[tokio::test]
async fn public_journals_never_leak_private_and_clamp_pagination() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = journal_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn journal_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;

    // 三条 Public + 两条 Private。
    let first = create_journal(app, &cookie, "public note 1", "public").await?;
    let second = create_journal(app, &cookie, "public note 2", "public").await?;
    let third = create_journal(app, &cookie, "public note 3", "public").await?;
    let private_a = create_journal(app, &cookie, "private note a", "private").await?;
    let private_b = create_journal(app, &cookie, "private note b", "private").await?;

    // 默认分页：total 只计 Public；倒序（id DESC）。
    let (status, headers, bytes) = app.get_raw("/api/public/journals").await?;
    ensure!(status == StatusCode::OK);
    // 聚合缓存策略：CACHE_AGGREGATE。
    let cache = headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .context("missing cache-control")?;
    ensure!(
        cache.contains("stale-while-revalidate"),
        "unexpected: {cache}"
    );

    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure_exact_keys(&body, &["items", "total", "page", "page_size"])?;
    ensure!(body["total"] == 3, "private leaked: {body}");
    ensure!(body["page"] == 1 && body["page_size"] == 20);
    let ids: Vec<i64> = body["items"]
        .as_array()
        .context("items")?
        .iter()
        .filter_map(|item| item["id"].as_i64())
        .collect();
    ensure!(ids == [third, second, first], "unexpected order: {ids:?}");
    for item in body["items"].as_array().context("items")? {
        // 公开条目不回 visibility 等管理字段。
        ensure_exact_keys(item, &["id", "content_html", "created_at"])?;
    }

    // 分页边界：page_size=1 时逐页取，total 恒为 3。
    let page_two = app.get("/api/public/journals?page=2&page_size=1").await?;
    ensure!(page_two.body["total"] == 3);
    ensure!(page_two.body["page"] == 2 && page_two.body["page_size"] == 1);
    ensure!(page_two.body["items"][0]["id"].as_i64() == Some(second));

    // 越界页：page > 1 且 items 为空 → 404（阻止爬虫生成无限重复 URL）。
    let overflow = app.get("/api/public/journals?page=99&page_size=20").await?;
    ensure!(
        overflow.status == StatusCode::NOT_FOUND,
        "{}",
        overflow.body
    );
    ensure!(overflow.body["error"]["code"] == "PAGE_OUT_OF_RANGE");

    // clamp 边界：page=0 视为 1，page_size=0 提升为 1。
    let clamped_low = app.get("/api/public/journals?page=0&page_size=0").await?;
    ensure!(clamped_low.body["page"] == 1 && clamped_low.body["page_size"] == 1);
    ensure!(clamped_low.body["items"][0]["id"].as_i64() == Some(third));

    // clamp 边界：page_size 超上限收斂到 100，一次拿完全部 3 条。
    let clamped_high = app.get("/api/public/journals?page_size=1000").await?;
    ensure!(clamped_high.body["page_size"] == 100);
    ensure!(
        clamped_high.body["items"].as_array().map(Vec::len) == Some(3),
        "{}",
        clamped_high.body
    );

    // 全部分页遍历：Private id 绝不出现在任何一页。
    let mut seen: Vec<i64> = Vec::new();
    for page in 1..=3 {
        let slice = app
            .get(&format!("/api/public/journals?page={page}&page_size=1"))
            .await?;
        seen.extend(
            slice.body["items"]
                .as_array()
                .context("items")?
                .iter()
                .filter_map(|item| item["id"].as_i64()),
        );
    }
    ensure!(
        !seen.contains(&private_a) && !seen.contains(&private_b),
        "private journal leaked: {seen:?}"
    );
    Ok(())
}

#[tokio::test]
async fn public_galleries_and_photos_visibility_404_and_broken_media() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = gallery_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn gallery_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;
    let media_a = insert_media_asset(app, 1, "2026/09/ext-a.jpg").await?;
    let media_b = insert_media_asset(app, 1, "2026/09/ext-b.jpg").await?;
    let media_c = insert_media_asset(app, 1, "2026/09/ext-c.jpg").await?;

    // 图库分类（kind = gallery）。
    let category = app
        .admin_post(
            "/api/admin/galleries/categories",
            serde_json::json!({ "name": "Travel", "slug": "travel" }),
            Some(&cookie),
        )
        .await?;
    ensure!(category.status == StatusCode::CREATED, "{}", category.body);
    let category_id = category.body["id"]
        .as_i64()
        .context("missing category id")?;

    // 已发布图库（封面 + 两个条目）与一个草稿图库（含一个条目）。
    let gallery = app
        .admin_post(
            "/api/admin/galleries",
            serde_json::json!({
                "category_id": category_id,
                "slug": "iceland",
                "title": "Iceland",
                "description": "North island",
                "cover_media_id": media_a,
                "status": "published",
            }),
            Some(&cookie),
        )
        .await?;
    ensure!(gallery.status == StatusCode::CREATED, "{}", gallery.body);
    let gallery_id = gallery.body["id"].as_i64().context("missing gallery id")?;
    let draft_gallery = app
        .admin_post(
            "/api/admin/galleries",
            serde_json::json!({
                "category_id": category_id,
                "slug": "wip-album",
                "title": "WIP",
            }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        draft_gallery.status == StatusCode::CREATED,
        "{}",
        draft_gallery.body
    );
    let draft_gallery_id = draft_gallery.body["id"].as_i64().context("missing id")?;

    for (media, alt, sort_order) in [(media_a, "Glacier", 0), (media_b, "Waterfall", 1)] {
        let item = app
            .admin_post(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                serde_json::json!({ "media_asset_id": media, "alt": alt, "sort_order": sort_order }),
                Some(&cookie),
            )
            .await?;
        ensure!(item.status == StatusCode::CREATED, "{}", item.body);
    }
    let draft_item = app
        .admin_post(
            &format!("/api/admin/galleries/{draft_gallery_id}/items"),
            serde_json::json!({ "media_asset_id": media_c, "alt": "Hidden" }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        draft_item.status == StatusCode::CREATED,
        "{}",
        draft_item.body
    );

    // 列表：草稿图库不出现；summary 字段集合固定。
    let list = app.get("/api/public/galleries").await?;
    ensure!(list.status == StatusCode::OK, "{}", list.body);
    ensure!(list.body["total"] == 1, "draft leaked: {}", list.body);
    let summary = &list.body["items"][0];
    ensure!(summary["slug"] == "iceland");
    ensure!(summary["cover_url"] == "/api/media/files/2026/09/ext-a.jpg");
    ensure_exact_keys(summary, &["slug", "title", "description", "cover_url"])?;

    // 分页边界：page > 1 且 items 为空 → 404；page_size clamp 生效。
    let overflow = app.get("/api/public/galleries?page=2&page_size=1").await?;
    ensure!(
        overflow.status == StatusCode::NOT_FOUND,
        "{}",
        overflow.body
    );
    ensure!(overflow.body["error"]["code"] == "PAGE_OUT_OF_RANGE");
    let clamped = app.get("/api/public/galleries?page=0&page_size=0").await?;
    ensure!(clamped.body["page"] == 1 && clamped.body["page_size"] == 1);
    ensure!(clamped.body["items"].as_array().map(Vec::len) == Some(1));

    // 详情：草稿与未知 slug 均 404。
    let draft_detail = app.get("/api/public/galleries/wip-album").await?;
    ensure!(draft_detail.status == StatusCode::NOT_FOUND);
    let unknown = app.get("/api/public/galleries/no-such-gallery").await?;
    ensure!(unknown.status == StatusCode::NOT_FOUND);

    // 已发布详情：条目按 sort_order 排列，含解析后的 URL/尺寸。
    let (status, headers, bytes) = app.get_raw("/api/public/galleries/iceland").await?;
    ensure!(status == StatusCode::OK);
    let cache = headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .context("missing cache-control")?;
    ensure!(cache == "public, max-age=60", "unexpected: {cache}");
    let detail: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure_exact_keys(&detail, &["slug", "title", "description", "items"])?;
    let items = detail["items"].as_array().context("items")?;
    ensure!(items.len() == 2, "{detail}");
    ensure!(items[0]["url"] == "/api/media/files/2026/09/ext-a.jpg");
    ensure!(items[0]["width"].as_i64() == Some(800));
    ensure!(items[1]["alt"] == "Waterfall");
    for item in items {
        ensure_exact_keys(item, &["url", "alt", "location", "width", "height"])?;
    }

    // 照片墙：只含已发布图库的照片，草稿图库的 Hidden 不出现。
    let photos = app.get("/api/public/photos").await?;
    ensure!(photos.status == StatusCode::OK, "{}", photos.body);
    let photo_list = photos.body.as_array().context("photos")?;
    ensure!(
        photo_list.len() == 2,
        "draft gallery leaked: {}",
        photos.body
    );
    ensure!(
        photo_list
            .iter()
            .all(|photo| photo["gallery_slug"] == "iceland"),
        "{}",
        photos.body
    );
    for photo in photo_list {
        ensure_exact_keys(
            photo,
            &[
                "url",
                "alt",
                "location",
                "width",
                "height",
                "gallery_slug",
                "gallery_title",
                "category_name",
            ],
        )?;
    }

    // 媒体软删除后：详情与照片墙跳过该条目（公开站不出坏图）。
    sqlx::query("UPDATE media_assets SET deleted_at = now() WHERE id = $1")
        .bind(media_b)
        .execute(&app.state.database)
        .await?;
    let detail_after = app.get("/api/public/galleries/iceland").await?;
    ensure!(
        detail_after.body["items"].as_array().map(Vec::len) == Some(1),
        "{}",
        detail_after.body
    );
    let photos_after = app.get("/api/public/photos").await?;
    ensure!(
        photos_after.body.as_array().map(Vec::len) == Some(1),
        "{}",
        photos_after.body
    );
    Ok(())
}

#[tokio::test]
async fn public_links_active_only_with_resolved_category() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = link_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn link_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;

    // 友链分类（kind = link）。
    let category = app
        .admin_post(
            "/api/admin/links/categories",
            serde_json::json!({ "name": "Blogs", "slug": "blogs" }),
            Some(&cookie),
        )
        .await?;
    ensure!(category.status == StatusCode::CREATED, "{}", category.body);
    let category_id = category.body["id"]
        .as_i64()
        .context("missing category id")?;

    // 一条 active（有分类）、一条 active（无分类）、一条 inactive。
    let active = app
        .admin_post(
            "/api/admin/links",
            serde_json::json!({
                "category_id": category_id,
                "title": "Example",
                "url": "https://example.com",
                "description": "An example site",
            }),
            Some(&cookie),
        )
        .await?;
    ensure!(active.status == StatusCode::CREATED, "{}", active.body);
    let solo = app
        .admin_post(
            "/api/admin/links",
            serde_json::json!({ "title": "Solo", "url": "https://solo.example.com" }),
            Some(&cookie),
        )
        .await?;
    ensure!(solo.status == StatusCode::CREATED, "{}", solo.body);
    let inactive = app
        .admin_post(
            "/api/admin/links",
            serde_json::json!({
                "title": "Inactive",
                "url": "https://inactive.example.com",
                "status": "inactive",
            }),
            Some(&cookie),
        )
        .await?;
    ensure!(inactive.status == StatusCode::CREATED, "{}", inactive.body);

    // 匿名访问：只见两条 active，inactive 不外泄。
    let (status, headers, bytes) = app.get_raw("/api/public/links").await?;
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
    let links = body.as_array().context("links")?;
    ensure!(links.len() == 2, "inactive leaked: {body}");
    ensure!(links.iter().all(|link| link["title"] != "Inactive"));

    // 分类名已解析；无分类友链 category_name 为 null。
    let example = links
        .iter()
        .find(|link| link["title"] == "Example")
        .context("missing Example")?;
    ensure!(example["category_id"].as_i64() == Some(category_id));
    ensure!(example["category_name"] == "Blogs");
    let solo = links
        .iter()
        .find(|link| link["title"] == "Solo")
        .context("missing Solo")?;
    ensure!(solo["category_id"].is_null() && solo["category_name"].is_null());

    // 公开 DTO 不回 status/sort_order 等管理字段。
    for link in links {
        ensure_exact_keys(
            link,
            &[
                "title",
                "url",
                "icon_url",
                "description",
                "category_id",
                "category_name",
            ],
        )?;
    }
    Ok(())
}

#[tokio::test]
async fn public_navigation_excludes_hidden_and_dead_targets() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = navigation_scenario(&app).await;
    let cleanup = app.cleanup().await;
    scenario?;
    cleanup
}

async fn navigation_scenario(app: &TestApp) -> anyhow::Result<()> {
    let cookie = app.bootstrap_owner().await?;

    // 前置目标：已发布文章、草稿文章（死目标）、文章分类。
    let article = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({ "title": "Public Post", "markdown_source": "# Hi" }),
            Some(&cookie),
        )
        .await?;
    ensure!(article.status == StatusCode::CREATED, "{}", article.body);
    let article_id = article.body["id"].as_i64().context("missing article id")?;
    let article_version = article.body["version"]
        .as_i64()
        .context("missing version")?;
    let article_slug = article.body["slug"]
        .as_str()
        .context("missing slug")?
        .to_owned();
    let published = app
        .admin_patch(
            &format!("/api/admin/articles/{article_id}/status"),
            serde_json::json!({ "command": "publish", "expected_version": article_version }),
            &cookie,
        )
        .await?;
    ensure!(published.status == StatusCode::OK, "{}", published.body);
    let draft_article = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({ "title": "Draft Post", "markdown_source": "# Draft" }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        draft_article.status == StatusCode::CREATED,
        "{}",
        draft_article.body
    );
    let draft_article_id = draft_article.body["id"].as_i64().context("missing id")?;
    let category = app
        .admin_post(
            "/api/admin/categories",
            serde_json::json!({ "name": "Rust", "slug": "rust" }),
            Some(&cookie),
        )
        .await?;
    ensure!(category.status == StatusCode::CREATED, "{}", category.body);
    let category_id = category.body["id"]
        .as_i64()
        .context("missing category id")?;

    async fn create_nav(
        app: &TestApp,
        cookie: &str,
        body: serde_json::Value,
    ) -> anyhow::Result<i64> {
        let response = app
            .admin_post("/api/admin/navigation", body, Some(cookie))
            .await?;
        ensure!(
            response.status == StatusCode::CREATED,
            "create nav failed: {}",
            response.body
        );
        response.body["id"].as_i64().context("missing nav id")
    }

    // 一级：article（活目标）、url、隐藏 url、死目标（草稿文章）。
    let posts_id = create_nav(
        app,
        &cookie,
        serde_json::json!({
            "label": "Posts", "target_type": "article", "target_id": article_id, "sort_order": 0,
        }),
    )
    .await?;
    create_nav(
        app,
        &cookie,
        serde_json::json!({
            "label": "Home", "target_type": "url", "url": "https://blog.example.com/",
            "open_in_new_tab": true, "sort_order": 1,
        }),
    )
    .await?;
    create_nav(
        app,
        &cookie,
        serde_json::json!({
            "label": "Secret", "target_type": "url", "url": "https://secret.example.com",
            "visible": false, "sort_order": 2,
        }),
    )
    .await?;
    create_nav(
        app,
        &cookie,
        serde_json::json!({
            "label": "Dead", "target_type": "article", "target_id": draft_article_id, "sort_order": 3,
        }),
    )
    .await?;
    // 二级：挂在 Posts 下的分类节点。
    create_nav(
        app,
        &cookie,
        serde_json::json!({
            "label": "Rust", "target_type": "category", "target_id": category_id,
            "parent_id": posts_id,
        }),
    )
    .await?;

    // 匿名访问：隐藏项与死目标被剔除，只剩 Posts 与 Home。
    let (status, headers, bytes) = app.get_raw("/api/public/navigation").await?;
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
    let tree = body.as_array().context("navigation tree")?;
    ensure!(
        tree.len() == 2,
        "hidden/dead items must be excluded: {body}"
    );

    // Posts：href 由服务端解析为 /articles/{slug}，含一个子节点。
    ensure!(tree[0]["label"] == "Posts");
    ensure!(tree[0]["href"] == format!("/articles/{article_slug}"));
    ensure!(tree[0]["target_type"] == "article");
    let children = tree[0]["children"].as_array().context("children")?;
    ensure!(children.len() == 1);
    ensure!(children[0]["label"] == "Rust");
    ensure!(children[0]["href"] == "/categories/rust");
    // Home：url 原样保留，open_in_new_tab 透传。
    ensure!(tree[1]["label"] == "Home");
    ensure!(tree[1]["href"] == "https://blog.example.com/");
    ensure!(tree[1]["open_in_new_tab"] == true);

    // 公开节点不回 visible/sort_order 等管理字段。
    for node in tree.iter().chain(children.iter()) {
        ensure_exact_keys(
            node,
            &[
                "label",
                "target_type",
                "target_id",
                "url",
                "href",
                "open_in_new_tab",
                "children",
            ],
        )?;
    }
    Ok(())
}
