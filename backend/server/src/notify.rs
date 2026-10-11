//! 邮件通知编排：评论通知入队与密码重置邮件的内联发送。
//! 投递审计行（email_deliveries）在入队时创建、delivery_id 随 Job payload 传递，
//! Worker 重试只重发不重建行，保证幂等；渲染好的正文随 payload 携带，
//! Worker 无需二次查库渲染。通知失败（含入队失败）只记日志，绝不影响主流程响应。

use aries_core::{
    comments::{Comment, CommentTargetType},
    email::{self, CommentNoticeKind, NewEmailDelivery},
    jobs::{JobKind, NewBackgroundJob},
    settings::{EmailSettings, SettingGroup},
};

use crate::state::AppState;

/// 从 `setting_groups` 现读邮件设置（配置变更无需重启）。
pub async fn load_email_settings(state: &AppState) -> Result<EmailSettings, String> {
    let record = state
        .settings
        .get_group(SettingGroup::Email)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_value(record.payload)
        .map_err(|error| format!("invalid email settings payload: {error}"))
}

/// 评论通知入队：每个收件人一行投递审计 + 一个 CommentNotification Job。
/// `recipients` 由调用方按业务规则解析（根评论→管理员列表，回复→被回复者）；
/// 自动去重、剔除空值与评论作者本人（不给自己发通知）。
pub async fn enqueue_comment_notifications(
    state: &AppState,
    comment: &Comment,
    kind: CommentNoticeKind,
    recipients: &[String],
) {
    let author_email = comment.author_email.trim().to_lowercase();
    let mut seen = std::collections::HashSet::new();
    let recipients: Vec<String> = recipients
        .iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && value.to_lowercase() != author_email)
        .filter(|value| seen.insert(value.to_lowercase()))
        .collect();
    if recipients.is_empty() {
        return;
    }

    // 解析目标标题与绝对 URL；site_url 未配置时邮件链接无从拼起，整体跳过。
    let site = match state.site_settings.get().await {
        Ok(site) => site,
        Err(error) => {
            tracing::error!(error = %error, comment_id = comment.id, "comment notification skipped: site settings unavailable");
            return;
        }
    };
    let site_url = site.site_url.trim().trim_end_matches('/').to_owned();
    if site_url.is_empty() {
        tracing::warn!(
            comment_id = comment.id,
            "comment notification skipped: site_url not configured"
        );
        return;
    }
    let target = resolve_target(state, comment, &site_url).await;
    let Some((target_title, target_url)) = target else {
        tracing::warn!(
            comment_id = comment.id,
            "comment notification skipped: comment target not found"
        );
        return;
    };

    let rendered = email::render_comment_notification(
        kind,
        &site.site_name,
        &target_title,
        &target_url,
        &comment.author_name,
        &comment.content_markdown,
    );

    let template = match kind {
        CommentNoticeKind::New => email::TEMPLATE_COMMENT_NEW,
        CommentNoticeKind::Reply => email::TEMPLATE_COMMENT_REPLY,
    };

    for recipient in recipients {
        let delivery = state
            .email_deliveries
            .create(NewEmailDelivery::new(
                Some(comment.id),
                recipient.clone(),
                rendered.subject.clone(),
                template.to_owned(),
            ))
            .await;
        let delivery_id = match delivery {
            Ok(delivery) => delivery.id,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    comment_id = comment.id,
                    "failed to create email delivery row; notification skipped"
                );
                continue;
            }
        };
        let payload = serde_json::json!({
            "delivery_id": delivery_id,
            "comment_id": comment.id,
            "recipient_email": recipient,
            "subject": rendered.subject,
            "text": rendered.text,
            "html": rendered.html,
        });
        if let Err(error) = state
            .jobs
            .enqueue(NewBackgroundJob::new(JobKind::CommentNotification, payload))
            .await
        {
            tracing::error!(
                error = %error,
                comment_id = comment.id,
                "failed to enqueue comment notification job"
            );
        }
    }
}

/// 解析评论目标的展示标题与公开绝对 URL（文章 `/articles/{slug}`、页面 `/custom/{slug}`）。
async fn resolve_target(
    state: &AppState,
    comment: &Comment,
    site_url: &str,
) -> Option<(String, String)> {
    match comment.target_type {
        CommentTargetType::Article => state
            .content
            .find_article(comment.target_id)
            .await
            .ok()?
            .map(|article| {
                (
                    article.title,
                    format!("{site_url}/articles/{}", article.slug),
                )
            }),
        CommentTargetType::Page => state
            .pages
            .find(comment.target_id)
            .await
            .ok()?
            .map(|page| (page.title, format!("{site_url}/custom/{}", page.slug))),
        CommentTargetType::Link => None,
    }
}

/// 密码重置邮件发送结果，写入审计 metadata 的 `delivery` 字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetDelivery {
    Sent,
    Failed,
    NotConfigured,
}

impl ResetDelivery {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::NotConfigured => "not_configured",
        }
    }
}

/// 忘记密码：内联发送重置邮件（端点已有 3 次/15 分钟限流，SMTP 10s 超时可接受）。
/// 不走 Background Job：raw Reset Token 不落库（background_jobs.payload 是 DB jsonb）。
/// 任何失败都只记日志并体现在返回值里，调用方仍返回 202（防账号枚举）。
pub async fn send_password_reset_email(
    state: &AppState,
    recipient: &str,
    token: &str,
) -> ResetDelivery {
    let settings = match load_email_settings(state).await {
        Ok(settings) => settings,
        Err(error) => {
            tracing::error!(error, "password reset email skipped: settings unavailable");
            return ResetDelivery::NotConfigured;
        }
    };
    if !settings.enabled {
        return ResetDelivery::NotConfigured;
    }
    let site = match state.site_settings.get().await {
        Ok(site) => site,
        Err(error) => {
            tracing::error!(error = %error, "password reset email skipped: site settings unavailable");
            return ResetDelivery::Failed;
        }
    };
    let site_url = site.site_url.trim().trim_end_matches('/');
    if site_url.is_empty() {
        tracing::warn!("password reset email skipped: site_url not configured");
        return ResetDelivery::NotConfigured;
    }
    let reset_url = format!("{site_url}/admin/auth/reset-password?token={token}");
    let rendered = email::render_password_reset(&site.site_name, &reset_url);

    // 投递审计：comment_id 为空（系统邮件），模板 password_reset。
    let delivery_id = match state
        .email_deliveries
        .create(NewEmailDelivery::new(
            None,
            recipient.to_owned(),
            rendered.subject.clone(),
            email::TEMPLATE_PASSWORD_RESET.to_owned(),
        ))
        .await
    {
        Ok(delivery) => Some(delivery.id),
        Err(error) => {
            tracing::error!(error = %error, "failed to create password reset delivery row");
            None
        }
    };

    let message = email::EmailMessage {
        to: recipient.to_owned(),
        subject: rendered.subject,
        text: rendered.text,
        html: Some(rendered.html),
    };
    match state.email.send(&settings, &message).await {
        Ok(()) => {
            if let Some(id) = delivery_id {
                let _ = state.email_deliveries.mark_sent(id).await;
            }
            ResetDelivery::Sent
        }
        Err(error) => {
            tracing::error!(error = %error, "failed to send password reset email");
            if let Some(id) = delivery_id {
                let _ = state
                    .email_deliveries
                    .mark_failed(id, &error.to_string())
                    .await;
            }
            ResetDelivery::Failed
        }
    }
}
