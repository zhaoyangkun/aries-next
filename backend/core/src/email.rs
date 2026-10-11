//! 邮件域：发送契约、投递审计模型与纯模板渲染。
//! `EmailSender` 为无状态适配器（配置随调用传入，与 `AiProvider` 模式一致），
//! Admin 修改 SMTP 设置后无需重启即生效；模板函数不做任何 IO，可纯单测。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

use crate::settings::EmailSettings;

/// 一封待发送邮件：`text` 为必备纯文本正文，`html` 可选。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EmailError {
    /// 邮件功能未启用或配置不完整：配置变更前重试无意义，调用方按终态处理。
    #[error("email disabled or not configured")]
    NotConfigured,
    /// 永久失败（收件人地址非法、SMTP 5xx 等）：重试不会成功。
    #[error("permanent delivery failure: {0}")]
    Permanent(String),
    /// 瞬时失败（连接/超时/SMTP 4xx 等）：交给调用方按既有重试策略处理。
    #[error("transient delivery failure: {0}")]
    Transient(String),
}

/// 邮件发送契约，由 infra 层实现（lettre SMTP）。
#[async_trait]
pub trait EmailSender: Send + Sync {
    /// 配置随每次调用传入（从 `setting_groups` 现读），设置变更无需重启。
    async fn send(
        &self,
        settings: &EmailSettings,
        message: &EmailMessage,
    ) -> Result<(), EmailError>;
}

// ============================================================
// 模板
// ============================================================

/// 投递模板标识，对应 `email_deliveries.template` 的取值（该列为无约束 text，集中在此定义）。
pub const TEMPLATE_PASSWORD_RESET: &str = "password_reset";
pub const TEMPLATE_COMMENT_NEW: &str = "comment_new";
pub const TEMPLATE_COMMENT_REPLY: &str = "comment_reply";
pub const TEMPLATE_TEST: &str = "test";

/// 评论通知类型：新根评论（通知站点管理员）/ 回复（通知被回复者）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentNoticeKind {
    New,
    Reply,
}

/// 渲染完成的邮件内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedEmail {
    pub subject: String,
    pub text: String,
    pub html: String,
}

/// Header 注入防护：进入邮件头的值（Subject、显示名）先剥控制字符；
/// lettre 构建 Message 时会再次校验，此处为双保险。
fn sanitize_header_value(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '\r' | '\n'))
        .collect()
}

/// 模板插值的 HTML 转义：所有用户可控值进入 HTML 正文前必须过此函数。
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 评论摘录：取 Markdown 源文本前 200 字符，避免通知邮件携带超长正文。
fn excerpt(markdown: &str) -> String {
    let truncated: String = markdown.chars().take(200).collect();
    if markdown.chars().count() > 200 {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// 密码重置邮件：`reset_url` 由服务端用 `site_url` 拼出，含一次性 Token。
pub fn render_password_reset(site_name: &str, reset_url: &str) -> RenderedEmail {
    let site_name = sanitize_header_value(site_name);
    let subject = format!("[{site_name}] 密码重置");
    let text = format!(
        "您好：\n\n我们收到了您在 {site_name} 的密码重置请求。\
         请在 30 分钟内打开以下链接设置新密码：\n\n{reset_url}\n\n\
         如果这不是您的操作，请忽略本邮件，您的密码不会改变。"
    );
    let html = format!(
        "<p>您好：</p><p>我们收到了您在 <strong>{}</strong> 的密码重置请求。\
         请在 30 分钟内点击以下按钮设置新密码：</p>\
         <p><a href=\"{}\" style=\"display:inline-block;padding:10px 20px;\
         background:#0f172a;color:#ffffff;text-decoration:none;border-radius:6px;\">重置密码</a></p>\
         <p style=\"color:#64748b;font-size:13px;\">或复制链接到浏览器打开：<br>{}</p>\
         <p style=\"color:#64748b;font-size:13px;\">如果这不是您的操作，请忽略本邮件，您的密码不会改变。</p>",
        escape_html(&site_name),
        escape_html(reset_url),
        escape_html(reset_url),
    );
    RenderedEmail {
        subject,
        text,
        html,
    }
}

/// 评论通知邮件：`target_url` 为新评论所在文章/页面的绝对 URL。
pub fn render_comment_notification(
    kind: CommentNoticeKind,
    site_name: &str,
    target_title: &str,
    target_url: &str,
    commenter_name: &str,
    comment_markdown: &str,
) -> RenderedEmail {
    let site_name = sanitize_header_value(site_name);
    let commenter_name = sanitize_header_value(commenter_name);
    let excerpt = excerpt(comment_markdown);
    let (action, subject) = match kind {
        CommentNoticeKind::New => (
            "发表了新评论",
            format!("[{site_name}] 新评论：{target_title}"),
        ),
        CommentNoticeKind::Reply => (
            "回复了您的评论",
            format!("[{site_name}] 评论回复：{target_title}"),
        ),
    };
    let subject = sanitize_header_value(&subject);
    let text = format!(
        "{commenter_name} 在《{target_title}》{action}：\n\n{excerpt}\n\n查看完整评论：{target_url}"
    );
    let html = format!(
        "<p><strong>{}</strong> 在《<a href=\"{}\">{}</a>》{}：</p>\
         <blockquote style=\"margin:12px 0;padding:8px 12px;border-left:3px solid #cbd5e1;\
         color:#334155;white-space:pre-wrap;\">{}</blockquote>\
         <p><a href=\"{}\">查看完整评论</a></p>",
        escape_html(&commenter_name),
        escape_html(target_url),
        escape_html(target_title),
        action,
        escape_html(&excerpt),
        escape_html(target_url),
    );
    RenderedEmail {
        subject,
        text,
        html,
    }
}

/// SMTP 测试邮件（Admin「发送测试邮件」）。
pub fn render_test_email(site_name: &str) -> RenderedEmail {
    let site_name = sanitize_header_value(site_name);
    RenderedEmail {
        subject: format!("[{site_name}] SMTP 测试邮件"),
        text: format!("这是一封来自 {site_name} 的测试邮件。收到它说明 SMTP 配置已生效。"),
        html: format!(
            "<p>这是一封来自 <strong>{}</strong> 的测试邮件。</p>\
             <p>收到它说明 SMTP 配置已生效。</p>",
            escape_html(&site_name),
        ),
    }
}

// ============================================================
// 投递审计
// ============================================================

/// 投递状态，与 `email_deliveries.status` 的 CHECK 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmailDeliveryStatus {
    Pending,
    Sent,
    Failed,
    Bounced,
}

impl EmailDeliveryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Bounced => "bounced",
        }
    }
}

impl fmt::Display for EmailDeliveryStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for EmailDeliveryStatus {
    type Err = EmailDeliveryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "sent" => Ok(Self::Sent),
            "failed" => Ok(Self::Failed),
            "bounced" => Ok(Self::Bounced),
            _ => Err(EmailDeliveryError::InvalidStatus),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmailDelivery {
    pub id: i64,
    /// 关联评论；密码重置、测试邮件等系统邮件为 None。
    pub comment_id: Option<i64>,
    pub recipient_email: String,
    pub subject: String,
    pub template: String,
    pub status: EmailDeliveryStatus,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct NewEmailDelivery {
    pub comment_id: Option<i64>,
    pub recipient_email: String,
    pub subject: String,
    pub template: String,
    pub max_attempts: i32,
}

impl NewEmailDelivery {
    /// 默认最多尝试 3 次，与 Background Job 的重试上限对齐。
    pub fn new(
        comment_id: Option<i64>,
        recipient_email: String,
        subject: String,
        template: String,
    ) -> Self {
        Self {
            comment_id,
            recipient_email,
            subject,
            template,
            max_attempts: 3,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EmailDeliveryListQuery {
    pub page: u32,
    pub page_size: u32,
    pub status: Option<EmailDeliveryStatus>,
}

#[derive(Debug, Clone)]
pub struct EmailDeliveryPage {
    pub items: Vec<EmailDelivery>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EmailDeliveryError {
    #[error("invalid email delivery status")]
    InvalidStatus,
    #[error("email delivery not found")]
    NotFound,
    #[error("email delivery store unavailable")]
    StoreUnavailable,
}

/// 投递审计 Repository，由 infra 层实现。
#[async_trait]
pub trait EmailDeliveryRepository: Send + Sync {
    /// 创建 pending 投递记录：在通知入队（或内联发送）时调用，
    /// 保证 Worker 重试时不会重复建行（幂等）。
    async fn create(&self, delivery: NewEmailDelivery)
    -> Result<EmailDelivery, EmailDeliveryError>;

    /// 标记发送成功。
    async fn mark_sent(&self, delivery_id: i64) -> Result<(), EmailDeliveryError>;

    /// 终态失败（永久错误或配置缺失）：记录 last_error 并置 failed，不再重试。
    async fn mark_failed(&self, delivery_id: i64, error: &str) -> Result<(), EmailDeliveryError>;

    /// 记录一次瞬时失败：attempts 递增并更新 last_error；
    /// 达到 max_attempts 时转为 failed，否则保持 pending 等待 Job 重试驱动。
    async fn record_attempt(&self, delivery_id: i64, error: &str)
    -> Result<(), EmailDeliveryError>;

    /// Admin 端分页查询，按 `created_at` 降序。
    async fn list(
        &self,
        query: EmailDeliveryListQuery,
    ) -> Result<EmailDeliveryPage, EmailDeliveryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_reset_email_contains_url_and_escapes_site_name() {
        let rendered = render_password_reset(
            "My<Blog\nInjected",
            "https://example.com/admin/auth/reset-password?token=abc",
        );
        // Subject 不得含控制字符（Header 注入防护）。
        assert!(!rendered.subject.contains(['\r', '\n']));
        assert!(
            rendered
                .text
                .contains("https://example.com/admin/auth/reset-password?token=abc")
        );
        // HTML 正文中的站点名必须转义。
        assert!(rendered.html.contains("My&lt;Blog"));
        assert!(!rendered.html.contains("My<Blog"));
    }

    #[test]
    fn comment_notification_distinguishes_new_and_reply() {
        let new = render_comment_notification(
            CommentNoticeKind::New,
            "Site",
            "标题",
            "https://example.com/articles/hello",
            "访客",
            "写得不错",
        );
        assert!(new.subject.contains("新评论"));
        assert!(new.text.contains("发表了新评论"));

        let reply = render_comment_notification(
            CommentNoticeKind::Reply,
            "Site",
            "标题",
            "https://example.com/articles/hello",
            "访客",
            "写得不错",
        );
        assert!(reply.subject.contains("评论回复"));
        assert!(reply.text.contains("回复了您的评论"));
        assert!(reply.html.contains("https://example.com/articles/hello"));
    }

    #[test]
    fn comment_excerpt_truncates_long_content() {
        let long = "a".repeat(500);
        let rendered = render_comment_notification(
            CommentNoticeKind::New,
            "Site",
            "t",
            "https://example.com",
            "n",
            &long,
        );
        // 摘录 200 字符 + 省略号。
        assert!(rendered.text.contains(&"a".repeat(200)));
        assert!(!rendered.text.contains(&"a".repeat(201)));
        assert!(rendered.text.contains('…'));
    }

    #[test]
    fn delivery_status_roundtrips() {
        assert_eq!("sent".parse(), Ok(EmailDeliveryStatus::Sent));
        assert_eq!(
            "bogus".parse::<EmailDeliveryStatus>(),
            Err(EmailDeliveryError::InvalidStatus)
        );
    }
}
