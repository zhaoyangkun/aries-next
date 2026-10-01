//! 媒体库与站点设置的 HTTP 级集成测试：上传校验、引用维护、软删除清理、Markdown 导入、SSRF 防护。

mod common;

use anyhow::{Context, ensure};
use axum::http::{StatusCode, header};
use base64::{Engine as _, engine::general_purpose::STANDARD};

use common::{MEDIA_PUBLIC_BASE_URL, MultipartFile, TestApp, maybe_app};

/// 1x1 透明 PNG，合法图片，尺寸探测应得到 1x1。
fn png_bytes() -> Vec<u8> {
    STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==")
        .expect("embedded png fixture must decode")
}

fn multipart_file(filename: &str, content_type: &str, data: Vec<u8>) -> MultipartFile {
    MultipartFile::new(filename, content_type, data)
}

async fn upload_one(
    app: &TestApp,
    cookie: &str,
    filename: &str,
) -> anyhow::Result<serde_json::Value> {
    let response = app
        .admin_post_multipart(
            "/api/admin/media",
            vec![multipart_file(filename, "image/png", png_bytes())],
            cookie,
        )
        .await?;
    ensure!(
        response.status == StatusCode::CREATED,
        "upload failed: {}",
        response.body
    );
    let items = response
        .body
        .as_array()
        .context("upload response must be an array")?;
    ensure!(items.len() == 1, "upload must return exactly one asset");
    Ok(items[0].clone())
}

#[tokio::test]
async fn media_upload_list_update_usage_soft_delete_and_cleanup_flow() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    // 上传两个文件：第二个与第一个内容相同，响应应带 duplicate_of 提示。
    let first = upload_one(&app, &cookie, "cover.png").await?;
    ensure!(
        first["url"]
            .as_str()
            .is_some_and(|url| url.starts_with(MEDIA_PUBLIC_BASE_URL))
    );
    ensure!(
        first["width"] == 1 && first["height"] == 1,
        "png dimensions must be probed"
    );
    ensure!(first["status"] == "active");
    let first_id = first["id"].as_i64().context("asset id")?;
    let first_key = first["object_key"]
        .as_str()
        .context("object key")?
        .to_owned();

    let second = upload_one(&app, &cookie, "body.png").await?;
    ensure!(
        second["duplicate_of"] == first_id,
        "same bytes must report duplicate_of"
    );
    let second_id = second["id"].as_i64().context("asset id")?;
    let second_url = second["url"].as_str().context("asset url")?.to_owned();
    let second_key = second["object_key"]
        .as_str()
        .context("object key")?
        .to_owned();
    ensure!(
        first_key != second_key,
        "object keys must be unpredictable and unique"
    );

    // 列表：分页 + keyword 过滤。
    let list = app
        .admin_get("/api/admin/media?page=1&page_size=10&keyword=body", &cookie)
        .await?;
    ensure!(list.status == StatusCode::OK, "list failed: {}", list.body);
    ensure!(list.body["total"] == 1);
    ensure!(list.body["items"][0]["original_name"] == "body.png");

    // 详情与更新。
    let detail = app
        .admin_get(&format!("/api/admin/media/{first_id}"), &cookie)
        .await?;
    ensure!(detail.status == StatusCode::OK);
    let updated = app
        .admin_put(
            &format!("/api/admin/media/{first_id}"),
            serde_json::json!({ "alt": "封面图", "original_name": "cover-renamed.png" }),
            &cookie,
        )
        .await?;
    ensure!(
        updated.status == StatusCode::OK,
        "update failed: {}",
        updated.body
    );
    ensure!(updated.body["alt"] == "封面图");
    ensure!(updated.body["original_name"] == "cover-renamed.png");

    // 静态文件：可读、带长缓存；路径穿越与未知 Key 一律 404。
    let (status, headers, bytes) = app
        .get_raw(&format!("{MEDIA_PUBLIC_BASE_URL}/{first_key}"))
        .await?;
    ensure!(status == StatusCode::OK, "static file must be served");
    ensure!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            == Some("image/png")
    );
    ensure!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|value| value.contains("immutable"))
    );
    ensure!(bytes == png_bytes());
    let (status, _, _) = app.get_raw("/api/media/files/../secret.png").await?;
    ensure!(
        status == StatusCode::NOT_FOUND,
        "traversal must be rejected"
    );
    let (status, _, _) = app.get_raw("/api/media/files/2026/08/missing.png").await?;
    ensure!(status == StatusCode::NOT_FOUND, "unknown key must be 404");

    // 按需缩略图：上传一张 400x200 大图，?w=100 应生成 100x50 小图并缓存，非法 w 回退原图。
    let mut big_png = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(400, 200, image::Rgb([9, 9, 9])))
        .write_to(
            &mut std::io::Cursor::new(&mut big_png),
            image::ImageFormat::Png,
        )?;
    let uploaded = app
        .admin_post_multipart(
            "/api/admin/media",
            vec![multipart_file("big.png", "image/png", big_png.clone())],
            &cookie,
        )
        .await?;
    ensure!(
        uploaded.status == StatusCode::CREATED,
        "upload failed: {}",
        uploaded.body
    );
    let big_key = uploaded.body[0]["object_key"]
        .as_str()
        .context("object key")?
        .to_owned();

    let (status, headers, thumb) = app
        .get_raw(&format!("{MEDIA_PUBLIC_BASE_URL}/{big_key}?w=100"))
        .await?;
    ensure!(status == StatusCode::OK, "thumbnail must be served");
    ensure!(
        thumb.len() < big_png.len(),
        "thumbnail must be smaller than original"
    );
    let decoded = image::load_from_memory(&thumb).context("decode thumbnail")?;
    ensure!(
        decoded.width() == 100 && decoded.height() == 50,
        "thumbnail must be 100x50, got {}x{}",
        decoded.width(),
        decoded.height()
    );
    ensure!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            == Some("image/png")
    );
    ensure!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|value| value.contains("immutable"))
    );
    // 缓存命中：再次请求内容一致
    let (status, _, thumb_again) = app
        .get_raw(&format!("{MEDIA_PUBLIC_BASE_URL}/{big_key}?w=100"))
        .await?;
    ensure!(status == StatusCode::OK);
    ensure!(thumb_again == thumb, "cached thumbnail must be identical");
    // 非法 w 静默回退原图
    let (status, _, bytes) = app
        .get_raw(&format!("{MEDIA_PUBLIC_BASE_URL}/{big_key}?w=99999"))
        .await?;
    ensure!(status == StatusCode::OK);
    ensure!(bytes == big_png, "invalid w must fall back to original");

    // 文章引用：cover + 正文图片，Usages 自动重建。
    let article = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({
                "title": "Uses Media",
                "markdown_source": format!("正文图片 ![x]({second_url})"),
                "cover_url": first["url"].as_str(),
            }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        article.status == StatusCode::CREATED,
        "article create failed: {}",
        article.body
    );
    let article_id = article.body["id"].as_i64().context("article id")?;

    let cover_usages = app
        .admin_get(&format!("/api/admin/media/{first_id}/usages"), &cookie)
        .await?;
    ensure!(cover_usages.body.as_array().is_some_and(|usages| {
        usages.iter().any(|usage| {
            usage["target_type"] == "article_cover" && usage["target_id"] == article_id
        })
    }));
    let content_usages = app
        .admin_get(&format!("/api/admin/media/{second_id}/usages"), &cookie)
        .await?;
    ensure!(content_usages.body.as_array().is_some_and(|usages| {
        usages.iter().any(|usage| {
            usage["target_type"] == "article_content" && usage["target_id"] == article_id
        })
    }));

    // 被引用的资产删除必须 409，并返回引用摘要。
    let blocked = app
        .admin_delete(&format!("/api/admin/media/{second_id}"), &cookie)
        .await?;
    ensure!(
        blocked.status == StatusCode::CONFLICT,
        "referenced delete must be 409: {}",
        blocked.body
    );
    ensure!(
        blocked.body["error"]["details"]["usages"]
            .as_array()
            .is_some_and(|u| !u.is_empty())
    );

    // 文章更新去掉引用后，删除放行（软删除）。
    let update = app
        .admin_put(
            &format!("/api/admin/articles/{article_id}"),
            serde_json::json!({
                "title": "Uses Media",
                "markdown_source": "不再引用任何图片",
                "expected_version": 1,
            }),
            &cookie,
        )
        .await?;
    ensure!(
        update.status == StatusCode::OK,
        "article update failed: {}",
        update.body
    );
    let usages_after = app
        .admin_get(&format!("/api/admin/media/{second_id}/usages"), &cookie)
        .await?;
    ensure!(usages_after.body.as_array().is_some_and(|u| u.is_empty()));

    let deleted = app
        .admin_delete(&format!("/api/admin/media/{second_id}"), &cookie)
        .await?;
    ensure!(
        deleted.status == StatusCode::NO_CONTENT,
        "delete failed: {}",
        deleted.body
    );
    let gone = app
        .admin_get(&format!("/api/admin/media/{second_id}"), &cookie)
        .await?;
    ensure!(
        gone.status == StatusCode::NOT_FOUND,
        "soft-deleted asset must be invisible"
    );
    // 软删除后静态路由即不可见（资产非 Active 不再发文件），清理任务负责物理移除。
    let (status, _, _) = app
        .get_raw(&format!("{MEDIA_PUBLIC_BASE_URL}/{second_key}"))
        .await?;
    ensure!(
        status == StatusCode::NOT_FOUND,
        "soft-deleted asset file must not be served"
    );

    // 后台清理任务：物理删除文件与行。
    aries_server::worker::run_pending_jobs(&app.state).await;
    ensure!(
        !app.media_dir.join(&second_key).exists(),
        "cleanup job must remove the file from disk"
    );
    let purged = app
        .admin_get(&format!("/api/admin/media/{second_id}"), &cookie)
        .await?;
    ensure!(purged.status == StatusCode::NOT_FOUND);

    app.cleanup().await
}

#[tokio::test]
async fn media_upload_rejects_disguised_and_oversized_batches() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    // 文本伪装 PNG：Magic Bytes 不匹配必须拒绝。
    let disguised = app
        .admin_post_multipart(
            "/api/admin/media",
            vec![multipart_file(
                "fake.png",
                "image/png",
                b"hello world".to_vec(),
            )],
            &cookie,
        )
        .await?;
    ensure!(
        disguised.status == StatusCode::BAD_REQUEST,
        "disguised file must be rejected: {}",
        disguised.body
    );

    // 声明 MIME 与内容不符。
    let mismatch = app
        .admin_post_multipart(
            "/api/admin/media",
            vec![multipart_file("real.png", "image/gif", png_bytes())],
            &cookie,
        )
        .await?;
    ensure!(mismatch.status == StatusCode::BAD_REQUEST);

    // 超过每批 5 个文件。
    let oversized_batch = app
        .admin_post_multipart(
            "/api/admin/media",
            (0..6)
                .map(|index| multipart_file(&format!("f{index}.png"), "image/png", png_bytes()))
                .collect(),
            &cookie,
        )
        .await?;
    ensure!(oversized_batch.status == StatusCode::BAD_REQUEST);

    // 空批次。
    let empty = app
        .admin_post_multipart("/api/admin/media", Vec::new(), &cookie)
        .await?;
    ensure!(empty.status == StatusCode::BAD_REQUEST);

    app.cleanup().await
}

/// 直接造一个指定 Role 的用户（生成真实 Argon2 Hash 入库）。
async fn create_user(
    app: &TestApp,
    username: &str,
    role: &str,
    password: &str,
) -> anyhow::Result<()> {
    let password_hash = app
        .state
        .passwords
        .hash(password)
        .context("failed to hash test password")?;
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
    .await
    .context("insert test user")?;
    Ok(())
}

/// 走真实 login 端点换 Session Cookie。
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
async fn media_batch_delete_partial_success_and_permissions() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    // 三个可删资产（不同文件名）+ 一个被文章引用的资产。
    let mut asset_ids = Vec::new();
    for index in 0..3 {
        let asset = upload_one(&app, &cookie, &format!("batch-{index}.png")).await?;
        asset_ids.push(asset["id"].as_i64().context("asset id")?);
    }
    // 上传第 4 个资产并写入文章正文，使其带引用。
    let referenced_asset = upload_one(&app, &cookie, "referenced.png").await?;
    let referenced_id = referenced_asset["id"].as_i64().context("asset id")?;
    let article = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({
                "title": "Keeps A Reference",
                "markdown_source": format!("![x]({})", referenced_asset["url"]),
            }),
            Some(&cookie),
        )
        .await?;
    ensure!(article.status == StatusCode::CREATED, "{}", article.body);

    // 混合批量删除：3 个可删 + 1 个被引用 + 2 个不存在 + 1 个重复 ID（去重后归一次类）。
    let mixed = app
        .admin_post(
            "/api/admin/media/batch-delete",
            serde_json::json!({ "ids": [
                asset_ids[0],
                asset_ids[1],
                asset_ids[2],
                referenced_id,
                99998,
                99997,
                asset_ids[0],
            ] }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        mixed.status == StatusCode::OK,
        "batch delete failed: {}",
        mixed.body
    );
    let deleted = mixed.body["deleted"].as_array().context("deleted list")?;
    let referenced = mixed.body["referenced"]
        .as_array()
        .context("referenced list")?;
    let not_found = mixed.body["not_found"]
        .as_array()
        .context("not_found list")?;
    ensure!(deleted.len() == 3, "expected 3 deleted: {}", mixed.body);
    ensure!(
        referenced
            .iter()
            .any(|id| *id == serde_json::json!(referenced_id)),
        "referenced asset must be skipped: {}",
        mixed.body
    );
    ensure!(
        not_found
            .iter()
            .all(|id| id == &serde_json::json!(99997) || id == &serde_json::json!(99998)),
        "not_found must contain exactly the missing ids: {}",
        mixed.body
    );
    ensure!(not_found.len() == 2);
    // 被引用资产必须保持 active。
    let still_active = app
        .admin_get(&format!("/api/admin/media/{referenced_id}"), &cookie)
        .await?;
    ensure!(
        still_active.body["status"] == "active",
        "referenced asset must stay active: {}",
        still_active.body
    );
    // 已删资产不可见。
    let gone = app
        .admin_get(&format!("/api/admin/media/{}", asset_ids[0]), &cookie)
        .await?;
    ensure!(gone.status == StatusCode::NOT_FOUND, "{}", gone.body);

    // 空批次与超上限批次必须 400（JSON 错误体）。
    let empty = app
        .admin_post(
            "/api/admin/media/batch-delete",
            serde_json::json!({ "ids": [] }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        empty.status == StatusCode::BAD_REQUEST,
        "empty batch must be rejected: {}",
        empty.body
    );
    let oversized = app
        .admin_post(
            "/api/admin/media/batch-delete",
            serde_json::json!({ "ids": (0..101).collect::<Vec<_>>() }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        oversized.status == StatusCode::BAD_REQUEST,
        "oversized batch must be rejected: {}",
        oversized.body
    );
    ensure!(
        oversized.body["error"]["code"] == "INVALID_BATCH_DELETE",
        "oversized batch must carry INVALID_BATCH_DELETE: {}",
        oversized.body
    );

    // 权限矩阵：Moderator 无 ManageContent → 403。
    create_user(&app, "moderator-bd", "moderator", "moderator-pass-1").await?;
    let moderator_cookie = login_cookie(&app, "moderator-bd", "moderator-pass-1").await?;
    let denied = app
        .admin_post(
            "/api/admin/media/batch-delete",
            serde_json::json!({ "ids": [referenced_id] }),
            Some(&moderator_cookie),
        )
        .await?;
    ensure!(
        denied.status == StatusCode::FORBIDDEN,
        "moderator must be denied: {}",
        denied.body
    );

    // 审计：批量删除必须落 media.batch_delete 事件且带逐项结果。
    let audit = app
        .admin_get("/api/admin/audit-logs?page=1&page_size=20", &cookie)
        .await?;
    ensure!(audit.status == StatusCode::OK, "{}", audit.body);
    let audit_items = audit.body["items"].as_array().context("audit items")?;
    ensure!(
        audit_items
            .iter()
            .any(|entry| entry["action"] == "media.batch_delete"
                && entry["metadata"]["deleted"]
                    .as_array()
                    .is_some_and(|v| v.len() == 3)),
        "batch delete audit must record per-item results: {}",
        audit.body
    );

    app.cleanup().await
}

#[tokio::test]
async fn remote_upload_rejects_internal_and_non_http_urls() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    for url in [
        "http://127.0.0.1:9/x.png",
        "http://10.0.0.8/x.png",
        "http://172.16.0.1/x.png",
        "http://192.168.1.1/x.png",
        "http://169.254.169.254/latest/meta-data",
        "http://localhost:9/x.png",
        "http://[::1]/x.png",
        "file:///etc/passwd",
        "ftp://example.com/x.png",
        "not-a-url",
    ] {
        let response = app
            .admin_post(
                "/api/admin/media/remote",
                serde_json::json!({ "url": url }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            response.status == StatusCode::BAD_REQUEST,
            "{url} must be rejected, got {}: {}",
            response.status,
            response.body
        );
    }

    app.cleanup().await
}

#[tokio::test]
async fn markdown_import_preview_conflict_and_commit_strategies() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    // 已存在的文章占据 slug hello-world。
    let existing = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({ "title": "Existing", "slug": "hello-world" }),
            Some(&cookie),
        )
        .await?;
    ensure!(existing.status == StatusCode::CREATED);

    let conflicting_md = b"---\ntitle: Hello\nslug: hello-world\ntags:\n  - rust\n  - brand-new-tag\nsummary: hi\n---\n# Body".to_vec();
    let fresh_md = b"---\ntitle: Fresh Post\nslug: fresh-post\n---\nbody".to_vec();
    let import = app
        .admin_post_multipart(
            "/api/admin/articles/imports",
            vec![
                multipart_file("hello.md", "text/markdown", conflicting_md.clone()),
                multipart_file("fresh.md", "text/markdown", fresh_md),
            ],
            &cookie,
        )
        .await?;
    ensure!(
        import.status == StatusCode::CREATED,
        "import failed: {}",
        import.body
    );
    let job_id = import.body["job_id"].as_i64().context("job id")?;
    let items = import.body["items"].as_array().context("items")?;
    ensure!(items.len() == 2);
    ensure!(
        items[0]["slug_conflict"] == true,
        "existing slug must be flagged"
    );
    ensure!(items[0]["tags"].as_array().is_some_and(|t| t.len() == 2));
    ensure!(items[1]["slug_conflict"] == false);

    // 预览端点返回同一内容。
    let preview = app
        .admin_get(&format!("/api/admin/imports/{job_id}"), &cookie)
        .await?;
    ensure!(preview.status == StatusCode::OK);
    ensure!(preview.body["status"] == "pending");
    ensure!(preview.body["items"][0]["slug"] == "hello-world");

    // Commit：冲突项 skip。
    let commit = app
        .admin_post(
            &format!("/api/admin/imports/{job_id}/commit"),
            serde_json::json!({ "items": [{ "index": 0, "strategy": "skip" }] }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        commit.status == StatusCode::OK,
        "commit failed: {}",
        commit.body
    );
    ensure!(
        commit.body["skipped"]
            .as_array()
            .is_some_and(|s| s.iter().any(|v| v == "hello-world"))
    );
    let created = commit.body["created"].as_array().context("created")?;
    ensure!(created.len() == 1 && created[0]["slug"] == "fresh-post");

    // 导入的文章必须是草稿，绝不自动发布。
    let created_id = created[0]["id"].as_i64().context("created id")?;
    let article = app
        .admin_get(&format!("/api/admin/articles/{created_id}"), &cookie)
        .await?;
    ensure!(article.status == StatusCode::OK);
    ensure!(
        article.body["status"] == "draft",
        "imported article must stay draft"
    );

    // 导入自动创建的标签应出现在标签列表中（本次 commit 只落了 fresh-post，不带标签）。
    let tags = app.admin_get("/api/admin/tags", &cookie).await?;
    ensure!(tags.status == StatusCode::OK);

    // 重复 Commit 必须 409。
    let again = app
        .admin_post(
            &format!("/api/admin/imports/{job_id}/commit"),
            serde_json::json!({ "items": [] }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        again.status == StatusCode::CONFLICT,
        "double commit must be rejected"
    );

    // 第二次导入同一文件，rename 策略自动加后缀。
    let second_import = app
        .admin_post_multipart(
            "/api/admin/articles/imports",
            vec![multipart_file("hello.md", "text/markdown", conflicting_md)],
            &cookie,
        )
        .await?;
    ensure!(second_import.status == StatusCode::CREATED);
    let second_job = second_import.body["job_id"].as_i64().context("job id")?;
    let second_commit = app
        .admin_post(
            &format!("/api/admin/imports/{second_job}/commit"),
            serde_json::json!({ "items": [{ "index": 0, "strategy": "rename" }] }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        second_commit.status == StatusCode::OK,
        "rename commit failed: {}",
        second_commit.body
    );
    ensure!(second_commit.body["renamed"].as_array().is_some_and(|r| {
        r.iter()
            .any(|entry| entry["from"] == "hello-world" && entry["to"] == "hello-world-2")
    }));
    let renamed = second_commit.body["created"]
        .as_array()
        .context("created")?;
    ensure!(renamed.len() == 1 && renamed[0]["slug"] == "hello-world-2");

    // rename 提交的文章带标签：brand-new-tag 应被自动创建。
    let tags_after = app.admin_get("/api/admin/tags", &cookie).await?;
    ensure!(
        tags_after
            .body
            .as_array()
            .is_some_and(|tags| tags.iter().any(|tag| tag["name"] == "brand-new-tag"))
    );

    app.cleanup().await
}

#[tokio::test]
async fn remote_upload_fetch_failures_return_clear_error_codes() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    // 非法 URL：静态校验阶段即拒绝，不发任何网络请求，错误码必须是 REMOTE_URL_INVALID。
    for url in [
        "http://",
        "https://",
        "//example.com/x.png",
        "http://exa mple.com/x.png",
    ] {
        let response = app
            .admin_post(
                "/api/admin/media/remote",
                serde_json::json!({ "url": url }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            response.status == StatusCode::BAD_REQUEST,
            "{url} must be rejected, got {}: {}",
            response.status,
            response.body
        );
        ensure!(
            response.body["error"]["code"] == "REMOTE_URL_INVALID",
            "{url} must carry REMOTE_URL_INVALID: {}",
            response.body
        );
    }

    // 不可达地址：`.invalid` 是 RFC 2606 保留 TLD，永不解析，
    // DNS 失败必须映射为 REMOTE_FETCH_FAILED 而非 500。
    let unresolved = app
        .admin_post(
            "/api/admin/media/remote",
            serde_json::json!({ "url": "http://media-test-unreachable.invalid/x.png" }),
            Some(&cookie),
        )
        .await?;
    ensure!(
        unresolved.status == StatusCode::BAD_REQUEST,
        "unresolvable host must be 400: {}",
        unresolved.body
    );
    ensure!(
        unresolved.body["error"]["code"] == "REMOTE_FETCH_FAILED",
        "unresolvable host must carry REMOTE_FETCH_FAILED: {}",
        unresolved.body
    );

    // 失败的远端抓取不得留下任何资产记录。
    let list = app
        .admin_get("/api/admin/media?page=1&page_size=10", &cookie)
        .await?;
    ensure!(
        list.body["total"] == 0,
        "failed remote fetches must not create assets: {}",
        list.body
    );

    app.cleanup().await
}

#[tokio::test]
async fn media_batch_delete_partial_cleanup_and_referenced_survives() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    // 一个可删资产 + 一个被文章正文引用的资产：部分成功语义下各归各类。
    let deletable = upload_one(&app, &cookie, "bd-deletable.png").await?;
    let deletable_id = deletable["id"].as_i64().context("asset id")?;
    let deletable_key = deletable["object_key"]
        .as_str()
        .context("object key")?
        .to_owned();
    let referenced = upload_one(&app, &cookie, "bd-referenced.png").await?;
    let referenced_id = referenced["id"].as_i64().context("asset id")?;
    let referenced_key = referenced["object_key"]
        .as_str()
        .context("object key")?
        .to_owned();
    let article = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({
                "title": "Holds Reference",
                "markdown_source": format!("![x]({})", referenced["url"]),
            }),
            Some(&cookie),
        )
        .await?;
    ensure!(article.status == StatusCode::CREATED, "{}", article.body);

    let response = app
        .admin_post(
            "/api/admin/media/batch-delete",
            serde_json::json!({ "ids": [deletable_id, referenced_id] }),
            Some(&cookie),
        )
        .await?;
    ensure!(response.status == StatusCode::OK, "{}", response.body);
    ensure!(response.body["deleted"] == serde_json::json!([deletable_id]));
    ensure!(response.body["referenced"] == serde_json::json!([referenced_id]));
    ensure!(
        response.body["not_found"]
            .as_array()
            .is_some_and(|v| v.is_empty()),
        "no id should be reported missing: {}",
        response.body
    );

    // 软删除立即生效：详情 404、静态文件停止服务，但磁盘文件要等清理任务回收。
    let detail = app
        .admin_get(&format!("/api/admin/media/{deletable_id}"), &cookie)
        .await?;
    ensure!(detail.status == StatusCode::NOT_FOUND);
    let (status, _, _) = app
        .get_raw(&format!("{MEDIA_PUBLIC_BASE_URL}/{deletable_key}"))
        .await?;
    ensure!(status == StatusCode::NOT_FOUND);
    ensure!(
        app.media_dir.join(&deletable_key).exists(),
        "soft delete must keep the file on disk until cleanup runs"
    );

    // 被引用资产完全不受影响：详情可见、文件仍可访问。
    let kept = app
        .admin_get(&format!("/api/admin/media/{referenced_id}"), &cookie)
        .await?;
    ensure!(kept.status == StatusCode::OK && kept.body["status"] == "active");
    let (status, _, _) = app
        .get_raw(&format!("{MEDIA_PUBLIC_BASE_URL}/{referenced_key}"))
        .await?;
    ensure!(status == StatusCode::OK);

    // 清理任务只物理清除可删资产，被引用资产保留行与文件。
    aries_server::worker::run_pending_jobs(&app.state).await;
    ensure!(
        !app.media_dir.join(&deletable_key).exists(),
        "cleanup job must remove the deletable file from disk"
    );
    ensure!(
        app.media_dir.join(&referenced_key).exists(),
        "referenced asset file must survive cleanup"
    );
    let kept_after = app
        .admin_get(&format!("/api/admin/media/{referenced_id}"), &cookie)
        .await?;
    ensure!(kept_after.status == StatusCode::OK);
    let list = app
        .admin_get("/api/admin/media?page=1&page_size=10", &cookie)
        .await?;
    ensure!(
        list.body["total"] == 1,
        "only the referenced asset may remain: {}",
        list.body
    );

    app.cleanup().await
}

#[tokio::test]
async fn media_usages_endpoint_shape_and_guards() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    // 不存在的资产：404 + MEDIA_NOT_FOUND。
    let missing = app
        .admin_get("/api/admin/media/424242/usages", &cookie)
        .await?;
    ensure!(missing.status == StatusCode::NOT_FOUND);
    ensure!(missing.body["error"]["code"] == "MEDIA_NOT_FOUND");

    let asset = upload_one(&app, &cookie, "usage-target.png").await?;
    let asset_id = asset["id"].as_i64().context("asset id")?;
    let asset_url = asset["url"].as_str().context("asset url")?.to_owned();

    // 无引用时返回空数组而非错误。
    let empty = app
        .admin_get(&format!("/api/admin/media/{asset_id}/usages"), &cookie)
        .await?;
    ensure!(empty.status == StatusCode::OK);
    ensure!(empty.body.as_array().is_some_and(|u| u.is_empty()));

    // 同一资产同时作为 Cover 与正文图片：应出现两种 target_type，且响应字段完整。
    let article = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({
                "title": "Both Cover And Content",
                "markdown_source": format!("正文 ![x]({asset_url})"),
                "cover_url": asset_url,
            }),
            Some(&cookie),
        )
        .await?;
    ensure!(article.status == StatusCode::CREATED, "{}", article.body);
    let article_id = article.body["id"].as_i64().context("article id")?;

    let usages = app
        .admin_get(&format!("/api/admin/media/{asset_id}/usages"), &cookie)
        .await?;
    ensure!(usages.status == StatusCode::OK, "{}", usages.body);
    let items = usages.body.as_array().context("usages list")?;
    ensure!(
        items.len() == 2,
        "cover + content must produce two usage rows: {}",
        usages.body
    );
    for usage in items {
        ensure!(usage["id"].as_i64().is_some(), "usage id: {usage}");
        ensure!(usage["asset_id"] == asset_id, "usage asset_id: {usage}");
        ensure!(usage["target_id"] == article_id, "usage target_id: {usage}");
        ensure!(
            usage["created_at"].as_str().is_some(),
            "usage created_at: {usage}"
        );
    }
    ensure!(items.iter().any(|u| u["target_type"] == "article_cover"));
    ensure!(items.iter().any(|u| u["target_type"] == "article_content"));

    // 权限矩阵：Moderator 无 ManageContent → 403。
    create_user(&app, "moderator-usage", "moderator", "moderator-pass-1").await?;
    let moderator_cookie = login_cookie(&app, "moderator-usage", "moderator-pass-1").await?;
    let denied = app
        .admin_get(
            &format!("/api/admin/media/{asset_id}/usages"),
            &moderator_cookie,
        )
        .await?;
    ensure!(
        denied.status == StatusCode::FORBIDDEN,
        "moderator must be denied: {}",
        denied.body
    );

    // 软删除后的资产对 Usages 端点同样不可见（与详情语义一致）。
    let orphan = upload_one(&app, &cookie, "usage-orphan.png").await?;
    let orphan_id = orphan["id"].as_i64().context("asset id")?;
    let deleted = app
        .admin_delete(&format!("/api/admin/media/{orphan_id}"), &cookie)
        .await?;
    ensure!(deleted.status == StatusCode::NO_CONTENT);
    let gone = app
        .admin_get(&format!("/api/admin/media/{orphan_id}/usages"), &cookie)
        .await?;
    ensure!(
        gone.status == StatusCode::NOT_FOUND,
        "usages of a soft-deleted asset must be 404: {}",
        gone.body
    );

    app.cleanup().await
}

#[tokio::test]
async fn media_list_filters_and_pagination_edges() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    // 三个资产：文件名用于 keyword 过滤，全部 local + image/png。
    upload_one(&app, &cookie, "AlphaCover.png").await?;
    upload_one(&app, &cookie, "beta-body.png").await?;
    upload_one(&app, &cookie, "gamma.png").await?;

    // keyword：大小写不敏感子串匹配；首尾空白被裁剪；无命中返回空页。
    let matched = app
        .admin_get("/api/admin/media?keyword=alpha", &cookie)
        .await?;
    ensure!(matched.body["total"] == 1, "{}", matched.body);
    ensure!(matched.body["items"][0]["original_name"] == "AlphaCover.png");
    let upper = app
        .admin_get("/api/admin/media?keyword=ALPHA", &cookie)
        .await?;
    ensure!(upper.body["total"] == 1, "keyword must be case-insensitive");
    let padded = app
        .admin_get("/api/admin/media?keyword=%20alpha%20", &cookie)
        .await?;
    ensure!(
        padded.body["total"] == 1,
        "keyword must be trimmed: {}",
        padded.body
    );
    let blank = app
        .admin_get("/api/admin/media?keyword=%20%20%20", &cookie)
        .await?;
    ensure!(
        blank.body["total"] == 3,
        "blank keyword must not filter: {}",
        blank.body
    );
    let none = app
        .admin_get("/api/admin/media?keyword=no-such-file", &cookie)
        .await?;
    ensure!(none.body["total"] == 0);
    ensure!(none.body["items"].as_array().is_some_and(|v| v.is_empty()));

    // provider 筛选：测试环境全部为 local；非法值必须 400。
    let local = app
        .admin_get("/api/admin/media?provider=local", &cookie)
        .await?;
    ensure!(local.body["total"] == 3, "{}", local.body);
    let s3 = app
        .admin_get("/api/admin/media?provider=s3", &cookie)
        .await?;
    ensure!(s3.body["total"] == 0, "{}", s3.body);
    let bogus = app
        .admin_get("/api/admin/media?provider=bogus", &cookie)
        .await?;
    ensure!(
        bogus.status == StatusCode::BAD_REQUEST,
        "invalid provider must be 400: {}",
        bogus.body
    );
    ensure!(bogus.body["error"]["code"] == "INVALID_MEDIA_PROVIDER");

    // mime 筛选：精确匹配。
    let png = app
        .admin_get("/api/admin/media?mime=image/png", &cookie)
        .await?;
    ensure!(png.body["total"] == 3, "{}", png.body);
    let jpeg = app
        .admin_get("/api/admin/media?mime=image/jpeg", &cookie)
        .await?;
    ensure!(jpeg.body["total"] == 0, "{}", jpeg.body);

    // 分页边界：稳定排序（created_at DESC, id DESC）下后上传的在前。
    let page_one = app
        .admin_get("/api/admin/media?page=1&page_size=2", &cookie)
        .await?;
    let items = page_one.body["items"].as_array().context("items")?;
    ensure!(items.len() == 2 && page_one.body["total"] == 3);
    ensure!(
        items[0]["original_name"] == "gamma.png",
        "latest upload must come first: {}",
        page_one.body
    );
    let page_two = app
        .admin_get("/api/admin/media?page=2&page_size=2", &cookie)
        .await?;
    ensure!(
        page_two.body["items"]
            .as_array()
            .is_some_and(|v| v.len() == 1)
    );
    // 越界页：items 为空但 total 不变。
    let out_of_range = app
        .admin_get("/api/admin/media?page=9&page_size=2", &cookie)
        .await?;
    ensure!(out_of_range.body["total"] == 3);
    ensure!(
        out_of_range.body["items"]
            .as_array()
            .is_some_and(|v| v.is_empty())
    );
    // page=0 收敛到第 1 页；page_size 收敛到 [1, 100]。
    let zero_page = app
        .admin_get("/api/admin/media?page=0&page_size=2", &cookie)
        .await?;
    ensure!(zero_page.body["page"] == 1, "{}", zero_page.body);
    ensure!(
        zero_page.body["items"]
            .as_array()
            .is_some_and(|v| v.len() == 2)
    );
    let zero_size = app
        .admin_get("/api/admin/media?page=1&page_size=0", &cookie)
        .await?;
    ensure!(zero_size.body["page_size"] == 1, "{}", zero_size.body);
    ensure!(
        zero_size.body["items"]
            .as_array()
            .is_some_and(|v| v.len() == 1)
    );
    let huge_size = app
        .admin_get("/api/admin/media?page=1&page_size=1000", &cookie)
        .await?;
    ensure!(huge_size.body["page_size"] == 100, "{}", huge_size.body);
    ensure!(
        huge_size.body["items"]
            .as_array()
            .is_some_and(|v| v.len() == 3)
    );

    // 软删除的资产立即从列表消失，total 同步下降。
    let gamma = page_one.body["items"][0]["id"]
        .as_i64()
        .context("asset id")?;
    let deleted = app
        .admin_delete(&format!("/api/admin/media/{gamma}"), &cookie)
        .await?;
    ensure!(deleted.status == StatusCode::NO_CONTENT);
    let after = app
        .admin_get("/api/admin/media?page=1&page_size=10", &cookie)
        .await?;
    ensure!(
        after.body["total"] == 2,
        "soft-deleted asset must leave the list: {}",
        after.body
    );

    // LIKE 通配符字面量化（ILIKE ESCAPE '\'）：%/_ 不再作为通配符匹配全部，
    // 只命中文件名中确实含字面量 % 的资产。
    upload_one(&app, &cookie, "rate_100%.png").await?;
    let percent = app
        .admin_get("/api/admin/media?keyword=%25", &cookie)
        .await?;
    ensure!(
        percent.body["total"] == 1,
        "keyword '%' must match literally: {}",
        percent.body
    );
    ensure!(percent.body["items"][0]["original_name"] == "rate_100%.png");
    let percent_phrase = app
        .admin_get("/api/admin/media?keyword=rate_100%25", &cookie)
        .await?;
    ensure!(
        percent_phrase.body["total"] == 1,
        "keyword with '%' inside must match literally: {}",
        percent_phrase.body
    );
    ensure!(percent_phrase.body["items"][0]["original_name"] == "rate_100%.png");

    app.cleanup().await
}

#[tokio::test]
async fn site_settings_get_and_validated_update() -> anyhow::Result<()> {
    let Some(app) = maybe_app().await? else {
        return Ok(());
    };
    let cookie = app.bootstrap_owner().await?;

    let current = app.admin_get("/api/admin/site-settings", &cookie).await?;
    ensure!(
        current.status == StatusCode::OK,
        "get failed: {}",
        current.body
    );
    ensure!(current.body["site_name"] == "Aries Next");
    ensure!(current.body["page_size_index"] == 10);

    let updated = app
        .admin_put(
            "/api/admin/site-settings",
            serde_json::json!({
                "site_name": "My Blog",
                "site_description": "记录与分享",
                "site_url": "https://blog.example.com",
                "logo_url": "",
                "icp_text": "",
                "default_cover_url": "",
                "page_size_index": 12,
                "page_size_archive": 20,
                "page_size_search": 10,
            }),
            &cookie,
        )
        .await?;
    ensure!(
        updated.status == StatusCode::OK,
        "update failed: {}",
        updated.body
    );
    ensure!(updated.body["site_name"] == "My Blog");
    ensure!(updated.body["page_size_archive"] == 20);

    // 非法 URL 与越界分页必须 400。
    for payload in [
        serde_json::json!({ "site_name": "X", "site_url": "javascript:alert(1)", "page_size_index": 10, "page_size_archive": 10, "page_size_search": 10 }),
        serde_json::json!({ "site_name": "X", "site_url": "", "page_size_index": 0, "page_size_archive": 10, "page_size_search": 10 }),
        serde_json::json!({ "site_name": "X", "site_url": "", "page_size_index": 101, "page_size_archive": 10, "page_size_search": 10 }),
        serde_json::json!({ "site_name": " ", "site_url": "", "page_size_index": 10, "page_size_archive": 10, "page_size_search": 10 }),
    ] {
        let response = app
            .admin_put("/api/admin/site-settings", payload, &cookie)
            .await?;
        ensure!(
            response.status == StatusCode::BAD_REQUEST,
            "invalid payload must be rejected: {}",
            response.body
        );
    }

    app.cleanup().await
}
