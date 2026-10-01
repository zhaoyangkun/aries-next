//! 日志初始化：控制台 + 文件 + PostgreSQL 三输出。
//! EnvFilter 可运行时重载（SQL 日志开关无需重启）；
//! DB Layer 从启动就在位，早期日志暂存 channel，等 Migration 完成后由 writer 任务消费。

use std::env;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Context, bail};
use tokio::sync::mpsc;
use tracing_appender::{non_blocking::WorkerGuard, rolling::Rotation};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, filter::Directive, fmt, layer::SubscriberExt, reload,
    util::SubscriberInitExt,
};

use crate::log_store::{
    DbLogLayer, LogRecord, QuietInternalSpanLayer, not_in_quiet_internal_scope,
};

/// 日志落库 channel 容量：突发日志高峰时的缓冲；满则丢弃（可丢弃数据）。
const LOG_CHANNEL_CAPACITY: usize = 4096;

/// 日志配置：控制台 + 文件双输出，文件按天切分并按保留天数自动清理。
#[derive(Debug, Clone)]
pub struct LogConfig {
    pub dir: String,
    pub retention_days: usize,
    pub sql_enabled: bool,
    pub console_json: bool,
}

impl LogConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let dir = env::var("LOG_DIR").unwrap_or_else(|_| "./logs".to_owned());
        let retention_days =
            parse_retention(&env::var("LOG_RETENTION_DAYS").unwrap_or_else(|_| "14".to_owned()))?;
        let console_json = env::var("APP_ENV").unwrap_or_default() == "production";
        // LOG_SQL 未设置时的默认值：生产关闭、开发开启（量大可在运行日志页运行时关闭）。
        let sql_enabled = match env::var("LOG_SQL") {
            Ok(value) => parse_flag(&value)?,
            Err(_) => default_sql_enabled(console_json),
        };

        Ok(Self {
            dir,
            retention_days,
            sql_enabled,
            console_json,
        })
    }
}

/// 组装 EnvFilter：LOG_SQL 环境变量只作初始值，运行期由 Admin API 切换；
/// custom 为运行时级别覆盖（如 `aries_server=debug`），优先级高于基础指令。
fn build_filter(sql_enabled: bool, custom: &[Directive]) -> anyhow::Result<EnvFilter> {
    let mut filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx::query=off"));
    if sql_enabled {
        filter = filter.add_directive(
            "sqlx::query=debug"
                .parse()
                .context("invalid sqlx log directive")?,
        );
    }
    for directive in custom {
        filter = filter.add_directive(directive.clone());
    }
    Ok(filter)
}

/// 校验并规范化自定义 directives 字符串（逗号分隔，如 `aries_server=debug,tower_http=debug`）。
/// 返回按 `, ` 连接的规范化串；空串表示清除覆盖。非法 directive 返回 Err（Admin API 转 400）。
pub fn validate_directives(directives: &str) -> anyhow::Result<String> {
    let mut parsed = Vec::new();
    for part in directives
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let directive =
            Directive::from_str(part).with_context(|| format!("invalid log directive: {part}"))?;
        parsed.push(directive);
    }
    Ok(parsed
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", "))
}

/// 运行时日志句柄：切换 SQL 日志开关与自定义级别覆盖（reload EnvFilter，无需重启）；
/// 覆盖可带自动复位（到点未再变更则恢复默认过滤器）。
/// `noop()` 供测试 Harness 使用：无 reload handle，仅维护状态位。
#[derive(Clone)]
pub struct LogHandle {
    reload_handle: Option<Arc<reload::Handle<EnvFilter, Registry>>>,
    sql_enabled: Arc<AtomicBool>,
    /// 与 `DbLogLayer` 共享的丢弃计数：channel 满被丢弃的运行日志条数。
    dropped: Arc<AtomicU64>,
    /// 运行时级别覆盖（EnvFilter directives），空串为无覆盖。
    custom_directives: Arc<std::sync::Mutex<String>>,
    /// 覆盖代际：每次设置递增；到期的自动复位任务仅当代际未变时才执行。
    generation: Arc<AtomicU64>,
    /// 自动复位时点；None 表示持续到被显式清除。
    restore_at: Arc<std::sync::Mutex<Option<std::time::Instant>>>,
}

impl LogHandle {
    /// 测试用空句柄：只记录开关状态，不重载全局过滤器。
    pub fn noop() -> Self {
        Self {
            reload_handle: None,
            sql_enabled: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicU64::new(0)),
            custom_directives: Arc::new(std::sync::Mutex::new(String::new())),
            generation: Arc::new(AtomicU64::new(0)),
            restore_at: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn sql_enabled(&self) -> bool {
        self.sql_enabled.load(Ordering::Relaxed)
    }

    /// 运行日志 channel 累计丢弃条数（可丢弃数据，仅作观测）。
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// 当前生效的自定义级别覆盖（空串为无）。
    pub fn custom_directives(&self) -> String {
        self.custom_directives.lock().unwrap().clone()
    }

    /// 自动复位剩余时间；None 表示无自动复位（或已到期未清理）。
    pub fn restore_in(&self) -> Option<std::time::Duration> {
        self.restore_at
            .lock()
            .unwrap()
            .map(|instant| instant.saturating_duration_since(std::time::Instant::now()))
    }

    fn parsed_custom(&self) -> anyhow::Result<Vec<Directive>> {
        let custom = self.custom_directives.lock().unwrap();
        Ok(custom
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(Directive::from_str)
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// 用当前状态（SQL 开关 + 自定义覆盖）重载全局过滤器。
    fn reload(&self) -> anyhow::Result<()> {
        if let Some(handle) = &self.reload_handle {
            handle
                .reload(build_filter(self.sql_enabled(), &self.parsed_custom()?)?)
                .context("failed to reload log filter")?;
        }
        Ok(())
    }

    pub fn set_sql_logging(&self, enabled: bool) -> anyhow::Result<()> {
        self.sql_enabled.store(enabled, Ordering::Relaxed);
        self.reload()
    }

    /// 设置运行时级别覆盖；`restore_after` 到期后若期间未被再次设置，自动清空恢复默认。
    /// 空 directives 等价于清除覆盖（代际仍会递增，使未到期的前一个自动复位失效）。
    pub fn set_custom_directives(
        &self,
        directives: &str,
        restore_after: Option<std::time::Duration>,
    ) -> anyhow::Result<()> {
        let normalized = validate_directives(directives)?;
        *self.custom_directives.lock().unwrap() = normalized;
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        *self.restore_at.lock().unwrap() = restore_after.map(|d| std::time::Instant::now() + d);
        self.reload()?;

        if let Some(after) = restore_after {
            let handle = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                // 期间被再次设置（代际变化）则不复位。
                if handle.generation.load(Ordering::Relaxed) != generation {
                    return;
                }
                *handle.custom_directives.lock().unwrap() = String::new();
                *handle.restore_at.lock().unwrap() = None;
                if let Err(error) = handle.reload() {
                    tracing::error!(error = %error, "failed to restore log filter after override");
                }
                tracing::info!("log filter override auto-restored to defaults");
            });
        }
        Ok(())
    }
}

/// 日志初始化产物：WorkerGuard 必须存活到进程退出（否则缓冲区日志可能丢失）；
/// receiver 交给 `log_store::spawn_writer`（Migration 完成后）。
pub struct LogGuard {
    pub worker: WorkerGuard,
    pub handle: LogHandle,
    pub receiver: mpsc::Receiver<LogRecord>,
    pub retention_days: usize,
}

/// 初始化全局日志：控制台按环境选择 pretty/JSON，文件始终 JSON 便于采集，
/// DB Layer 通过 channel 异步落库（见 `log_store`）。
/// 只能调用一次（全局 Subscriber 唯一），测试用 `LogHandle::noop()` 代替。
pub fn init_tracing() -> anyhow::Result<LogGuard> {
    let config = LogConfig::from_env()?;
    let (filter_layer, reload_handle) = reload::Layer::new(build_filter(config.sql_enabled, &[])?);

    // 目录不存在时 max_log_files 清理逻辑会报错，先创建。
    std::fs::create_dir_all(&config.dir)
        .with_context(|| format!("failed to create log directory {}", config.dir))?;

    let appender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("aries-server")
        .max_log_files(config.retention_days)
        .build(&config.dir)
        .with_context(|| format!("failed to create log directory {}", config.dir))?;
    let (file_writer, guard) = tracing_appender::non_blocking(appender);
    let file_layer = fmt::layer()
        .json()
        .with_ansi(false)
        .with_writer(file_writer);

    let console_layer: Box<dyn Layer<_> + Send + Sync> = if config.console_json {
        fmt::layer().json().boxed()
    } else {
        fmt::layer().pretty().boxed()
    };

    let (tx, rx) = mpsc::channel(LOG_CHANNEL_CAPACITY);
    // 丢弃计数由句柄共享：Admin 端可观测 channel 满导致的累计丢弃量。
    let dropped = Arc::new(AtomicU64::new(0));
    let db_layer = DbLogLayer::with_counter(tx, dropped.clone());

    // writer Span 标记层：让三个输出层的 Filter 能识别并消音 writer 自身的 DB 日志
    //（否则每条批量 INSERT 都会在控制台/文件/数据库三路各出现一次，空闲时也刷噪音）。
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(QuietInternalSpanLayer)
        .with(console_layer.with_filter(not_in_quiet_internal_scope()))
        .with(file_layer.with_filter(not_in_quiet_internal_scope()))
        .with(db_layer.with_filter(not_in_quiet_internal_scope()))
        .init();

    Ok(LogGuard {
        worker: guard,
        handle: LogHandle {
            reload_handle: Some(Arc::new(reload_handle)),
            sql_enabled: Arc::new(AtomicBool::new(config.sql_enabled)),
            dropped,
            custom_directives: Arc::new(std::sync::Mutex::new(String::new())),
            generation: Arc::new(AtomicU64::new(0)),
            restore_at: Arc::new(std::sync::Mutex::new(None)),
        },
        receiver: rx,
        retention_days: config.retention_days,
    })
}

fn parse_retention(value: &str) -> anyhow::Result<usize> {
    let days = value
        .parse::<usize>()
        .context("LOG_RETENTION_DAYS must be a positive integer")?;
    if days == 0 {
        bail!("LOG_RETENTION_DAYS must be at least 1");
    }
    Ok(days)
}

/// LOG_SQL 缺省策略：生产（console_json=true，即 APP_ENV=production）默认关闭，
/// 其余环境默认开启。显式设置 LOG_SQL 时以显式值为准。
fn default_sql_enabled(console_json: bool) -> bool {
    !console_json
}

fn parse_flag(value: &str) -> anyhow::Result<bool> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => bail!("LOG_SQL must be true or false"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_rejects_zero_and_non_numeric() {
        assert_eq!(parse_retention("7").unwrap(), 7);
        assert!(parse_retention("0").is_err());
        assert!(parse_retention("abc").is_err());
    }

    #[test]
    fn flag_accepts_only_boolean_forms() {
        assert!(parse_flag("true").unwrap());
        assert!(parse_flag("1").unwrap());
        assert!(!parse_flag("false").unwrap());
        assert!(parse_flag("yes").is_err());
    }

    #[test]
    fn sql_toggle_appends_query_directive() {
        let filter = build_filter(true, &[]).unwrap().to_string();
        assert!(filter.contains("sqlx::query=debug"));
    }

    #[test]
    fn noop_handle_tracks_sql_toggle_without_reloading() {
        let handle = LogHandle::noop();
        assert!(!handle.sql_enabled());
        handle.set_sql_logging(true).unwrap();
        assert!(handle.sql_enabled());
        handle.set_sql_logging(false).unwrap();
        assert!(!handle.sql_enabled());
    }

    #[test]
    fn rebuilt_filter_reflects_toggle() {
        let on = build_filter(true, &[]).unwrap().to_string();
        let off = build_filter(false, &[]).unwrap().to_string();
        assert!(on.contains("sqlx::query=debug"));
        assert!(!off.contains("sqlx::query=debug"));
    }

    #[test]
    fn rebuilt_filter_includes_custom_directives() {
        let custom = vec![Directive::from_str("aries_server=debug").unwrap()];
        let filter = build_filter(false, &custom).unwrap().to_string();
        assert!(filter.contains("aries_server=debug"));
    }

    #[test]
    fn validate_directives_normalizes_and_rejects_invalid() {
        assert_eq!(
            validate_directives(" aries_server=debug ,tower_http=debug ").unwrap(),
            "aries_server=debug, tower_http=debug"
        );
        assert_eq!(validate_directives("").unwrap(), "");
        assert_eq!(validate_directives(" , ").unwrap(), "");
        assert!(validate_directives("==nonsense").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn custom_override_auto_restores_after_deadline() {
        let handle = LogHandle::noop();
        handle
            .set_custom_directives(
                "aries_server=debug",
                Some(std::time::Duration::from_secs(30)),
            )
            .unwrap();
        assert_eq!(handle.custom_directives(), "aries_server=debug");
        assert!(handle.restore_in().is_some());

        // 到期自动复位：覆盖清空、deadline 清除。
        tokio::time::sleep(std::time::Duration::from_secs(31)).await;
        tokio::task::yield_now().await;
        assert_eq!(handle.custom_directives(), "");
        assert!(handle.restore_in().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn renewed_override_cancels_pending_restore() {
        let handle = LogHandle::noop();
        handle
            .set_custom_directives("a=debug", Some(std::time::Duration::from_secs(30)))
            .unwrap();
        // 到期前再次设置（无自动复位）：代际变化使前一个复位任务失效。
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        handle.set_custom_directives("b=debug", None).unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(25)).await;
        tokio::task::yield_now().await;
        assert_eq!(handle.custom_directives(), "b=debug");
        assert!(handle.restore_in().is_none());
    }

    #[test]
    fn sql_logging_defaults_on_for_development_and_off_for_production() {
        // 开发（console_json = false）默认开启，生产默认关闭。
        assert!(default_sql_enabled(false));
        assert!(!default_sql_enabled(true));
    }
}
