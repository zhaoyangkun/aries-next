use std::{env, net::SocketAddr};

use anyhow::{Context, bail};
use time::Duration;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub address: SocketAddr,
    pub admin_origins: Vec<String>,
    pub bootstrap_secret: String,
    pub session_ttl: Duration,
    pub cookie_secure: bool,
    /// 媒体公开 URL 前缀，Usage 解析与静态文件路由共用，必须与存储后端的 Base URL 一致。
    pub media_public_base_url: String,
    /// 媒体存储提供者（local / s3），上传入库时写入 provider 字段。
    pub media_provider: String,
    /// 慢请求阈值（毫秒）：请求耗时达到即打 WARN 日志（带 path/status），0 关闭。
    pub slow_request_ms: u64,
    /// ERROR 尖峰阈值：最近 5 分钟 ERROR 日志数达到即打 WARN（未来可挂通知通道），0 关闭。
    pub log_error_spike_threshold: u64,
    /// 是否挂载 OpenAPI 文档（Scalar UI，spec 内嵌页面）到 /api/docs。默认关闭：
    /// 文档会暴露全部端点结构，生产环境按需显式开启。
    pub openapi_docs_enabled: bool,
}

impl ServerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let address = env::var("SERVER_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8088".to_owned())
            .parse()
            .context("invalid SERVER_ADDR")?;
        let admin_origins = parse_origins(
            &env::var("ADMIN_ORIGINS")
                .or_else(|_| env::var("ADMIN_ORIGIN"))
                .unwrap_or_else(|_| "http://127.0.0.1:5173,http://localhost:5173".to_owned()),
        )?;

        let bootstrap_secret = env::var("BOOTSTRAP_SECRET")
            .context("missing required environment variable BOOTSTRAP_SECRET")?;
        if bootstrap_secret.len() < 24 {
            bail!("BOOTSTRAP_SECRET must contain at least 24 characters");
        }

        let session_hours = env::var("SESSION_TTL_HOURS")
            .unwrap_or_else(|_| "12".to_owned())
            .parse::<i64>()
            .context("SESSION_TTL_HOURS must be an integer")?;
        if !(1..=720).contains(&session_hours) {
            bail!("SESSION_TTL_HOURS must be between 1 and 720");
        }

        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_owned());
        let cookie_secure = parse_bool(
            "SESSION_COOKIE_SECURE",
            env::var("SESSION_COOKIE_SECURE").ok().as_deref().unwrap_or(
                if app_env == "production" {
                    "true"
                } else {
                    "false"
                },
            ),
        )?;

        let media_public_base_url =
            env::var("MEDIA_PUBLIC_BASE_URL").unwrap_or_else(|_| "/api/media/files".to_owned());
        if !media_public_base_url.starts_with('/') && !media_public_base_url.starts_with("http") {
            bail!("MEDIA_PUBLIC_BASE_URL must be an absolute path or URL");
        }
        let media_provider = env::var("MEDIA_PROVIDER").unwrap_or_else(|_| "local".to_owned());
        if !matches!(media_provider.as_str(), "local" | "s3") {
            bail!("MEDIA_PROVIDER must be local or s3");
        }
        let slow_request_ms = env::var("LOG_SLOW_REQUEST_MS")
            .unwrap_or_else(|_| "1000".to_owned())
            .parse::<u64>()
            .context("LOG_SLOW_REQUEST_MS must be a non-negative integer")?;
        let log_error_spike_threshold = env::var("LOG_ERROR_SPIKE_THRESHOLD")
            .unwrap_or_else(|_| "10".to_owned())
            .parse::<u64>()
            .context("LOG_ERROR_SPIKE_THRESHOLD must be a non-negative integer")?;
        let openapi_docs_enabled = parse_bool(
            "OPENAPI_DOCS",
            env::var("OPENAPI_DOCS").ok().as_deref().unwrap_or("false"),
        )?;

        Ok(Self {
            address,
            admin_origins,
            bootstrap_secret,
            session_ttl: Duration::hours(session_hours),
            cookie_secure,
            media_public_base_url,
            media_provider,
            slow_request_ms,
            log_error_spike_threshold,
            openapi_docs_enabled,
        })
    }
}

fn validate_origin(origin: &str) -> anyhow::Result<()> {
    let uri = origin
        .parse::<axum::http::Uri>()
        .context("ADMIN_ORIGINS must contain valid origins")?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri.authority().is_none()
        || (!uri.path().is_empty() && uri.path() != "/")
        || uri.query().is_some()
    {
        bail!("ADMIN_ORIGINS entries must contain only scheme and authority");
    }
    Ok(())
}

fn parse_origins(value: &str) -> anyhow::Result<Vec<String>> {
    let mut origins = Vec::new();
    for origin in value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_origin(origin)?;
        if !origins.iter().any(|existing| existing == origin) {
            origins.push(origin.to_owned());
        }
    }
    if origins.is_empty() {
        bail!("ADMIN_ORIGINS must contain at least one origin");
    }
    Ok(origins)
}

fn parse_bool(name: &str, value: &str) -> anyhow::Result<bool> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => bail!("{name} must be true or false"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_rejects_paths_and_non_http_schemes() {
        assert!(validate_origin("http://127.0.0.1:5173").is_ok());
        assert!(validate_origin("http://127.0.0.1:5173/admin").is_err());
        assert!(validate_origin("file://local").is_err());
    }

    #[test]
    fn origins_accept_multiple_exact_values_and_remove_duplicates() {
        let origins =
            parse_origins("http://127.0.0.1:5173, http://localhost:5173,http://localhost:5173")
                .unwrap();

        assert_eq!(
            origins,
            vec![
                "http://127.0.0.1:5173".to_owned(),
                "http://localhost:5173".to_owned()
            ]
        );
        assert!(parse_origins(" , ").is_err());
    }
}
