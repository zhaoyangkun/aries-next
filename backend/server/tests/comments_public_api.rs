//! Phase 05 公开端评论 Contract Test：访客发表/回复、已批准评论树、
//! comment_policy 行为、限流、重复提交与隐私边界（email/ip 不出 API）。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

/// 创建并发布一篇文章，返回 (id, slug)。
async fn create_published_article(app: &TestApp, cookie: &str) -> anyhow::Result<(i64, String)> {
    let created = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({ "title": "Commented Post", "markdown_source": "# c" }),
            Some(cookie),
        )
        .await?;
    ensure!(created.status == StatusCode::CREATED, "{}", created.body);
    let id = created.body["id"].as_i64().context("missing article id")?;
    let version = created.body["version"]
        .as_i64()
        .context("missing version")?;
    let slug = created.body["slug"]
        .as_str()
        .context("missing slug")?
        .to_owned();
    let published = app
        .admin_patch(
            &format!("/api/admin/articles/{id}/status"),
            serde_json::json!({ "command": "publish", "expected_version": version }),
            cookie,
        )
        .await?;
    ensure!(published.status == StatusCode::OK, "{}", published.body);
    Ok((id, slug))
}

/// 访客提交评论（可指定来源 IP 以隔离限流窗口）。
async fn submit_comment(
    app: &TestApp,
    ip: &str,
    body: serde_json::Value,
) -> anyhow::Result<common::TestResponse> {
    app.post_with_headers("/api/public/comments", body, &[("x-forwarded-for", ip)])
        .await
}

async fn submit_reply(
    app: &TestApp,
    ip: &str,
    comment_id: i64,
    body: serde_json::Value,
) -> anyhow::Result<common::TestResponse> {
    app.post_with_headers(
        &format!("/api/public/comments/{comment_id}/replies"),
        body,
        &[("x-forwarded-for", ip)],
    )
    .await
}

fn comment_body(target_slug: &str, content: &str) -> serde_json::Value {
    serde_json::json!({
        "target_type": "article",
        "target_slug": target_slug,
        "nickname": "Reader",
        "email": "reader@example.com",
        "content": content,
    })
}

/// Admin 审核：把指定评论置为 approved。
async fn approve_comment(app: &TestApp, cookie: &str, comment_id: i64) -> anyhow::Result<()> {
    let approved = app
        .admin_patch(
            &format!("/api/admin/comments/{comment_id}/status"),
            serde_json::json!({ "status": "approved" }),
            cookie,
        )
        .await?;
    ensure!(approved.status == StatusCode::OK, "{}", approved.body);
    Ok(())
}

#[tokio::test]
async fn public_comment_submit_moderation_and_tree_flow() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let (_article_id, slug) = create_published_article(&app, &owner_cookie).await?;

        // 发表：默认 moderated → pending；Markdown 服务端渲染并 Sanitize。
        let mut body = comment_body(&slug, "Nice post! <script>alert(1)</script>");
        body["website"] = serde_json::json!("https://reader.example.com");
        let created = submit_comment(&app, "10.10.0.1", body).await?;
        ensure!(created.status == StatusCode::OK, "{}", created.body);
        ensure!(created.body["status"] == "pending");
        ensure!(created.body["nickname"] == "Reader");
        // website 是访客公开展示信息，入库前已校验 scheme，原样回传。
        ensure!(created.body["website"] == "https://reader.example.com");
        let html = created.body["content_html"]
            .as_str()
            .context("missing html")?;
        ensure!(html.contains("Nice post!"), "html: {html}");
        ensure!(!html.contains("<script>"), "html: {html}");
        let comment_id = created.body["id"].as_i64().context("missing comment id")?;

        // 评论通知 job 已入队（Worker 当前只记录日志；入队失败不影响创建响应）。
        let latest_job: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT payload FROM background_jobs \
             WHERE kind = 'comment_notification' ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&app.state.database)
        .await?;
        let (payload,) = latest_job.context("comment notification job not enqueued")?;
        ensure!(
            payload["comment_id"].as_i64() == Some(comment_id),
            "job payload must reference the created comment: {payload}"
        );
        ensure!(
            payload["target_type"] == "article"
                && payload["target_id"].as_i64() == Some(_article_id),
            "job payload must carry the comment target: {payload}"
        );

        // 隐私边界：公开响应不含 email/ip/ua 字段。
        ensure!(created.body.get("email").is_none());
        ensure!(created.body.get("ip_hash").is_none());
        // 头像为 Cravatar（email 的 SHA-256），不含明文 email。
        let avatar = created.body["avatar_url"]
            .as_str()
            .context("missing avatar")?;
        ensure!(
            avatar.starts_with("https://cravatar.cn/avatar/"),
            "{avatar}"
        );
        ensure!(!avatar.contains("reader@example.com"));

        // 重复提交（同 email + target + 相同内容）→ 409。
        let duplicate = submit_comment(
            &app,
            "10.10.0.1",
            comment_body(&slug, "Nice post! <script>alert(1)</script>"),
        )
        .await?;
        ensure!(
            duplicate.status == StatusCode::CONFLICT,
            "{}",
            duplicate.body
        );
        ensure!(duplicate.body["error"]["code"] == "COMMENT_DUPLICATE");

        // pending 评论不出现在公开列表。
        let list = app
            .get(&format!("/api/public/articles/{slug}/comments"))
            .await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body["total"].as_i64() == Some(0));

        // 审核通过后出现；分页语义只对根评论生效。
        approve_comment(&app, &owner_cookie, comment_id).await?;
        let list = app
            .get(&format!("/api/public/articles/{slug}/comments"))
            .await?;
        ensure!(list.body["total"].as_i64() == Some(1));
        ensure!(list.body["items"][0]["id"].as_i64() == Some(comment_id));
        ensure!(list.body["items"][0].get("email").is_none());
        ensure!(list.body["items"][0]["is_admin"] == false);
        // 公开列表同样回传 website（回复与根评论一致）。
        ensure!(list.body["items"][0]["website"] == "https://reader.example.com");

        // 回复已批准评论 → pending，公开列表暂不出现。
        let reply = submit_reply(
            &app,
            "10.10.0.2",
            comment_id,
            serde_json::json!({
                "nickname": "Guest", "email": "guest@example.com", "content": "Me too",
            }),
        )
        .await?;
        ensure!(reply.status == StatusCode::OK, "{}", reply.body);
        ensure!(reply.body["status"] == "pending");
        ensure!(reply.body["parent_id"].as_i64() == Some(comment_id));
        let reply_id = reply.body["id"].as_i64().context("missing reply id")?;

        // 回复同样入队通知 job，recipient 为被回复评论的作者。
        let (reply_payload,): (serde_json::Value,) = sqlx::query_as(
            "SELECT payload FROM background_jobs \
             WHERE kind = 'comment_notification' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(
            reply_payload["comment_id"].as_i64() == Some(reply_id),
            "reply job payload must reference the reply: {reply_payload}"
        );
        ensure!(
            reply_payload["recipient_email"] == "reader@example.com",
            "reply job must notify the parent comment author: {reply_payload}"
        );

        let list = app
            .get(&format!("/api/public/articles/{slug}/comments"))
            .await?;
        ensure!(list.body["items"][0]["children"].as_array().map(Vec::len) == Some(0));

        // 回复审核通过 → 出现在根的 children 中。
        approve_comment(&app, &owner_cookie, reply_id).await?;
        let list = app
            .get(&format!("/api/public/articles/{slug}/comments"))
            .await?;
        let children = list.body["items"][0]["children"]
            .as_array()
            .context("missing children")?;
        ensure!(children.len() == 1);
        ensure!(children[0]["nickname"] == "Guest");
        ensure!(children[0].get("email").is_none());

        // 回复未审核的评论 → 404（不泄露其存在与状态）。
        let pending =
            submit_comment(&app, "10.10.0.3", comment_body(&slug, "still pending")).await?;
        ensure!(pending.status == StatusCode::OK, "{}", pending.body);
        let pending_id = pending.body["id"].as_i64().context("missing pending id")?;
        let reply_to_pending = submit_reply(
            &app,
            "10.10.0.4",
            pending_id,
            serde_json::json!({
                "nickname": "Guest", "email": "guest@example.com", "content": "hello?",
            }),
        )
        .await?;
        ensure!(
            reply_to_pending.status == StatusCode::NOT_FOUND,
            "{}",
            reply_to_pending.body
        );

        // 目标不存在 / 未发布 → 404。
        let missing = submit_comment(&app, "10.10.0.5", comment_body("no-such-post", "hi")).await?;
        ensure!(missing.status == StatusCode::NOT_FOUND, "{}", missing.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn public_comment_validation_page_target_and_rate_limit() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let (_article_id, slug) = create_published_article(&app, &owner_cookie).await?;

        // 非法 website scheme → 400。
        let mut body = comment_body(&slug, "hello");
        body["website"] = serde_json::json!("javascript:alert(1)");
        let bad_website = submit_comment(&app, "10.20.0.1", body).await?;
        ensure!(bad_website.status == StatusCode::BAD_REQUEST, "{}", bad_website.body);
        ensure!(bad_website.body["error"]["code"] == "INVALID_COMMENT_URL");

        // 邮箱格式非法 → 400。
        let mut body = comment_body(&slug, "hello");
        body["email"] = serde_json::json!("not-an-email");
        let bad_email = submit_comment(&app, "10.20.0.1", body).await?;
        ensure!(bad_email.status == StatusCode::BAD_REQUEST, "{}", bad_email.body);
        ensure!(bad_email.body["error"]["code"] == "INVALID_COMMENT_EMAIL");

        // 昵称为空 → 400。
        let mut body = comment_body(&slug, "hello");
        body["nickname"] = serde_json::json!("   ");
        let bad_name = submit_comment(&app, "10.20.0.1", body).await?;
        ensure!(bad_name.status == StatusCode::BAD_REQUEST, "{}", bad_name.body);

        // page 目标：已发布页面可评论。
        let page = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "guestbook", "title": "Guestbook", "status": "published" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(page.status == StatusCode::CREATED, "{}", page.body);
        let mut body = comment_body("guestbook", "page comment");
        body["target_type"] = serde_json::json!("page");
        let page_comment = submit_comment(&app, "10.20.0.2", body).await?;
        ensure!(page_comment.status == StatusCode::OK, "{}", page_comment.body);
        let page_comment_id = page_comment.body["id"].as_i64().context("missing id")?;

        // 页面评论列表：审核前为空，审核后可拉取（与文章同语义的两级树）。
        let page_comments = app.get("/api/public/pages/guestbook/comments").await?;
        ensure!(page_comments.status == StatusCode::OK, "{}", page_comments.body);
        ensure!(page_comments.body["total"].as_i64() == Some(0));
        approve_comment(&app, &owner_cookie, page_comment_id).await?;
        let page_comments = app.get("/api/public/pages/guestbook/comments").await?;
        ensure!(page_comments.body["total"].as_i64() == Some(1));
        ensure!(page_comments.body["items"][0]["nickname"] == "Reader");

        // page 目标：草稿页面不可评论 → 404。
        let draft = app
            .admin_post(
                "/api/admin/pages",
                serde_json::json!({ "slug": "draft-page", "title": "Draft" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(draft.status == StatusCode::CREATED, "{}", draft.body);
        let mut body = comment_body("draft-page", "nope");
        body["target_type"] = serde_json::json!("page");
        let draft_target = submit_comment(&app, "10.20.0.3", body).await?;
        ensure!(draft_target.status == StatusCode::NOT_FOUND, "{}", draft_target.body);
        // 未发布页面的评论列表同样 404。
        let draft_comments = app.get("/api/public/pages/draft-page/comments").await?;
        ensure!(draft_comments.status == StatusCode::NOT_FOUND, "{}", draft_comments.body);

        // 限流：同一 fingerprint 60 秒内最多 3 条（前 3 条成功，第 4 条 429）。
        for index in 0..3 {
            let ok = submit_comment(&app, "10.20.0.9", comment_body(&slug, &format!("msg {index}")))
                .await?;
            ensure!(ok.status == StatusCode::OK, "{}: {}", index, ok.body);
        }
        let limited = submit_comment(&app, "10.20.0.9", comment_body(&slug, "one too many"))
            .await?;
        ensure!(limited.status == StatusCode::TOO_MANY_REQUESTS, "{}", limited.body);
        ensure!(limited.body["error"]["code"] == "RATE_LIMITED");
        // 写端点响应禁止缓存。
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn public_comment_policy_closed_and_auto_approve() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let (_article_id, slug) = create_published_article(&app, &owner_cookie).await?;

        // 切换评论策略的辅助：读取当前设置再整体写回。
        async fn set_policy(app: &TestApp, cookie: &str, policy: &str) -> anyhow::Result<()> {
            let current = app.admin_get("/api/admin/site-settings", cookie).await?;
            ensure!(current.status == StatusCode::OK, "{}", current.body);
            let mut payload = current.body.clone();
            payload["comment_policy"] = serde_json::json!(policy);
            let updated = app
                .admin_put("/api/admin/site-settings", payload, cookie)
                .await?;
            ensure!(updated.status == StatusCode::OK, "{}", updated.body);
            Ok(())
        }

        // closed → 403 COMMENTS_CLOSED。
        set_policy(&app, &owner_cookie, "closed").await?;
        let closed =
            submit_comment(&app, "10.30.0.1", comment_body(&slug, "anyone there?")).await?;
        ensure!(closed.status == StatusCode::FORBIDDEN, "{}", closed.body);
        ensure!(closed.body["error"]["code"] == "COMMENTS_CLOSED");

        // auto_approve → 直接 approved，无需审核即出现在公开列表。
        set_policy(&app, &owner_cookie, "auto_approve").await?;
        let auto = submit_comment(&app, "10.30.0.2", comment_body(&slug, "instant")).await?;
        ensure!(auto.status == StatusCode::OK, "{}", auto.body);
        ensure!(auto.body["status"] == "approved");
        let list = app
            .get(&format!("/api/public/articles/{slug}/comments"))
            .await?;
        ensure!(list.body["total"].as_i64() == Some(1));

        // 恢复默认策略。
        set_policy(&app, &owner_cookie, "moderated").await?;
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
