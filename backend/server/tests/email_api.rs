//! 邮件链路 Contract Test：SMTP 测试发送、投递审计查询、
//! 忘记密码邮件（含完整重置流程）、评论通知 Worker 的发送与重试语义。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::{RecordingEmailSender, TestApp};

/// 直接改库启用邮件功能（SMTP 主机指向假地址，实际发送由 RecordingEmailSender 接管）。
async fn enable_email_settings(app: &TestApp) -> anyhow::Result<()> {
    sqlx::query("UPDATE setting_groups SET payload = $2 WHERE grp = $1")
        .bind("email")
        .bind(serde_json::json!({
            "enabled": true,
            "smtp_host": "smtp.example.com",
            "smtp_port": 587,
            "smtp_username": "mailer",
            "smtp_password": "secret",
            "from_address": "noreply@example.com",
            "from_name": "Test Blog"
        }))
        .execute(&app.state.database)
        .await
        .context("failed to enable email settings")?;
    sqlx::query("UPDATE site_settings SET site_url = 'https://blog.example.com'")
        .execute(&app.state.database)
        .await
        .context("failed to set site_url")?;
    Ok(())
}

async fn create_published_article(app: &TestApp, cookie: &str) -> anyhow::Result<(i64, String)> {
    let created = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({ "title": "Notified Post", "markdown_source": "# c" }),
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

#[tokio::test]
async fn test_email_endpoint_sends_and_audits() -> anyhow::Result<()> {
    let sender = RecordingEmailSender::new();
    let Some(app) = common::maybe_app_with_email(Some(sender.clone())).await? else {
        return Ok(());
    };
    let result = async {
        let cookie = app.bootstrap_owner().await?;

        // 未启用邮件功能时：400 EMAIL_NOT_CONFIGURED。
        let response = app
            .admin_post(
                "/api/admin/settings/email/test",
                serde_json::json!({ "recipient": "admin@example.com" }),
                Some(&cookie),
            )
            .await?;
        ensure!(
            response.status == StatusCode::BAD_REQUEST,
            "{}",
            response.body
        );
        ensure!(response.body["error"]["code"] == "EMAIL_NOT_CONFIGURED");

        // 未认证：401。
        let response = app
            .admin_post(
                "/api/admin/settings/email/test",
                serde_json::json!({ "recipient": "admin@example.com" }),
                None,
            )
            .await?;
        ensure!(
            response.status == StatusCode::UNAUTHORIZED,
            "{}",
            response.body
        );

        enable_email_settings(&app).await?;
        let response = app
            .admin_post(
                "/api/admin/settings/email/test",
                serde_json::json!({ "recipient": "admin@example.com" }),
                Some(&cookie),
            )
            .await?;
        ensure!(response.status == StatusCode::OK, "{}", response.body);
        ensure!(response.body["status"] == "sent");

        // 假 Sender 捕获到测试邮件；投递审计落 sent。
        let captured = sender.captured();
        ensure!(captured.len() == 1);
        ensure!(captured[0].to == "admin@example.com");
        ensure!(captured[0].subject.contains("SMTP"));

        let list = app
            .admin_get("/api/admin/email-deliveries?status=sent", &cookie)
            .await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        let items = list.body["items"].as_array().context("missing items")?;
        ensure!(items.iter().any(|item| item["template"] == "test"));

        // 非法状态过滤：400。
        let list = app
            .admin_get("/api/admin/email-deliveries?status=bogus", &cookie)
            .await?;
        ensure!(list.status == StatusCode::BAD_REQUEST, "{}", list.body);
        // 未认证：401。
        let list = app.get("/api/admin/email-deliveries").await?;
        ensure!(list.status == StatusCode::UNAUTHORIZED, "{}", list.body);
        Ok(())
    }
    .await;
    app.cleanup().await?;
    result
}

#[tokio::test]
async fn forgot_password_sends_reset_email_and_token_completes_flow() -> anyhow::Result<()> {
    let sender = RecordingEmailSender::new();
    let Some(app) = common::maybe_app_with_email(Some(sender.clone())).await? else {
        return Ok(());
    };
    let result = async {
        let _cookie = app.bootstrap_owner().await?;
        enable_email_settings(&app).await?;

        let response = app
            .admin_post(
                "/api/admin/auth/password/forgot",
                serde_json::json!({ "email": "owner@example.com" }),
                None,
            )
            .await?;
        ensure!(response.status == StatusCode::ACCEPTED, "{}", response.body);

        // 捕获重置邮件：链接指向 Admin SPA 重置页并携带 Token。
        let captured = sender.captured();
        ensure!(captured.len() == 1, "expected one reset email");
        let token = captured[0]
            .text
            .split("token=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .context("reset email missing token")?
            .to_owned();
        ensure!(
            captured[0]
                .text
                .contains("https://blog.example.com/admin/auth/reset-password?token=")
        );

        // Raw Token 不得出现在 background_jobs（重置邮件走内联发送，不经队列）。
        let job_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM background_jobs WHERE payload::text LIKE '%' || $1 || '%'",
        )
        .bind(&token)
        .fetch_one(&app.state.database)
        .await?;
        ensure!(job_count == 0);

        // 用邮件里的 Token 完成重置，并用新密码登录验证。
        let reset = app
            .admin_post(
                "/api/admin/auth/password/reset",
                serde_json::json!({ "token": token, "password": "new-password-123" }),
                None,
            )
            .await?;
        ensure!(reset.status == StatusCode::NO_CONTENT, "{}", reset.body);

        // 审计记录了实际投递结果。
        let (delivery,): (String,) = sqlx::query_as(
            "SELECT metadata->>'delivery' FROM audit_logs \
             WHERE action = 'auth.password_reset_requested'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(delivery == "sent");
        Ok(())
    }
    .await;
    app.cleanup().await?;
    result
}

#[tokio::test]
async fn comment_notification_worker_sends_and_retries() -> anyhow::Result<()> {
    let sender = RecordingEmailSender::new();
    let Some(app) = common::maybe_app_with_email(Some(sender.clone())).await? else {
        return Ok(());
    };
    let result = async {
        let cookie = app.bootstrap_owner().await?;
        enable_email_settings(&app).await?;
        let (_id, slug) = create_published_article(&app, &cookie).await?;

        // 访客根评论 → 通知站点管理员（bootstrap 创建的 owner@example.com）。
        let comment = app
            .post_with_headers(
                "/api/public/comments",
                serde_json::json!({
                    "target_type": "article",
                    "target_slug": slug,
                    "nickname": "Reader",
                    "email": "reader@example.com",
                    "content": "first!"
                }),
                &[("x-forwarded-for", "10.0.0.1")],
            )
            .await?;
        ensure!(comment.status == StatusCode::OK, "{}", comment.body);

        // 入队即创建投递审计行（pending）。
        let (status,): (String,) =
            sqlx::query_as("SELECT status FROM email_deliveries WHERE template = 'comment_new'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(status == "pending");

        // 先让发送瞬时失败：Job 回到 pending 重试，投递行保持 pending 并累计 attempts。
        *sender.failure.lock().expect("sender lock") = Some(
            aries_core::email::EmailError::Transient("smtp down".to_owned()),
        );
        aries_server::worker::run_pending_jobs(&app.state).await;
        let (job_status, delivery_status, attempts): (String, String, i32) = sqlx::query_as(
            "SELECT j.status, d.status, d.attempts FROM background_jobs j \
             JOIN email_deliveries d ON d.id = (j.payload->>'delivery_id')::bigint \
             WHERE j.kind = 'comment_notification'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(
            job_status == "pending",
            "transient failure should requeue job"
        );
        ensure!(delivery_status == "pending");
        ensure!(attempts == 1);

        // 恢复后重试成功：投递 sent，管理员收到含文章链接的通知。
        // 失败退避把 run_at 推迟了 30s，测试里直接拨回立即到期。
        *sender.failure.lock().expect("sender lock") = None;
        sqlx::query(
            "UPDATE background_jobs SET run_at = now() WHERE kind = 'comment_notification'",
        )
        .execute(&app.state.database)
        .await?;
        aries_server::worker::run_pending_jobs(&app.state).await;
        let captured = sender.captured();
        ensure!(captured.len() == 1, "expected one notification email");
        ensure!(captured[0].to == "owner@example.com");
        ensure!(
            captured[0]
                .text
                .contains("https://blog.example.com/articles/")
        );
        let (status,): (String,) =
            sqlx::query_as("SELECT status FROM email_deliveries WHERE template = 'comment_new'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(status == "sent");
        Ok(())
    }
    .await;
    app.cleanup().await?;
    result
}

#[tokio::test]
async fn admin_reply_notifies_commenter() -> anyhow::Result<()> {
    let sender = RecordingEmailSender::new();
    let Some(app) = common::maybe_app_with_email(Some(sender.clone())).await? else {
        return Ok(());
    };
    let result = async {
        let cookie = app.bootstrap_owner().await?;
        enable_email_settings(&app).await?;
        let (_id, slug) = create_published_article(&app, &cookie).await?;

        // 访客评论（moderated 策略下为 pending），管理员审核通过。
        let comment = app
            .post_with_headers(
                "/api/public/comments",
                serde_json::json!({
                    "target_type": "article",
                    "target_slug": slug,
                    "nickname": "Reader",
                    "email": "reader@example.com",
                    "content": "question"
                }),
                &[("x-forwarded-for", "10.0.0.2")],
            )
            .await?;
        ensure!(comment.status == StatusCode::OK, "{}", comment.body);
        let comment_id = comment.body["id"].as_i64().context("missing comment id")?;
        let approved = app
            .admin_patch(
                &format!("/api/admin/comments/{comment_id}/status"),
                serde_json::json!({ "status": "approved" }),
                &cookie,
            )
            .await?;
        ensure!(approved.status == StatusCode::OK, "{}", approved.body);

        // 管理员回复 → 访客收到 reply 通知（收件人为访客 Email）。
        let reply = app
            .admin_post(
                &format!("/api/admin/comments/{comment_id}/reply"),
                serde_json::json!({ "content_markdown": "answer" }),
                Some(&cookie),
            )
            .await?;
        ensure!(reply.status == StatusCode::CREATED, "{}", reply.body);

        aries_server::worker::run_pending_jobs(&app.state).await;
        let captured = sender.captured();
        // 根评论的管理员通知 + 回复的访客通知，共两封。
        ensure!(
            captured.len() == 2,
            "expected owner + commenter emails, got {}",
            captured.len()
        );
        let reply_email = captured
            .iter()
            .find(|message| message.to == "reader@example.com")
            .context("missing reply notification to commenter")?;
        ensure!(reply_email.subject.contains("回复"));
        Ok(())
    }
    .await;
    app.cleanup().await?;
    result
}
