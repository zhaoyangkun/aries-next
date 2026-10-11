//! 邮件发送（lettre SMTP）与投递审计 Repository 的 PostgreSQL 实现。
//! `EmailSender` 配置随调用传入，Admin 修改 SMTP 设置后无需重启即生效；
//! 错误映射严格区分永久/瞬时失败，错误信息中绝不携带 SMTP 密码或完整配置。

use std::{str::FromStr, time::Duration};

use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use aries_core::email::{
    EmailDelivery, EmailDeliveryError, EmailDeliveryListQuery, EmailDeliveryPage,
    EmailDeliveryRepository, EmailDeliveryStatus, EmailError, EmailMessage, EmailSender,
    NewEmailDelivery,
};
use aries_core::settings::EmailSettings;
use async_trait::async_trait;
use lettre::message::{Mailbox, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

/// 无状态 SMTP 发送器：每次调用从 `EmailSettings` 现读配置并新建连接池。
pub struct LettreEmailSender;

/// 校验邮件配置完整性，返回 `(smtp_host, smtp_port, from_address)`。
/// 配置缺失/未启用属终态错误（重试无意义），先于任何网络操作返回。
fn validate(settings: &EmailSettings) -> Result<(&str, u16, &str), EmailError> {
    if !settings.enabled {
        return Err(EmailError::NotConfigured);
    }
    let host = settings
        .smtp_host
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(EmailError::NotConfigured)?;
    let port = settings.smtp_port.ok_or(EmailError::NotConfigured)?;
    let from_address = settings
        .from_address
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(EmailError::NotConfigured)?;
    Ok((host, port, from_address))
}

/// 端口决定 TLS 模式：465 为隐式 TLS（Wrapper），其余端口走 STARTTLS（Required）。
/// lettre 0.11 起 `Tls` 变体携带 `TlsParameters`（证书校验域名用 SMTP Host）。
fn build_tls(host: &str, port: u16) -> Result<Tls, EmailError> {
    let parameters = TlsParameters::new(host.to_owned())
        .map_err(|error| EmailError::Transient(error.to_string()))?;
    Ok(if port == 465 {
        Tls::Wrapper(parameters)
    } else {
        Tls::Required(parameters)
    })
}

/// 按 SMTP 响应类别映射发送错误：5xx 永久失败不重试，4xx/连接/超时等交调用方重试。
fn map_send_error(error: lettre::transport::smtp::Error) -> EmailError {
    if error.is_permanent() {
        EmailError::Permanent(error.to_string())
    } else {
        // is_transient（SMTP 4xx）与连接/超时等网络错误统一按瞬时处理。
        EmailError::Transient(error.to_string())
    }
}

#[async_trait]
impl EmailSender for LettreEmailSender {
    async fn send(
        &self,
        settings: &EmailSettings,
        message: &EmailMessage,
    ) -> Result<(), EmailError> {
        let (host, port, from_address) = validate(settings)?;

        // 每次调用重建 Transport：配置可能已变更，lettre 连接池随调用生命周期释放。
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|error| EmailError::Transient(error.to_string()))?
            .tls(build_tls(host, port)?)
            .timeout(Some(Duration::from_secs(10)));
        if let Some(username) = settings
            .smtp_username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let password = settings.smtp_password.clone().unwrap_or_default();
            builder = builder.credentials(Credentials::new(username.to_owned(), password));
        }
        let transport = builder.build();

        // 地址解析失败是终态错误（重试不会使非法地址变合法）。
        let from = Address::from_str(from_address)
            .map_err(|error| EmailError::Permanent(error.to_string()))?;
        let from_mailbox = Mailbox::new(settings.from_name.clone(), from);
        let to = Address::from_str(&message.to)
            .map_err(|error| EmailError::Permanent(error.to_string()))?;

        let text_part = SinglePart::builder()
            .header(ContentType::TEXT_PLAIN)
            .body(message.text.clone());
        let builder = Message::builder()
            .from(from_mailbox)
            .to(Mailbox::new(None, to))
            .subject(message.subject.clone());
        let email = match &message.html {
            // multipart/alternative：同时携带纯文本与 HTML，由邮件客户端自行选择展示。
            Some(html) => builder.multipart(
                MultiPart::alternative().singlepart(text_part).singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(html.clone()),
                ),
            ),
            None => builder.body(message.text.clone()),
        }
        .map_err(|error| EmailError::Transient(error.to_string()))?;

        transport.send(email).await.map_err(map_send_error)?;
        Ok(())
    }
}

// ============================================================
// 投递审计 Repository
// ============================================================

#[derive(Clone)]
pub struct PostgresEmailDeliveryRepository {
    pool: PgPool,
}

impl PostgresEmailDeliveryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct DeliveryRow {
    id: i64,
    comment_id: Option<i64>,
    recipient_email: String,
    subject: String,
    template: String,
    status: String,
    attempts: i32,
    max_attempts: i32,
    last_error: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<DeliveryRow> for EmailDelivery {
    type Error = EmailDeliveryError;

    fn try_from(row: DeliveryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            comment_id: row.comment_id,
            recipient_email: row.recipient_email,
            subject: row.subject,
            template: row.template,
            status: EmailDeliveryStatus::from_str(&row.status)?,
            attempts: row.attempts,
            max_attempts: row.max_attempts,
            last_error: row.last_error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const DELIVERY_COLUMNS: &str = "id, comment_id, recipient_email, subject, template, \
    status, attempts, max_attempts, last_error, created_at, updated_at";

fn map_sqlx(error: sqlx::Error) -> EmailDeliveryError {
    tracing::error!(error = %error, "email delivery repository operation failed");
    EmailDeliveryError::StoreUnavailable
}

#[async_trait]
impl EmailDeliveryRepository for PostgresEmailDeliveryRepository {
    async fn create(
        &self,
        delivery: NewEmailDelivery,
    ) -> Result<EmailDelivery, EmailDeliveryError> {
        // status 落库默认 'pending'（列默认值），与幂等创建语义一致。
        let query = format!(
            "INSERT INTO email_deliveries \
                (comment_id, recipient_email, subject, template, max_attempts) \
             VALUES ($1, $2, $3, $4, $5) RETURNING {DELIVERY_COLUMNS}"
        );
        let row = logged_query_as::<DeliveryRow>(&query)
            .bind(delivery.comment_id)
            .bind(delivery.recipient_email)
            .bind(delivery.subject)
            .bind(delivery.template)
            .bind(delivery.max_attempts)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }

    async fn mark_sent(&self, delivery_id: i64) -> Result<(), EmailDeliveryError> {
        let result = logged_query(
            "UPDATE email_deliveries SET status = 'sent', attempts = attempts + 1, \
             last_error = NULL, updated_at = now() WHERE id = $1",
        )
        .bind(delivery_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(EmailDeliveryError::NotFound);
        }
        Ok(())
    }

    async fn mark_failed(&self, delivery_id: i64, error: &str) -> Result<(), EmailDeliveryError> {
        let result = logged_query(
            "UPDATE email_deliveries SET status = 'failed', last_error = $2, \
             updated_at = now() WHERE id = $1",
        )
        .bind(delivery_id)
        // last_error 截断到 2000 字符，避免超长堆栈撑爆行。
        .bind(error.chars().take(2000).collect::<String>())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(EmailDeliveryError::NotFound);
        }
        Ok(())
    }

    async fn record_attempt(
        &self,
        delivery_id: i64,
        error: &str,
    ) -> Result<(), EmailDeliveryError> {
        // attempts 递增后与 max_attempts 比较：超限转 Failed，否则保持 Pending 等待重试。
        let result = logged_query(
            "UPDATE email_deliveries SET attempts = attempts + 1, last_error = $2, \
             status = CASE WHEN attempts + 1 >= max_attempts THEN 'failed' ELSE 'pending' END, \
             updated_at = now() WHERE id = $1",
        )
        .bind(delivery_id)
        .bind(error.chars().take(2000).collect::<String>())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(EmailDeliveryError::NotFound);
        }
        Ok(())
    }

    async fn list(
        &self,
        query: EmailDeliveryListQuery,
    ) -> Result<EmailDeliveryPage, EmailDeliveryError> {
        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, 100);
        let offset = i64::from(page - 1) * i64::from(page_size);
        let status = query.status.map(|value| value.as_str().to_owned());

        let total = logged_query_scalar::<i64>(
            "SELECT count(*) FROM email_deliveries \
             WHERE ($1::text IS NULL OR status = $1)",
        )
        .bind(status.as_deref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let list_query = format!(
            "SELECT {DELIVERY_COLUMNS} FROM email_deliveries \
             WHERE ($1::text IS NULL OR status = $1) \
             ORDER BY created_at DESC, id DESC LIMIT $2 OFFSET $3"
        );
        let rows = logged_query_as::<DeliveryRow>(&list_query)
            .bind(status.as_deref())
            .bind(i64::from(page_size))
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let items = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(EmailDeliveryPage {
            items,
            total,
            page,
            page_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_settings() -> EmailSettings {
        EmailSettings {
            enabled: true,
            smtp_host: Some("smtp.example.com".to_owned()),
            smtp_port: Some(587),
            smtp_username: None,
            smtp_password: None,
            from_address: Some("noreply@example.com".to_owned()),
            from_name: None,
        }
    }

    #[test]
    fn validate_rejects_disabled() {
        let mut settings = sample_settings();
        settings.enabled = false;
        assert_eq!(validate(&settings), Err(EmailError::NotConfigured));
    }

    #[test]
    fn validate_rejects_blank_host_and_missing_port_and_blank_from() {
        let mut settings = sample_settings();
        settings.smtp_host = Some("   ".to_owned());
        assert_eq!(validate(&settings), Err(EmailError::NotConfigured));
        settings.smtp_host = None;
        assert_eq!(validate(&settings), Err(EmailError::NotConfigured));

        let mut settings = sample_settings();
        settings.smtp_port = None;
        assert_eq!(validate(&settings), Err(EmailError::NotConfigured));

        let mut settings = sample_settings();
        settings.from_address = Some(String::new());
        assert_eq!(validate(&settings), Err(EmailError::NotConfigured));
        settings.from_address = None;
        assert_eq!(validate(&settings), Err(EmailError::NotConfigured));
    }

    #[test]
    fn validate_accepts_complete_settings() {
        assert_eq!(
            validate(&sample_settings()),
            Ok(("smtp.example.com", 587, "noreply@example.com"))
        );
    }

    #[test]
    fn tls_mode_follows_port() {
        // 465 隐式 TLS，其余端口 STARTTLS。
        assert!(matches!(
            build_tls("smtp.example.com", 465),
            Ok(Tls::Wrapper(_))
        ));
        assert!(matches!(
            build_tls("smtp.example.com", 587),
            Ok(Tls::Required(_))
        ));
        assert!(matches!(
            build_tls("smtp.example.com", 25),
            Ok(Tls::Required(_))
        ));
    }
}
