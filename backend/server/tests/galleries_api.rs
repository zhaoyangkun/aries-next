//! Phase 06 Contract Test：图库 Admin CRUD + 条目管理 + 原子排序 + Public 端点。

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

/// 直接 SQL 插入媒体资产作为前置数据（绕过上传流程）。
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

#[tokio::test]
async fn gallery_crud_items_reorder_and_public_boundary() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let media_a = insert_media_asset(&app, 1, "2026/09/a.jpg").await?;
        let media_b = insert_media_asset(&app, 1, "2026/09/b.jpg").await?;

        // 图库分类（kind = gallery）。
        let category = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "Travel", "slug": "travel" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(category.status == StatusCode::CREATED, "{}", category.body);
        ensure!(category.body["kind"] == "gallery");
        let category_id = category.body["id"].as_i64().context("missing category id")?;

        // 创建图库（草稿 + 封面）。
        let created = app
            .admin_post(
                "/api/admin/galleries",
                serde_json::json!({
                    "category_id": category_id,
                    "slug": "iceland",
                    "title": "Iceland",
                    "cover_media_id": media_a,
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        ensure!(created.body["status"] == "draft");
        let gallery_id = created.body["id"].as_i64().context("missing gallery id")?;

        // slug 冲突 → 409。
        let duplicate = app
            .admin_post(
                "/api/admin/galleries",
                serde_json::json!({
                    "category_id": category_id, "slug": "iceland", "title": "Again",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(duplicate.status == StatusCode::CONFLICT, "{}", duplicate.body);
        ensure!(duplicate.body["error"]["code"] == "GALLERY_CONFLICT");

        // 草稿对 Public 不可见。
        let draft_public = app.get("/api/public/galleries/iceland").await?;
        ensure!(draft_public.status == StatusCode::NOT_FOUND);

        // 添加两个条目；重复媒体 → 409。
        let item_a = app
            .admin_post(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                serde_json::json!({ "media_asset_id": media_a, "alt": "Glacier" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(item_a.status == StatusCode::CREATED, "{}", item_a.body);
        // 新增响应内联媒体摘要（消除前端二次拉取的 N+1）。
        let media = &item_a.body["media"];
        ensure!(media["id"].as_i64() == Some(media_a), "{}", item_a.body);
        ensure!(media["url"] == "/api/media/files/2026/09/a.jpg");
        ensure!(media["width"].as_i64() == Some(800));
        ensure!(media["height"].as_i64() == Some(600));
        let item_a_id = item_a.body["id"].as_i64().context("missing item id")?;
        let item_b = app
            .admin_post(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                serde_json::json!({ "media_asset_id": media_b, "location": "Reykjavik" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(item_b.status == StatusCode::CREATED, "{}", item_b.body);
        let item_b_id = item_b.body["id"].as_i64().context("missing item id")?;

        let duplicated_item = app
            .admin_post(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                serde_json::json!({ "media_asset_id": media_a }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(duplicated_item.status == StatusCode::CONFLICT, "{}", duplicated_item.body);

        // 原子排序：B 排到 A 前。
        let reordered = app
            .admin_put(
                &format!("/api/admin/galleries/{gallery_id}/items/order"),
                serde_json::json!({ "item_ids": [item_b_id, item_a_id] }),
                &owner_cookie,
            )
            .await?;
        ensure!(reordered.status == StatusCode::NO_CONTENT, "{}", reordered.body);
        let items = app
            .admin_get(&format!("/api/admin/galleries/{gallery_id}/items"), &owner_cookie)
            .await?;
        ensure!(items.status == StatusCode::OK, "{}", items.body);
        ensure!(items.body[0]["id"].as_i64() == Some(item_b_id));
        ensure!(items.body[1]["id"].as_i64() == Some(item_a_id));
        // 列表同样内联媒体摘要。
        ensure!(items.body[0]["media"]["url"] == "/api/media/files/2026/09/b.jpg");
        ensure!(items.body[1]["media"]["id"].as_i64() == Some(media_a));

        // 更新条目 alt/location；更新响应也带 media。
        let updated_item = app
            .admin_put(
                &format!("/api/admin/galleries/{gallery_id}/items/{item_a_id}"),
                serde_json::json!({ "alt": "Vatnajokull", "location": "Skaftafell", "sort_order": 1 }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated_item.status == StatusCode::OK, "{}", updated_item.body);
        ensure!(updated_item.body["alt"] == "Vatnajokull");
        ensure!(updated_item.body["media"]["url"] == "/api/media/files/2026/09/a.jpg");

        // 媒体资产软删除后：media 为 null（LEFT JOIN + deleted_at 过滤），条目本身仍在。
        sqlx::query("UPDATE media_assets SET deleted_at = now() WHERE id = $1")
            .bind(media_a)
            .execute(&app.state.database)
            .await?;
        let items_after_delete = app
            .admin_get(&format!("/api/admin/galleries/{gallery_id}/items"), &owner_cookie)
            .await?;
        ensure!(items_after_delete.status == StatusCode::OK);
        let row_a = items_after_delete.body
            .as_array()
            .context("missing items")?
            .iter()
            .find(|item| item["id"].as_i64() == Some(item_a_id))
            .context("missing item a")?;
        ensure!(row_a["media"].is_null(), "{}", row_a);
        // 恢复（不影响后续发布流程）。
        sqlx::query("UPDATE media_assets SET deleted_at = NULL WHERE id = $1")
            .bind(media_a)
            .execute(&app.state.database)
            .await?;

        // 发布图库：Public 列表与详情可见，详情含条目 URL 与尺寸。
        let published = app
            .admin_put(
                &format!("/api/admin/galleries/{gallery_id}"),
                serde_json::json!({
                    "category_id": category_id,
                    "slug": "iceland",
                    "title": "Iceland",
                    "cover_media_id": media_a,
                    "status": "published",
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(published.status == StatusCode::OK, "{}", published.body);

        let public_list = app.get("/api/public/galleries").await?;
        ensure!(public_list.status == StatusCode::OK, "{}", public_list.body);
        ensure!(public_list.body["total"].as_i64() == Some(1));
        ensure!(
            public_list.body["items"][0]["cover_url"].as_str()
                == Some("/api/media/files/2026/09/a.jpg")
        );

        let public_detail = app.get("/api/public/galleries/iceland").await?;
        ensure!(public_detail.status == StatusCode::OK, "{}", public_detail.body);
        let public_items = public_detail.body["items"].as_array().context("missing items")?;
        ensure!(public_items.len() == 2);
        // 排序生效：B 在前。
        ensure!(public_items[0]["url"] == "/api/media/files/2026/09/b.jpg");
        ensure!(public_items[0]["location"] == "Reykjavik");
        ensure!(public_items[1]["alt"] == "Vatnajokull");
        ensure!(public_items[1]["width"].as_i64() == Some(800));

        // 分类被图库引用 → 删除 409。
        let referenced = app
            .admin_delete(
                &format!("/api/admin/galleries/categories/{category_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(referenced.status == StatusCode::CONFLICT, "{}", referenced.body);

        // 移除条目 + 软删图库。
        let removed = app
            .admin_delete(
                &format!("/api/admin/galleries/{gallery_id}/items/{item_b_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(removed.status == StatusCode::NO_CONTENT, "{}", removed.body);
        let deleted = app
            .admin_delete(&format!("/api/admin/galleries/{gallery_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let public_gone = app.get("/api/public/galleries/iceland").await?;
        ensure!(public_gone.status == StatusCode::NOT_FOUND);

        // 审计：gallery.created/updated/deleted + item 操作 + category.created。
        let (audit_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs WHERE target_type = 'gallery'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(audit_count >= 8, "expected >= 8 gallery audit events, got {audit_count}");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn gallery_permission_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-g", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-g", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-g", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-g", "moderator-pass-1").await?;

        let categories = app
            .admin_get("/api/admin/galleries/categories", &editor_cookie)
            .await?;
        ensure!(categories.status == StatusCode::OK, "{}", categories.body);

        let denied = app
            .admin_get("/api/admin/galleries", &moderator_cookie)
            .await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);

        let unauthenticated = app.get("/api/admin/galleries").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

/// 创建图库并返回 id。
async fn create_gallery(
    app: &TestApp,
    cookie: &str,
    category_id: i64,
    slug: &str,
    title: &str,
    status: &str,
    sort_order: i32,
) -> anyhow::Result<i64> {
    let created = app
        .admin_post(
            "/api/admin/galleries",
            serde_json::json!({
                "category_id": category_id,
                "slug": slug,
                "title": title,
                "status": status,
                "sort_order": sort_order,
            }),
            Some(cookie),
        )
        .await?;
    ensure!(created.status == StatusCode::CREATED, "{}", created.body);
    created.body["id"].as_i64().context("missing gallery id")
}

async fn add_item(
    app: &TestApp,
    cookie: &str,
    gallery_id: i64,
    media_asset_id: i64,
    alt: &str,
    location: &str,
    sort_order: i32,
) -> anyhow::Result<i64> {
    let added = app
        .admin_post(
            &format!("/api/admin/galleries/{gallery_id}/items"),
            serde_json::json!({
                "media_asset_id": media_asset_id,
                "alt": alt,
                "location": location,
                "sort_order": sort_order,
            }),
            Some(cookie),
        )
        .await?;
    ensure!(added.status == StatusCode::CREATED, "{}", added.body);
    added.body["id"].as_i64().context("missing item id")
}

/// 创建图库分类并返回 id。
async fn create_gallery_category(
    app: &TestApp,
    cookie: &str,
    name: &str,
    slug: Option<&str>,
) -> anyhow::Result<i64> {
    let payload = match slug {
        Some(slug) => serde_json::json!({ "name": name, "slug": slug }),
        None => serde_json::json!({ "name": name }),
    };
    let created = app
        .admin_post("/api/admin/galleries/categories", payload, Some(cookie))
        .await?;
    ensure!(created.status == StatusCode::CREATED, "{}", created.body);
    created.body["id"].as_i64().context("missing category id")
}

#[tokio::test]
async fn public_photos_wall_flattens_published_galleries_in_stable_order() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let media_a = insert_media_asset(&app, 1, "2026/09/wall-a.jpg").await?;
        let media_b = insert_media_asset(&app, 1, "2026/09/wall-b.jpg").await?;
        let media_c = insert_media_asset(&app, 1, "2026/09/wall-c.jpg").await?;

        let category = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "Travel", "slug": "travel" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(category.status == StatusCode::CREATED, "{}", category.body);
        let category_id = category.body["id"]
            .as_i64()
            .context("missing category id")?;

        // 三个相册：B 排序在前（sort_order 0），A 其次（sort_order 1），C 为草稿（不出现）。
        let gallery_a = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "iceland",
            "Iceland",
            "published",
            1,
        )
        .await?;
        let gallery_b = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "kyoto",
            "Kyoto",
            "published",
            0,
        )
        .await?;
        let gallery_c = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "draft-album",
            "Draft",
            "draft",
            0,
        )
        .await?;

        add_item(&app, &owner_cookie, gallery_a, media_a, "Glacier", "Vik", 0).await?;
        add_item(
            &app,
            &owner_cookie,
            gallery_b,
            media_b,
            "Temple",
            "Kyoto",
            1,
        )
        .await?;
        add_item(
            &app,
            &owner_cookie,
            gallery_b,
            media_c,
            "Bamboo",
            "Arashiyama",
            0,
        )
        .await?;
        add_item(
            &app,
            &owner_cookie,
            gallery_c,
            media_a,
            "Hidden",
            "Nowhere",
            0,
        )
        .await?;

        // 公开照片墙：无 Cookie、不分页、一次拉完。
        let (status, headers, bytes) = app.get_raw("/api/public/photos").await?;
        ensure!(status == StatusCode::OK);
        // 缓存策略与 galleries 列表一致（CACHE_AGGREGATE）。
        let cache_control = headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .context("missing cache-control")?;
        ensure!(cache_control.contains("max-age=60"), "{cache_control}");

        let photos: serde_json::Value = serde_json::from_slice(&bytes)?;
        let photos = photos.as_array().context("photos is not an array")?;
        // draft 相册的照片不出现：3 张而非 4 张。
        ensure!(photos.len() == 3, "{photos:?}");

        // 稳定排序：gallery B（sort 0）内条目按自身 sort_order，然后 gallery A。
        let urls: Vec<&str> = photos.iter().filter_map(|p| p["url"].as_str()).collect();
        ensure!(
            urls == vec![
                "/api/media/files/2026/09/wall-c.jpg",
                "/api/media/files/2026/09/wall-b.jpg",
                "/api/media/files/2026/09/wall-a.jpg",
            ],
            "{urls:?}"
        );

        // 字段齐全：相册信息与分类名已解析，尺寸来自媒体资产。
        let first = &photos[0];
        ensure!(first["alt"] == "Bamboo");
        ensure!(first["location"] == "Arashiyama");
        ensure!(first["width"].as_i64() == Some(800));
        ensure!(first["height"].as_i64() == Some(600));
        ensure!(first["gallery_slug"] == "kyoto");
        ensure!(first["gallery_title"] == "Kyoto");
        ensure!(first["category_name"] == "Travel");
        ensure!(photos[2]["gallery_slug"] == "iceland");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn gallery_validation_and_not_found_boundaries() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let category_id =
            create_gallery_category(&app, &owner_cookie, "Travel", Some("travel")).await?;

        // 非法 slug（大写/特殊字符）→ 400 INVALID_GALLERY_SLUG。
        let bad_slug = app
            .admin_post(
                "/api/admin/galleries",
                serde_json::json!({
                    "category_id": category_id, "slug": "Iceland!", "title": "Iceland",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            bad_slug.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_slug.body
        );
        ensure!(bad_slug.body["error"]["code"] == "INVALID_GALLERY_SLUG");

        // 空白标题 → 400 INVALID_GALLERY_TITLE。
        let bad_title = app
            .admin_post(
                "/api/admin/galleries",
                serde_json::json!({
                    "category_id": category_id, "slug": "empty-title", "title": "   ",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            bad_title.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_title.body
        );
        ensure!(bad_title.body["error"]["code"] == "INVALID_GALLERY_TITLE");

        // 未知 status → 400 INVALID_GALLERY_STATUS。
        let bad_status = app
            .admin_post(
                "/api/admin/galleries",
                serde_json::json!({
                    "category_id": category_id, "slug": "bad-status",
                    "title": "Bad Status", "status": "live",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            bad_status.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_status.body
        );
        ensure!(bad_status.body["error"]["code"] == "INVALID_GALLERY_STATUS");

        // 不存在的分类 / 封面媒体 → FK 23503 映射为 404 GALLERY_NOT_FOUND。
        let missing_category = app
            .admin_post(
                "/api/admin/galleries",
                serde_json::json!({
                    "category_id": 999_999, "slug": "no-category", "title": "No Category",
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            missing_category.status == StatusCode::NOT_FOUND,
            "{}",
            missing_category.body
        );
        ensure!(missing_category.body["error"]["code"] == "GALLERY_NOT_FOUND");
        let missing_cover = app
            .admin_post(
                "/api/admin/galleries",
                serde_json::json!({
                    "category_id": category_id, "slug": "no-cover",
                    "title": "No Cover", "cover_media_id": 999_999,
                }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            missing_cover.status == StatusCode::NOT_FOUND,
            "{}",
            missing_cover.body
        );

        // 不存在图库的读/改/删 → 404 GALLERY_NOT_FOUND。
        let get_missing = app
            .admin_get("/api/admin/galleries/999999", &owner_cookie)
            .await?;
        ensure!(
            get_missing.status == StatusCode::NOT_FOUND,
            "{}",
            get_missing.body
        );
        ensure!(get_missing.body["error"]["code"] == "GALLERY_NOT_FOUND");
        let update_missing = app
            .admin_put(
                "/api/admin/galleries/999999",
                serde_json::json!({
                    "category_id": category_id, "slug": "ghost", "title": "Ghost",
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            update_missing.status == StatusCode::NOT_FOUND,
            "{}",
            update_missing.body
        );
        let delete_missing = app
            .admin_delete("/api/admin/galleries/999999", &owner_cookie)
            .await?;
        ensure!(
            delete_missing.status == StatusCode::NOT_FOUND,
            "{}",
            delete_missing.body
        );

        // 列表查询的 status 参数非法 → 400。
        let bad_filter = app
            .admin_get("/api/admin/galleries?status=live", &owner_cookie)
            .await?;
        ensure!(
            bad_filter.status == StatusCode::BAD_REQUEST,
            "{}",
            bad_filter.body
        );
        ensure!(bad_filter.body["error"]["code"] == "INVALID_GALLERY_STATUS");

        // 更新为已占用的 slug → 409 GALLERY_CONFLICT。
        let alpha_id = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "alpha",
            "Alpha",
            "draft",
            0,
        )
        .await?;
        let beta_id =
            create_gallery(&app, &owner_cookie, category_id, "beta", "Beta", "draft", 1).await?;
        let conflict = app
            .admin_put(
                &format!("/api/admin/galleries/{beta_id}"),
                serde_json::json!({
                    "category_id": category_id, "slug": "alpha", "title": "Beta",
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(conflict.status == StatusCode::CONFLICT, "{}", conflict.body);
        ensure!(conflict.body["error"]["code"] == "GALLERY_CONFLICT");

        // 软删除后 Admin 详情立即 404（deleted_at 排除），重复删除也是 404。
        let deleted = app
            .admin_delete(&format!("/api/admin/galleries/{beta_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let get_deleted = app
            .admin_get(&format!("/api/admin/galleries/{beta_id}"), &owner_cookie)
            .await?;
        ensure!(
            get_deleted.status == StatusCode::NOT_FOUND,
            "{}",
            get_deleted.body
        );
        let delete_again = app
            .admin_delete(&format!("/api/admin/galleries/{beta_id}"), &owner_cookie)
            .await?;
        ensure!(
            delete_again.status == StatusCode::NOT_FOUND,
            "{}",
            delete_again.body
        );

        // alpha 未受影响，仍可读取。
        let alpha = app
            .admin_get(&format!("/api/admin/galleries/{alpha_id}"), &owner_cookie)
            .await?;
        ensure!(alpha.status == StatusCode::OK, "{}", alpha.body);
        ensure!(alpha.body["slug"] == "alpha");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn gallery_list_filters_and_pagination() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let travel_id =
            create_gallery_category(&app, &owner_cookie, "Travel", Some("travel")).await?;
        let city_id = create_gallery_category(&app, &owner_cookie, "City", Some("city")).await?;

        // 三个图库：不同分类、状态与排序（列表按 sort_order, id 升序）。
        create_gallery(
            &app,
            &owner_cookie,
            travel_id,
            "iceland",
            "Iceland",
            "published",
            0,
        )
        .await?;
        create_gallery(&app, &owner_cookie, travel_id, "kyoto", "Kyoto", "draft", 1).await?;
        create_gallery(
            &app,
            &owner_cookie,
            city_id,
            "tokyo",
            "Tokyo",
            "published",
            2,
        )
        .await?;

        // 无过滤：total = 3，按 sort_order 排序。
        let all = app.admin_get("/api/admin/galleries", &owner_cookie).await?;
        ensure!(all.status == StatusCode::OK, "{}", all.body);
        ensure!(all.body["total"].as_i64() == Some(3), "{}", all.body);
        let slugs: Vec<&str> = all.body["items"]
            .as_array()
            .context("missing items")?
            .iter()
            .filter_map(|item| item["slug"].as_str())
            .collect();
        ensure!(slugs == vec!["iceland", "kyoto", "tokyo"], "{slugs:?}");

        // status 过滤。
        let published = app
            .admin_get("/api/admin/galleries?status=published", &owner_cookie)
            .await?;
        ensure!(
            published.body["total"].as_i64() == Some(2),
            "{}",
            published.body
        );
        let draft = app
            .admin_get("/api/admin/galleries?status=draft", &owner_cookie)
            .await?;
        ensure!(draft.body["total"].as_i64() == Some(1), "{}", draft.body);
        ensure!(draft.body["items"][0]["slug"] == "kyoto");

        // category_id 过滤。
        let by_city = app
            .admin_get(
                &format!("/api/admin/galleries?category_id={city_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(
            by_city.body["total"].as_i64() == Some(1),
            "{}",
            by_city.body
        );
        ensure!(by_city.body["items"][0]["slug"] == "tokyo");

        // keyword 同时匹配 title 与 slug（ILIKE）。
        let by_keyword = app
            .admin_get("/api/admin/galleries?keyword=iceland", &owner_cookie)
            .await?;
        ensure!(
            by_keyword.body["total"].as_i64() == Some(1),
            "{}",
            by_keyword.body
        );
        // 纯空白 keyword 被忽略，不过滤。
        let blank_keyword = app
            .admin_get("/api/admin/galleries?keyword=%20%20", &owner_cookie)
            .await?;
        ensure!(
            blank_keyword.body["total"].as_i64() == Some(3),
            "{}",
            blank_keyword.body
        );

        // 分页：page=2&page_size=2 只剩 1 条，分页元数据原样回显。
        let page_two = app
            .admin_get("/api/admin/galleries?page=2&page_size=2", &owner_cookie)
            .await?;
        ensure!(
            page_two.body["total"].as_i64() == Some(3),
            "{}",
            page_two.body
        );
        ensure!(page_two.body["page"].as_u64() == Some(2));
        ensure!(page_two.body["page_size"].as_u64() == Some(2));
        ensure!(
            page_two.body["items"]
                .as_array()
                .context("missing items")?
                .len()
                == 1
        );
        ensure!(page_two.body["items"][0]["slug"] == "tokyo");

        // 边界钳制：page=0 → 1，page_size=0 → 1。
        let clamped = app
            .admin_get("/api/admin/galleries?page=0&page_size=0", &owner_cookie)
            .await?;
        ensure!(clamped.status == StatusCode::OK, "{}", clamped.body);
        ensure!(clamped.body["page"].as_u64() == Some(1));
        ensure!(clamped.body["page_size"].as_u64() == Some(1));
        ensure!(
            clamped.body["items"]
                .as_array()
                .context("missing items")?
                .len()
                == 1
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn gallery_item_errors_and_reorder_atomicity() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let media_a = insert_media_asset(&app, 1, "2026/09/item-a.jpg").await?;
        let media_b = insert_media_asset(&app, 1, "2026/09/item-b.jpg").await?;
        let media_c = insert_media_asset(&app, 1, "2026/09/item-c.jpg").await?;
        let category_id =
            create_gallery_category(&app, &owner_cookie, "Travel", Some("travel")).await?;
        let gallery_id = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "iceland",
            "Iceland",
            "draft",
            0,
        )
        .await?;
        let other_gallery_id = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "kyoto",
            "Kyoto",
            "draft",
            1,
        )
        .await?;

        // 向不存在的图库添加条目 → FK 23503 映射为 404 GALLERY_NOT_FOUND。
        let missing_gallery = app
            .admin_post(
                "/api/admin/galleries/999999/items",
                serde_json::json!({ "media_asset_id": media_a }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            missing_gallery.status == StatusCode::NOT_FOUND,
            "{}",
            missing_gallery.body
        );
        ensure!(missing_gallery.body["error"]["code"] == "GALLERY_NOT_FOUND");

        // 引用不存在的媒体资产 → 404。
        let missing_media = app
            .admin_post(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                serde_json::json!({ "media_asset_id": 999_999 }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            missing_media.status == StatusCode::NOT_FOUND,
            "{}",
            missing_media.body
        );

        // 不存在图库的条目列表 → 404（而非空列表，避免歧义）。
        let items_missing = app
            .admin_get("/api/admin/galleries/999999/items", &owner_cookie)
            .await?;
        ensure!(
            items_missing.status == StatusCode::NOT_FOUND,
            "{}",
            items_missing.body
        );

        // 更新 / 删除不存在的条目 → 404 GALLERY_ITEM_NOT_FOUND。
        let update_missing = app
            .admin_put(
                &format!("/api/admin/galleries/{gallery_id}/items/999999"),
                serde_json::json!({ "alt": "x", "location": "", "sort_order": 0 }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            update_missing.status == StatusCode::NOT_FOUND,
            "{}",
            update_missing.body
        );
        ensure!(update_missing.body["error"]["code"] == "GALLERY_ITEM_NOT_FOUND");
        let remove_missing = app
            .admin_delete(
                &format!("/api/admin/galleries/{gallery_id}/items/999999"),
                &owner_cookie,
            )
            .await?;
        ensure!(
            remove_missing.status == StatusCode::NOT_FOUND,
            "{}",
            remove_missing.body
        );

        let item_a = add_item(&app, &owner_cookie, gallery_id, media_a, "A", "", 0).await?;
        let item_b = add_item(&app, &owner_cookie, gallery_id, media_b, "B", "", 1).await?;
        let item_c = add_item(&app, &owner_cookie, other_gallery_id, media_c, "C", "", 0).await?;

        // 排序请求包含不存在的条目 → 404，且事务回滚：已有 sort_order 不变。
        let reorder_missing = app
            .admin_put(
                &format!("/api/admin/galleries/{gallery_id}/items/order"),
                serde_json::json!({ "item_ids": [item_b, 999_999] }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            reorder_missing.status == StatusCode::NOT_FOUND,
            "{}",
            reorder_missing.body
        );
        ensure!(reorder_missing.body["error"]["code"] == "GALLERY_ITEM_NOT_FOUND");
        let items = app
            .admin_get(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                &owner_cookie,
            )
            .await?;
        let rows = items.body.as_array().context("missing items")?;
        ensure!(rows[0]["id"].as_i64() == Some(item_a), "{rows:?}");
        ensure!(rows[0]["sort_order"].as_i64() == Some(0), "{rows:?}");
        ensure!(rows[1]["id"].as_i64() == Some(item_b), "{rows:?}");
        ensure!(rows[1]["sort_order"].as_i64() == Some(1), "{rows:?}");

        // 排序请求携带其他图库的条目 → 404（gallery_id 作用域隔离），目标条目不受影响。
        let reorder_foreign = app
            .admin_put(
                &format!("/api/admin/galleries/{gallery_id}/items/order"),
                serde_json::json!({ "item_ids": [item_c] }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            reorder_foreign.status == StatusCode::NOT_FOUND,
            "{}",
            reorder_foreign.body
        );
        let other_items = app
            .admin_get(
                &format!("/api/admin/galleries/{other_gallery_id}/items"),
                &owner_cookie,
            )
            .await?;
        ensure!(
            other_items.body[0]["sort_order"].as_i64() == Some(0),
            "{}",
            other_items.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn gallery_category_crud_and_reference_protection() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 初始为空列表。
        let empty = app
            .admin_get("/api/admin/galleries/categories", &owner_cookie)
            .await?;
        ensure!(empty.status == StatusCode::OK, "{}", empty.body);
        ensure!(
            empty.body.as_array().is_some_and(Vec::is_empty),
            "{}",
            empty.body
        );

        // 不传 slug 时回退为 name 的规范化结果。
        let created = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "Landscapes" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        ensure!(created.body["slug"] == "landscapes", "{}", created.body);
        ensure!(created.body["kind"] == "gallery");
        let category_id = created.body["id"].as_i64().context("missing category id")?;

        // 同 kind 下 slug 重复 → 409 TAXONOMY_CONFLICT。
        let duplicate = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "Again", "slug": "landscapes" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            duplicate.status == StatusCode::CONFLICT,
            "{}",
            duplicate.body
        );
        ensure!(duplicate.body["error"]["code"] == "TAXONOMY_CONFLICT");

        // 空白 name → 400 INVALID_TAXONOMY_NAME。
        let blank_name = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "   " }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            blank_name.status == StatusCode::BAD_REQUEST,
            "{}",
            blank_name.body
        );
        ensure!(blank_name.body["error"]["code"] == "INVALID_TAXONOMY_NAME");

        // kind 隔离：文章分类可用同名 slug，且不出现在图库分类列表中。
        let article_category = app
            .admin_post(
                "/api/admin/categories",
                serde_json::json!({ "name": "Landscapes", "slug": "landscapes" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            article_category.status == StatusCode::CREATED,
            "{}",
            article_category.body
        );
        let article_category_id = article_category.body["id"]
            .as_i64()
            .context("missing category id")?;
        let gallery_categories = app
            .admin_get("/api/admin/galleries/categories", &owner_cookie)
            .await?;
        let ids: Vec<i64> = gallery_categories
            .body
            .as_array()
            .context("missing categories")?
            .iter()
            .filter_map(|category| category["id"].as_i64())
            .collect();
        ensure!(ids == vec![category_id], "{ids:?}");

        // 更新名称与 slug。
        let updated = app
            .admin_put(
                &format!("/api/admin/galleries/categories/{category_id}"),
                serde_json::json!({ "name": "Nature", "slug": "nature" }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);
        ensure!(updated.body["name"] == "Nature");
        ensure!(updated.body["slug"] == "nature");

        // 更新不存在 / 其他 kind 的分类 → 404 TAXONOMY_NOT_FOUND。
        let update_missing = app
            .admin_put(
                "/api/admin/galleries/categories/999999",
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
        let update_wrong_kind = app
            .admin_put(
                &format!("/api/admin/galleries/categories/{article_category_id}"),
                serde_json::json!({ "name": "Hijack" }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            update_wrong_kind.status == StatusCode::NOT_FOUND,
            "{}",
            update_wrong_kind.body
        );

        // 删除不存在的分类 → 404。
        let delete_missing = app
            .admin_delete("/api/admin/galleries/categories/999999", &owner_cookie)
            .await?;
        ensure!(
            delete_missing.status == StatusCode::NOT_FOUND,
            "{}",
            delete_missing.body
        );

        // 引用保护：分类被图库使用时删除 → 409 TAXONOMY_IN_USE 且带引用计数。
        let gallery_id = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "iceland",
            "Iceland",
            "draft",
            0,
        )
        .await?;
        let referenced = app
            .admin_delete(
                &format!("/api/admin/galleries/categories/{category_id}"),
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

        // 图库软删除后引用计数归零，但 galleries.category_id 是 FK RESTRICT，
        // 软删除行仍物理引用分类 → 分类物理删除被 23503 阻断，映射为 404 TAXONOMY_NOT_FOUND。
        // （links 表刻意用 ON DELETE SET NULL 避开此问题，见 Migration 注释；此处按现状断言。）
        let delete_gallery = app
            .admin_delete(&format!("/api/admin/galleries/{gallery_id}"), &owner_cookie)
            .await?;
        ensure!(
            delete_gallery.status == StatusCode::NO_CONTENT,
            "{}",
            delete_gallery.body
        );
        let blocked = app
            .admin_delete(
                &format!("/api/admin/galleries/categories/{category_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(blocked.status == StatusCode::NOT_FOUND, "{}", blocked.body);
        ensure!(blocked.body["error"]["code"] == "TAXONOMY_NOT_FOUND");

        // 未被引用的分类可物理删除，并从列表消失。
        let temp_id =
            create_gallery_category(&app, &owner_cookie, "Temporary", Some("temporary")).await?;
        let delete_temp = app
            .admin_delete(
                &format!("/api/admin/galleries/categories/{temp_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(
            delete_temp.status == StatusCode::NO_CONTENT,
            "{}",
            delete_temp.body
        );
        let after_delete = app
            .admin_get("/api/admin/galleries/categories", &owner_cookie)
            .await?;
        let remaining_ids: Vec<i64> = after_delete
            .body
            .as_array()
            .context("missing categories")?
            .iter()
            .filter_map(|category| category["id"].as_i64())
            .collect();
        ensure!(remaining_ids == vec![category_id], "{remaining_ids:?}");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn gallery_write_endpoints_enforce_auth_and_permission() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-w", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-w", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-w", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-w", "moderator-pass-1").await?;
        let category_id =
            create_gallery_category(&app, &owner_cookie, "Travel", Some("travel")).await?;
        let gallery_id = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "iceland",
            "Iceland",
            "draft",
            0,
        )
        .await?;
        let payload = serde_json::json!({
            "category_id": category_id, "slug": "new", "title": "New",
        });

        // 无会话的写请求 → 401（Origin 合法，仅缺 Cookie）。
        let post_anon = app
            .admin_post("/api/admin/galleries", payload.clone(), None)
            .await?;
        ensure!(
            post_anon.status == StatusCode::UNAUTHORIZED,
            "{}",
            post_anon.body
        );
        let post_category_anon = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "Anon" }),
                None,
            )
            .await?;
        ensure!(post_category_anon.status == StatusCode::UNAUTHORIZED);
        let post_item_anon = app
            .admin_post(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                serde_json::json!({ "media_asset_id": 1 }),
                None,
            )
            .await?;
        ensure!(post_item_anon.status == StatusCode::UNAUTHORIZED);
        // PUT/DELETE 走无效会话 Cookie → 401。
        let bogus = "aries_admin_session=not-a-real-token";
        let reorder_anon = app
            .admin_put(
                &format!("/api/admin/galleries/{gallery_id}/items/order"),
                serde_json::json!({ "item_ids": [1] }),
                bogus,
            )
            .await?;
        ensure!(
            reorder_anon.status == StatusCode::UNAUTHORIZED,
            "{}",
            reorder_anon.body
        );
        let delete_anon = app
            .admin_delete(&format!("/api/admin/galleries/{gallery_id}"), bogus)
            .await?;
        ensure!(
            delete_anon.status == StatusCode::UNAUTHORIZED,
            "{}",
            delete_anon.body
        );

        // moderator 无 ManageContent 权限：各类写端点 → 403。
        let post_denied = app
            .admin_post(
                "/api/admin/galleries",
                payload.clone(),
                Some(&moderator_cookie),
            )
            .await?;
        ensure!(
            post_denied.status == StatusCode::FORBIDDEN,
            "{}",
            post_denied.body
        );
        let category_denied = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "Denied" }),
                Some(&moderator_cookie),
            )
            .await?;
        ensure!(category_denied.status == StatusCode::FORBIDDEN);
        let item_denied = app
            .admin_post(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                serde_json::json!({ "media_asset_id": 1 }),
                Some(&moderator_cookie),
            )
            .await?;
        ensure!(item_denied.status == StatusCode::FORBIDDEN);
        let reorder_denied = app
            .admin_put(
                &format!("/api/admin/galleries/{gallery_id}/items/order"),
                serde_json::json!({ "item_ids": [1] }),
                &moderator_cookie,
            )
            .await?;
        ensure!(reorder_denied.status == StatusCode::FORBIDDEN);
        let delete_denied = app
            .admin_delete(
                &format!("/api/admin/galleries/{gallery_id}"),
                &moderator_cookie,
            )
            .await?;
        ensure!(delete_denied.status == StatusCode::FORBIDDEN);

        // editor 拥有 ManageContent：写操作放行。
        let editor_category = app
            .admin_post(
                "/api/admin/galleries/categories",
                serde_json::json!({ "name": "Editor Category" }),
                Some(&editor_cookie),
            )
            .await?;
        ensure!(
            editor_category.status == StatusCode::CREATED,
            "{}",
            editor_category.body
        );
        let editor_gallery = app
            .admin_post("/api/admin/galleries", payload, Some(&editor_cookie))
            .await?;
        ensure!(
            editor_gallery.status == StatusCode::CREATED,
            "{}",
            editor_gallery.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

/// 媒体资产被图库条目引用时，物理删除受 FK RESTRICT 保护：
/// 软删除放行（media_usages 不跟踪 gallery_item），但后台清理任务无法物理移除该行，
/// 直到条目被移除后清理才能真正 purge。
#[tokio::test]
async fn media_referenced_by_gallery_item_survives_physical_purge() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let media_id = insert_media_asset(&app, 1, "2026/09/restricted.jpg").await?;
        let category_id =
            create_gallery_category(&app, &owner_cookie, "Travel", Some("travel")).await?;
        let gallery_id = create_gallery(
            &app,
            &owner_cookie,
            category_id,
            "iceland",
            "Iceland",
            "draft",
            0,
        )
        .await?;
        let item_id = add_item(&app, &owner_cookie, gallery_id, media_id, "Glacier", "", 0).await?;

        // 软删除放行：图库引用不登记在 media_usages，无 409。
        let soft_deleted = app
            .admin_delete(&format!("/api/admin/media/{media_id}"), &owner_cookie)
            .await?;
        ensure!(
            soft_deleted.status == StatusCode::NO_CONTENT,
            "{}",
            soft_deleted.body
        );

        // 后台清理任务运行后：FK RESTRICT 阻断物理删除，资产行仍在。
        aries_server::worker::run_pending_jobs(&app.state).await;
        let (remaining,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM media_assets WHERE id = $1")
                .bind(media_id)
                .fetch_one(&app.state.database)
                .await?;
        ensure!(
            remaining == 1,
            "FK RESTRICT must block physical purge of referenced media"
        );

        // 条目仍在，但媒体摘要因软删除而为 null。
        let items = app
            .admin_get(
                &format!("/api/admin/galleries/{gallery_id}/items"),
                &owner_cookie,
            )
            .await?;
        ensure!(items.status == StatusCode::OK, "{}", items.body);
        ensure!(items.body[0]["id"].as_i64() == Some(item_id));
        ensure!(items.body[0]["media"].is_null(), "{}", items.body);

        // 移除条目后引用解除，再次清理即可物理删除。
        let removed = app
            .admin_delete(
                &format!("/api/admin/galleries/{gallery_id}/items/{item_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(removed.status == StatusCode::NO_CONTENT, "{}", removed.body);
        app.state
            .jobs
            .enqueue(aries_core::jobs::NewBackgroundJob::new(
                aries_core::jobs::JobKind::MediaCleanup,
                serde_json::json!({}),
            ))
            .await
            .context("enqueue media cleanup")?;
        aries_server::worker::run_pending_jobs(&app.state).await;
        let (remaining,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM media_assets WHERE id = $1")
                .bind(media_id)
                .fetch_one(&app.state.database)
                .await?;
        ensure!(
            remaining == 0,
            "media row must be purged after the gallery item is removed"
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
