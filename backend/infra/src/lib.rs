use std::{env, str::FromStr};

use anyhow::{Context, bail};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

pub mod ai;
pub mod auth;
pub mod chunks;
pub mod comments;
pub mod content;
pub mod galleries;
pub mod jobs;
pub mod journals;
pub mod like;
pub mod links;
pub mod logged;
pub mod logs;
pub mod markdown;
pub mod media;
pub mod navigation;
pub mod pages;
pub mod password;
pub mod settings;
pub mod site_settings;
pub mod storage;
pub mod thumbnail;
pub mod upload;
pub mod where_clause;

pub use ai::{
    AnthropicProvider, DispatchingAiProvider, OpenAiCompatibleProvider, PostgresAiRequestRepository,
};
pub use auth::PostgresAuthRepository;
pub use chunks::PostgresChunkRepository;
pub use comments::PostgresCommentRepository;
pub use content::PostgresContentRepository;
pub use galleries::PostgresGalleryRepository;
pub use jobs::PostgresJobRepository;
pub use journals::PostgresJournalRepository;
pub use like::{escape_like, like_pattern};
pub use links::PostgresLinkRepository;
pub use logs::PostgresLogRepository;
pub use markdown::ComrakMarkdownRenderer;
pub use media::PostgresMediaRepository;
pub use navigation::PostgresNavigationRepository;
pub use pages::PostgresPageRepository;
pub use password::Argon2PasswordHasher;
pub use settings::PostgresSettingRepository;
pub use site_settings::PostgresSiteSettingsRepository;

pub struct PostgresConfig {
    options: PgConnectOptions,
    schema: String,
    /// 慢查询阈值（毫秒）：0 表示关闭；超过阈值的 SQL 以 WARN 级记录
    ///（sqlx 原生 log_slow_statements，走 tracing 管道自动落 server_logs）。
    slow_query_ms: u64,
}

impl PostgresConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let schema = env::var("DATABASE_SCHEMA").unwrap_or_else(|_| "public".to_owned());
        validate_schema(&schema)?;
        let slow_query_ms =
            parse_slow_query_ms(env::var("DATABASE_SLOW_QUERY_MS").ok().as_deref())?;

        let structured_names = [
            "DATABASE_HOST",
            "DATABASE_USERNAME",
            "DATABASE_PASSWORD",
            "DATABASE_NAME",
        ];
        let has_structured_config = structured_names
            .iter()
            .any(|name| env::var_os(name).is_some());

        let options = if has_structured_config {
            let host = required_env("DATABASE_HOST")?;
            let port = env::var("DATABASE_PORT")
                .unwrap_or_else(|_| "5432".to_owned())
                .parse::<u16>()
                .context("DATABASE_PORT must be a valid port number")?;
            let username = required_env("DATABASE_USERNAME")?;
            let password = required_env("DATABASE_PASSWORD")?;
            let database = required_env("DATABASE_NAME")?;

            PgConnectOptions::new()
                .host(&host)
                .port(port)
                .username(&username)
                .password(&password)
                .database(&database)
        } else {
            let url = required_env("DATABASE_URL")?;
            PgConnectOptions::from_str(&url)
                .context("DATABASE_URL is not a valid PostgreSQL URL")?
        };

        Ok(Self {
            options,
            schema,
            slow_query_ms,
        })
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// 派生使用自定义 search_path 的配置；集成测试用随机 Schema 隔离数据时，
    /// 需要把基础 Schema 留在 search_path 中，才能解析 citext 等扩展类型。
    pub fn with_search_path(&self, search_path: String) -> Self {
        Self {
            options: self.options.clone(),
            schema: search_path,
            slow_query_ms: self.slow_query_ms,
        }
    }

    fn options(&self) -> PgConnectOptions {
        let options = self
            .options
            .clone()
            .options([("search_path", self.schema.as_str())]);
        if self.slow_query_ms > 0 {
            options.log_slow_statements(
                log::LevelFilter::Warn,
                std::time::Duration::from_millis(self.slow_query_ms),
            )
        } else {
            options
        }
    }
}

pub async fn connect_postgres(config: &PostgresConfig) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect_with(config.options())
        .await
        .context("failed to connect to PostgreSQL")
}

pub async fn ensure_schema(pool: &PgPool, schema: &str) -> anyhow::Result<()> {
    let statement = format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"");
    sqlx::query(&statement)
        .execute(pool)
        .await
        .with_context(|| format!("failed to create PostgreSQL schema {schema}"))?;
    Ok(())
}

pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../../migrations")
        .run(pool)
        .await
        .context("failed to run PostgreSQL migrations")
}

fn required_env(name: &str) -> anyhow::Result<String> {
    env::var(name).with_context(|| format!("missing required environment variable {name}"))
}

/// 慢查询阈值解析：未设置默认 500ms；0 关闭；负数/非数字报错。
fn parse_slow_query_ms(value: Option<&str>) -> anyhow::Result<u64> {
    match value {
        None => Ok(500),
        Some(raw) => raw
            .parse::<u64>()
            .context("DATABASE_SLOW_QUERY_MS must be a non-negative integer (milliseconds)"),
    }
}
fn validate_schema(schema: &str) -> anyhow::Result<()> {
    let mut chars = schema.chars();
    let valid_start = chars
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    let valid_rest = chars.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if !valid_start || !valid_rest {
        bail!("DATABASE_SCHEMA must be a simple PostgreSQL identifier")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_slow_query_ms;

    #[test]
    fn slow_query_ms_defaults_and_boundaries() {
        // 未设置 → 默认 500ms。
        assert_eq!(parse_slow_query_ms(None).unwrap(), 500);
        // 0 → 关闭。
        assert_eq!(parse_slow_query_ms(Some("0")).unwrap(), 0);
        assert_eq!(parse_slow_query_ms(Some("250")).unwrap(), 250);
        // 负数与非法值报错。
        assert!(parse_slow_query_ms(Some("-1")).is_err());
        assert!(parse_slow_query_ms(Some("abc")).is_err());
        assert!(parse_slow_query_ms(Some("")).is_err());
    }
}
