//! 运行日志的 PostgreSQL 落库：tracing Layer 把事件送入 channel，
//! 后台任务批量写入 `server_logs` 并定时清理过期数据。
//! 运行日志属可丢弃数据：channel 满时丢弃比阻塞业务更重要。

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Map, Value};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// 批量写入阈值：攒满即写，不等下一个 tick。
const BATCH_SIZE: usize = 100;
/// 批量写入的 flush 间隔。
const FLUSH_INTERVAL: Duration = Duration::from_millis(500);
/// 过期清理间隔（含启动时一次）。
const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);
/// 优雅退出时排空阶段的总时长上限：DB 挂死时放弃剩余记录，保证进程能退出。
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
/// 单值截断上限（字符数）：防止超长 SQL / 消息撑大 jsonb 行。
const MAX_VALUE_CHARS: usize = 8192;
const TRUNCATED_MARKER: &str = "…[truncated]";

/// SSE tail 唤醒 channel 容量：语义是「有新日志落库了」，不带 payload、幂等，
/// 溢出（Lagged）只等价于合并多次唤醒，接收端据此再 poll 一次即可。
const WAKE_CHANNEL_CAPACITY: usize = 16;

/// 日志落库唤醒信号的发送端：writer 每次批量 INSERT 成功后 send 一次；
/// SSE tail 连接经 `AppState` 持有它并 `subscribe()`，把轮询改为唤醒驱动。
pub type LogWakeSender = broadcast::Sender<()>;

/// 创建 tail 唤醒 channel，返回发送端（接收端由订阅方按需 `subscribe()`）。
/// 无订阅者时 `send` 返回 Err，属正常（无人监听），调用方直接忽略。
pub fn log_wake_channel() -> LogWakeSender {
    broadcast::channel(WAKE_CHANNEL_CAPACITY).0
}

/// 键名（不区分大小写）包含以下子串即视为敏感，值一律替换为 "***"。
/// 与请求参数脱敏（`crate::lib`）共用同一份清单，落库层再兜底一道。
const SENSITIVE_KEY_FRAGMENTS: [&str; 8] = [
    "password",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "authorization",
    "cookie",
];

pub(crate) fn is_sensitive_key(key: &str) -> bool {
    SENSITIVE_KEY_FRAGMENTS
        .iter()
        .any(|fragment| key.to_lowercase().contains(fragment))
}

/// 形似密码哈希的值（Argon2 / bcrypt 的 modular crypt 前缀）整串脱敏：
/// SQL 绑定参数里会出现哈希值（如登录、改密的 INSERT/UPDATE），键名无法定位到它。
fn looks_like_password_hash(value: &str) -> bool {
    value.contains("$argon2")
        || value.contains("$2a$")
        || value.contains("$2b$")
        || value.contains("$2y$")
}

/// query string 逐参数脱敏：`a=1&token=x` → `a=1&token=***`；键不敏感的参数原样保留。
fn redact_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((key, _)) if is_sensitive_key(key) => format!("{key}=***"),
            _ => pair.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// 落库前的最后一道脱敏防线（请求参数在源头已脱敏，此处覆盖 SQL 参数、
/// query string 与业务字段里漏网的敏感值）：
/// 敏感键名整体替换；`http.query` 按参数逐键判断；形似密码哈希的值整串替换。
fn redact_fields(fields: &mut Map<String, Value>) {
    for (name, value) in fields.iter_mut() {
        if is_sensitive_key(name) {
            *value = Value::String("***".to_owned());
        } else if name == "http.query" {
            if let Some(query) = value.as_str() {
                *value = Value::String(redact_query(query));
            }
        } else if let Some(text) = value.as_str() {
            if looks_like_password_hash(text) {
                *value = Value::String("***".to_owned());
            }
        }
    }
}

/// 超长值截断并追加标记，避免单行日志无限膨胀。
fn truncate_value(value: &str) -> String {
    if value.chars().count() > MAX_VALUE_CHARS {
        let mut cut: String = value.chars().take(MAX_VALUE_CHARS).collect();
        cut.push_str(TRUNCATED_MARKER);
        cut
    } else {
        value.to_owned()
    }
}

/// 一条日志事件的结构化记录（与 `server_logs` 列一一对应）。
#[derive(Debug, Clone)]
pub struct LogRecord {
    pub ts: OffsetDateTime,
    pub level: String,
    pub target: String,
    pub message: String,
    pub span_name: Option<String>,
    pub request_id: Option<String>,
    pub fields: Value,
}

/// Span 扩展：TraceLayer 创建的请求 Span 携带的 HTTP 上下文。
/// 事件落库时把 method/path/query/request_id 注入 fields，一次请求的每条日志
///（含 SQL 日志）都能直接看到 URL 与参数。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SpanHttpContext {
    request_id: Option<String>,
    method: Option<String>,
    path: Option<String>,
    query: Option<String>,
}

/// 事件字段提取：message 单独成列，其余进 fields JSON。
#[derive(Default)]
struct EventVisitor {
    message: String,
    fields: Map<String, Value>,
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            let value = trim_sql_field(field.name(), value);
            self.fields
                .insert(field.name().to_owned(), Value::String(value));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message = rendered;
        } else {
            let rendered = trim_sql_field(field.name(), &rendered);
            self.fields
                .insert(field.name().to_owned(), Value::String(rendered));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        let number = serde_json::Number::from_f64(value);
        self.fields.insert(
            field.name().to_owned(),
            number.map_or(Value::Null, Value::Number),
        );
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), Value::from(value));
    }
}

/// sqlx-core 在 `db.statement` 中硬编码了 `"\n\n{}\n"` 格式；
/// 对这两个字段做 trim，避免入库时前后有多余空行。
fn trim_sql_field(name: &str, value: &str) -> String {
    match name {
        "db.statement" | "summary" => value.trim().to_owned(),
        _ => value.to_owned(),
    }
}

/// 携带 SQL 参数调试信息的 Span 扩展。
/// 由 `logged_query` 在执行前写入，`on_event` 读取后注入 `db.parameters`。
#[derive(Clone)]
pub(crate) struct SqlParams(pub String);

/// Span 属性提取：请求 Span 的 request_id/method/path/query 字段，
/// 以及 `sql_params` Span 的 `db.parameters` 字段。
#[derive(Default)]
struct SpanVisitor {
    context: SpanHttpContext,
    db_params: Option<String>,
}

impl Visit for SpanVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "request_id" => self.context.request_id = Some(value.to_owned()),
            "method" => self.context.method = Some(value.to_owned()),
            "path" => self.context.path = Some(value.to_owned()),
            "query" => self.context.query = Some(value.to_owned()),
            "db.parameters" => self.db_params = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{value:?}");
        match field.name() {
            "request_id" => self.context.request_id = Some(rendered),
            "method" => self.context.method = Some(rendered),
            "path" => self.context.path = Some(rendered),
            "query" => self.context.query = Some(rendered),
            "db.parameters" => self.db_params = Some(rendered),
            _ => {}
        }
    }
}

/// 把 tracing 事件转发进 channel 的 Layer。从启动就在位：
/// DB 就绪前的早期日志暂存 channel，由 writer 任务启动后补写。
pub struct DbLogLayer {
    sender: mpsc::Sender<LogRecord>,
    /// 丢弃计数：channel 满时 try_send 失败即丢，节流打 warn（防日志风暴打爆 DB）。
    /// 计数器可与 `logging::LogHandle` 共享，供 Admin 端观测累计丢弃量。
    dropped: std::sync::Arc<AtomicU64>,
    last_drop_warn: Mutex<Option<std::time::Instant>>,
}

impl DbLogLayer {
    pub fn new(sender: mpsc::Sender<LogRecord>) -> Self {
        Self::with_counter(sender, std::sync::Arc::new(AtomicU64::new(0)))
    }

    /// 共享外部计数器：进程内由 `init_tracing` 创建并同时交给 `LogHandle` 暴露给 Admin API。
    pub fn with_counter(
        sender: mpsc::Sender<LogRecord>,
        dropped: std::sync::Arc<AtomicU64>,
    ) -> Self {
        Self {
            sender,
            dropped,
            last_drop_warn: Mutex::new(None),
        }
    }

    /// 递归防护判定：自身批量 INSERT 走 sqlx::query 日志。
    /// sqlx 0.8 的 query 事件把 SQL 文本放在 `summary` / `db.statement` 字段
    ///（普通事件 message 为空），三个来源任一含 server_logs 即丢弃。
    /// 只拦这条 INSERT；用户业务 SQL 日志（不含 server_logs 字样）照常入库。
    fn is_self_insert(target: &str, message: &str, fields: &Map<String, Value>) -> bool {
        if target != "sqlx::query" {
            return false;
        }
        if message.contains("server_logs") {
            return true;
        }
        ["summary", "db.statement"].iter().any(|key| {
            fields
                .get(*key)
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains("server_logs"))
        })
    }

    fn emit(&self, record: LogRecord) {
        if self.sender.try_send(record).is_err() {
            let dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            let mut last_warn = self.last_drop_warn.lock().unwrap();
            let should_warn = last_warn
                .map(|instant| instant.elapsed() >= Duration::from_secs(60))
                .unwrap_or(true);
            if should_warn {
                *last_warn = Some(std::time::Instant::now());
                // 直接走 eprintln：此时 channel 已满，再进 tracing 只会再被丢弃。
                eprintln!("server log channel full, {dropped} records dropped so far");
            }
        }
    }
}

impl<S> Layer<S> for DbLogLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        let mut visitor = SpanVisitor::default();
        attrs.record(&mut visitor);
        if let Some(span) = ctx.span(id) {
            if visitor.context != SpanHttpContext::default() {
                span.extensions_mut().insert(visitor.context);
            }
            if let Some(params) = visitor.db_params {
                span.extensions_mut().insert(SqlParams(params));
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        let metadata = event.metadata();
        if Self::is_self_insert(metadata.target(), &visitor.message, &visitor.fields) {
            return;
        }

        // 沿 Span 栈向上找最近的 HTTP 上下文与 SQL 参数；span_name 取最近 Span。
        // sql_params Span（logged_query 注入参数用）不算来源 Span，取 span_name 时跳过。
        let mut span_name = None;
        let mut http_context: Option<SpanHttpContext> = None;
        let mut sql_params: Option<String> = None;
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope {
                if span_name.is_none() && span.name() != aries_infra::logged::SQL_PARAMS_SPAN {
                    span_name = Some(span.name().to_owned());
                }
                if http_context.is_none() {
                    if let Some(context) = span.extensions().get::<SpanHttpContext>() {
                        http_context = Some(context.clone());
                    }
                }
                if sql_params.is_none() {
                    if let Some(params) = span.extensions().get::<SqlParams>() {
                        sql_params = Some(params.0.clone());
                    }
                }
                if span_name.is_some() && http_context.is_some() && sql_params.is_some() {
                    break;
                }
            }
        }

        let mut fields = visitor.fields;
        let mut request_id = None;
        if let Some(context) = http_context {
            request_id = context.request_id.clone();
            // HTTP 上下文注入 fields；事件自身已有同名字段时不覆盖。
            let inject = [
                ("http.method", context.method),
                ("http.path", context.path),
                ("http.query", context.query),
                ("http.request_id", context.request_id),
            ];
            for (key, value) in inject {
                if let Some(value) = value {
                    fields.entry(key.to_owned()).or_insert(Value::String(value));
                }
            }
        }
        // SQL 参数注入；事件自身已有同名字段时不覆盖。
        if let Some(params) = sql_params {
            fields
                .entry("db.parameters".to_owned())
                .or_insert(Value::String(params));
        }
        // 落库前脱敏（敏感键 / query string / 密码哈希形态的值）并截断超长值。
        redact_fields(&mut fields);
        for value in fields.values_mut() {
            if let Some(text) = value.as_str() {
                *value = Value::String(truncate_value(text));
            }
        }

        self.emit(LogRecord {
            ts: OffsetDateTime::now_utc(),
            level: metadata.level().to_string(),
            target: metadata.target().to_owned(),
            message: truncate_value(&visitor.message),
            span_name,
            request_id,
            fields: Value::Object(fields),
        });
    }
}

/// 内部静默操作的 Span 名；输出层 Filter 据此消音例行维护产生的噪音日志。
/// 使用方：日志写入任务（批量 INSERT/过期清理）、Worker 的空轮询领任务查询。
pub const QUIET_INTERNAL_SPAN: &str = "quiet_internal";

/// Span 扩展标记：该 Span 属于内部静默操作。
struct InQuietInternalSpan;

/// 辅助 Layer：on_new_span 时把静默标记写进 span extensions，
/// 供输出层 Filter 沿 Span 作用域向上查询。
pub struct QuietInternalSpanLayer;

impl<S> Layer<S> for QuietInternalSpanLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        if attrs.metadata().name() == QUIET_INTERNAL_SPAN {
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(InQuietInternalSpan);
            }
        }
    }
}

/// 输出层过滤器：静默 Span 作用域内只消音 INFO/DEBUG/TRACE 例行噪音——
/// 内部例行操作（日志落库、Worker 空轮询）的查询日志在控制台、文件、数据库三路都不再出现；
/// WARN/ERROR 是故障告警（批量 INSERT 失败、过期清理失败、轮询查询失败等），一律放行。
/// 只过滤事件；Span 本身放行（Span 被过滤会破坏作用域链条）。
pub fn not_in_quiet_internal_scope<S>() -> impl tracing_subscriber::layer::Filter<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    tracing_subscriber::filter::DynFilterFn::new(|metadata, ctx| {
        if !metadata.is_event() {
            return true;
        }
        if *metadata.level() <= tracing::Level::WARN {
            return true;
        }
        if let Some(current) = ctx.lookup_current() {
            for span in current.scope() {
                if span.extensions().get::<InQuietInternalSpan>().is_some() {
                    return false;
                }
            }
        }
        true
    })
}

/// 批量 INSERT：UNNEST 数组一次多行，减少 round trip。
/// 返回是否写入成功：成功后调用方据此广播 tail 唤醒信号（失败不唤醒，
/// 订阅端等下一次唤醒或兜底 tick 再拉取，不会丢日志）。
async fn insert_batch(pool: &PgPool, batch: &[LogRecord]) -> bool {
    let (ts, levels, targets, messages, span_names, request_ids, fields): (
        Vec<_>,
        Vec<_>,
        Vec<_>,
        Vec<_>,
        Vec<_>,
        Vec<_>,
        Vec<_>,
    ) = batch
        .iter()
        .map(|record| {
            (
                record.ts,
                record.level.clone(),
                record.target.clone(),
                record.message.clone(),
                record.span_name.clone(),
                record.request_id.clone(),
                record.fields.clone(),
            )
        })
        .collect();
    let result = sqlx::query(
        "INSERT INTO server_logs (ts, level, target, message, span_name, request_id, fields) \
         SELECT * FROM UNNEST(\
            $1::timestamptz[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::jsonb[])",
    )
    .bind(ts)
    .bind(levels)
    .bind(targets)
    .bind(messages)
    .bind(span_names)
    .bind(request_ids)
    .bind(fields)
    .execute(pool)
    .await;
    if let Err(error) = result {
        tracing::error!(error = %error, "failed to flush server logs");
        return false;
    }
    true
}

/// 过期清理：物理 DELETE 超过保留期的运行日志。
async fn cleanup_expired(pool: &PgPool, retention_days: usize) {
    let days = i32::try_from(retention_days).unwrap_or(14);
    let result =
        sqlx::query("DELETE FROM server_logs WHERE ts < now() - make_interval(days => $1)")
            .bind(days)
            .execute(pool)
            .await;
    if let Err(error) = result {
        tracing::error!(error = %error, "failed to clean up expired server logs");
    }
}

/// 统一的落库路径：INSERT 成功后广播 tail 唤醒（无订阅者时 send 返回 Err，
/// 属正常，直接忽略），随后清空批次。
async fn flush_batch(pool: &PgPool, wake: &LogWakeSender, batch: &mut Vec<LogRecord>) {
    use tracing::Instrument;
    if insert_batch(pool, batch)
        .instrument(tracing::info_span!(QUIET_INTERNAL_SPAN))
        .await
    {
        let _ = wake.send(());
    }
    batch.clear();
}

/// 启动日志写入后台任务：每 500ms 或攒满 100 条批量写入；
/// 启动时与每小时执行过期清理。应在 Migration 完成后调用。
///
/// 每次批量 INSERT 成功（含排空阶段）后向 `wake` 广播一次唤醒信号，
/// SSE tail 连接据此按需拉取，空闲时零 DB 轮询；无订阅者时 send 返回 Err，忽略。
///
/// `shutdown` 是优雅退出信号（`true` 或发送端 drop 均触发）：收到后任务不再
/// 接收新 select 轮次，先把当前批次落库，再把 channel 中残留记录分批写完后退出，
/// 由调用方 `await` 返回的 JoinHandle 等待排空完成（见 main 的退出路径）。
pub fn spawn_writer(
    pool: PgPool,
    mut receiver: mpsc::Receiver<LogRecord>,
    retention_days: usize,
    mut shutdown: watch::Receiver<bool>,
    wake: LogWakeSender,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // writer 自身的 DB 操作包在标记 Span 内：输出层 Filter 据此消音例行噪音，
        // 控制台/文件/数据库三路都不会出现 writer 自己的 INSERT/DELETE 查询日志；
        // 批量写入或清理失败时的 WARN/ERROR 告警不受静默影响，照常可见。
        // 每次操作新建 Span 而非常驻持有：Span 会让 Subscriber（含 channel
        // sender）保持存活，常驻持有会导致发送端永不释放、任务无法退出。
        use tracing::Instrument;
        cleanup_expired(&pool, retention_days)
            .instrument(tracing::info_span!(QUIET_INTERNAL_SPAN))
            .await;

        let mut batch: Vec<LogRecord> = Vec::with_capacity(BATCH_SIZE);
        let mut flush_tick = tokio::time::interval(FLUSH_INTERVAL);
        flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut retention_tick = tokio::time::interval(RETENTION_INTERVAL);
        retention_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = flush_tick.tick() => {
                    if !batch.is_empty() {
                        flush_batch(&pool, &wake, &mut batch).await;
                    }
                }
                maybe = receiver.recv() => {
                    match maybe {
                        Some(record) => {
                            batch.push(record);
                            if batch.len() >= BATCH_SIZE {
                                flush_batch(&pool, &wake, &mut batch).await;
                            }
                        }
                        // 发送端全部释放：与 shutdown 信号走同一排空路径。
                        None => break,
                    }
                }
                _ = retention_tick.tick() => {
                    cleanup_expired(&pool, retention_days)
                        .instrument(tracing::info_span!(QUIET_INTERNAL_SPAN))
                        .await;
                }
                // 优雅退出信号：发送端被 drop（未显式 send）同样触发。
                _ = shutdown.changed() => break,
            }
        }

        // 排空：先写当前批次，再循环 try_recv 把 channel 残留记录分批写完。
        // 语义是「尽力排空，不阻塞进程退出」：只处理已进入 channel 的记录，
        // 排空期间其他任务（如后台轮询）新产生的日志不保证落库；
        // 整体受 DRAIN_TIMEOUT 保护，DB 挂死时放弃剩余记录让进程退出。
        // 单批 INSERT 失败与常规路径一致：记 error 后丢弃该批，继续后续批次。
        let drain = async {
            if !batch.is_empty() {
                flush_batch(&pool, &wake, &mut batch).await;
            }
            while let Ok(record) = receiver.try_recv() {
                batch.push(record);
                if batch.len() >= BATCH_SIZE {
                    flush_batch(&pool, &wake, &mut batch).await;
                }
            }
            if !batch.is_empty() {
                flush_batch(&pool, &wake, &mut batch).await;
            }
        };
        if tokio::time::timeout(DRAIN_TIMEOUT, drain).await.is_err() {
            tracing::error!(
                timeout_ms = DRAIN_TIMEOUT.as_millis() as u64,
                "log writer drain timed out on shutdown; dropping remaining records"
            );
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_insert_detection_only_filters_own_batch_insert() {
        let fields_with = |sql: &str| {
            let mut fields = Map::new();
            fields.insert("summary".to_owned(), Value::String(sql.to_owned()));
            fields
        };
        // 自身批量 INSERT 的 sqlx 日志必须丢弃（递归防护）。
        assert!(DbLogLayer::is_self_insert(
            "sqlx::query",
            "",
            &fields_with("INSERT INTO server_logs ...")
        ));
        // message 携带时也丢弃（兼容旧版事件形状）。
        assert!(DbLogLayer::is_self_insert(
            "sqlx::query",
            "INSERT INTO server_logs ...",
            &Map::new()
        ));
        // 用户业务 SQL 日志照常入库。
        assert!(!DbLogLayer::is_self_insert(
            "sqlx::query",
            "",
            &fields_with("SELECT * FROM articles")
        ));
        // 其他 target 不受此规则影响。
        assert!(!DbLogLayer::is_self_insert(
            "aries_server",
            "server_logs mentioned in business log",
            &Map::new()
        ));
    }

    /// 复现回归：sqlx 0.8 的 query 事件把 SQL 文本放在 `summary` / `db.statement`
    /// 字段而非 message（普通事件 message 为空），递归防护必须覆盖这些字段。
    #[test]
    fn sqlx_self_insert_event_with_summary_field_is_dropped() {
        use tracing_subscriber::layer::SubscriberExt;

        let (tx, mut rx) = mpsc::channel(16);
        let subscriber = tracing_subscriber::registry().with(DbLogLayer::new(tx));
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            // 贴近 sqlx-core 0.8.6 真实事件：无 message，summary/db.statement 携带 SQL。
            tracing::debug!(
                target: "sqlx::query",
                summary = "INSERT INTO server_logs (ts, level) SELECT * FROM UNNEST($1::timestamptz[])",
                db.statement = "\n\nINSERT INTO server_logs (ts, level) SELECT * FROM UNNEST($1::timestamptz[])\n",
            );
            // 对照：业务 SQL 照常入库。
            tracing::debug!(
                target: "sqlx::query",
                summary = "SELECT id FROM articles",
                db.statement = "SELECT id FROM articles",
            );
        });
        drop(dispatch);

        // 自身 INSERT 被丢弃，只有业务 SQL 进 channel。
        let record = rx.try_recv().expect("business sql event");
        assert_eq!(
            record.fields["summary"],
            serde_json::json!("SELECT id FROM articles")
        );
        assert!(rx.try_recv().is_err(), "self-insert event must be dropped");
    }

    /// 静默作用域过滤：quiet_internal Span 内的 INFO/DEBUG 噪音被禁用，
    /// 但 WARN/ERROR 故障告警照常通过，Span 外部事件不受影响。
    #[test]
    fn writer_scope_filter_suppresses_events_inside_writer_span() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::layer::SubscriberExt;

        struct CaptureLayer {
            messages: Arc<Mutex<Vec<String>>>,
        }
        impl<S> Layer<S> for CaptureLayer
        where
            S: Subscriber + for<'span> LookupSpan<'span>,
        {
            fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
                let mut visitor = EventVisitor::default();
                event.record(&mut visitor);
                self.messages.lock().unwrap().push(visitor.message);
            }
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let capture = CaptureLayer {
            messages: captured.clone(),
        };
        let subscriber = tracing_subscriber::registry()
            .with(QuietInternalSpanLayer)
            .with(capture.with_filter(not_in_quiet_internal_scope()));
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("outside writer");
            let span = tracing::info_span!(QUIET_INTERNAL_SPAN);
            let _guard = span.enter();
            tracing::info!("inside writer");
            tracing::debug!(target: "sqlx::query", summary = "INSERT INTO server_logs ...");
            tracing::warn!("writer flush failed");
            tracing::error!("writer cleanup failed");
        });
        drop(dispatch);

        let messages = captured.lock().unwrap();
        assert_eq!(
            messages.as_slice(),
            [
                "outside writer",
                "writer flush failed",
                "writer cleanup failed"
            ]
        );
    }

    /// 落库脱敏：敏感键名整体替换、query string 逐参数脱敏、密码哈希形态的值整串替换；
    /// 非敏感字段（如 object_key）原样保留。
    #[test]
    fn redact_fields_scrubs_sensitive_names_query_and_hash_values() {
        let mut fields = Map::new();
        fields.insert("password".to_owned(), Value::String("hunter2".to_owned()));
        fields.insert("X-Api-Key".to_owned(), Value::String("abc".to_owned()));
        fields.insert(
            "http.query".to_owned(),
            Value::String("page=1&access_token=sekrit&tag=a%20b".to_owned()),
        );
        fields.insert(
            "db.parameters".to_owned(),
            Value::String("$1 = 'alice', $2 = '$argon2id$v=19$m=1,t=1'".to_owned()),
        );
        fields.insert(
            "object_key".to_owned(),
            Value::String("2026/09/abc.png".to_owned()),
        );
        fields.insert("status".to_owned(), Value::from(200_u16));

        redact_fields(&mut fields);

        assert_eq!(fields["password"], serde_json::json!("***"));
        assert_eq!(fields["X-Api-Key"], serde_json::json!("***"));
        assert_eq!(
            fields["http.query"],
            serde_json::json!("page=1&access_token=***&tag=a%20b")
        );
        assert_eq!(fields["db.parameters"], serde_json::json!("***"));
        assert_eq!(fields["object_key"], serde_json::json!("2026/09/abc.png"));
        assert_eq!(fields["status"], serde_json::json!(200));
    }

    /// 截断：超长值截到上限并追加标记，短值原样返回。
    #[test]
    fn truncate_value_cuts_long_strings_with_marker() {
        let short = "hello";
        assert_eq!(truncate_value(short), short);
        let long = "x".repeat(MAX_VALUE_CHARS + 100);
        let cut = truncate_value(&long);
        assert!(cut.ends_with(TRUNCATED_MARKER));
        assert_eq!(
            cut.chars().count(),
            MAX_VALUE_CHARS + TRUNCATED_MARKER.chars().count()
        );
    }

    /// 事件 → record 转换：用真实 subscriber 驱动 DbLogLayer，验证字段提取与递归过滤。
    #[test]
    fn events_are_converted_to_records_with_span_context() {
        use tracing_subscriber::layer::SubscriberExt;

        let (tx, mut rx) = mpsc::channel(16);
        let subscriber = tracing_subscriber::registry().with(DbLogLayer::new(tx));
        let dispatch = tracing::Dispatch::new(subscriber);

        tracing::dispatcher::with_default(&dispatch, || {
            // 普通事件：message 成列，其余字段进 fields。
            tracing::info!(status = 200_u16, ok = true, "request completed");
            // 递归防护：sqlx::query + server_logs 被丢弃。
            tracing::debug!(target: "sqlx::query", "INSERT INTO server_logs (ts) VALUES ($1)");
            // Span 上下文：span_name、request_id 与 http.* 字段透传。
            let span = tracing::info_span!(
                "http_request",
                request_id = "req-123",
                method = "POST",
                path = "/api/admin/articles",
                query = "page=1",
            );
            let _guard = span.enter();
            tracing::warn!("slow query");
            // 事件自身已有 http.path 时不被覆盖。
            tracing::warn!(http.path = "/custom", "event with own path");
            // 参数 Span（logged_query 的形状）：注入 db.parameters，但不抢占 span_name。
            let sql_span = tracing::debug_span!(
                target: "sqlx::query",
                aries_infra::logged::SQL_PARAMS_SPAN,
                db.parameters = "$1 = 42",
            );
            let _sql_guard = sql_span.enter();
            tracing::debug!(target: "sqlx::query", summary = "SELECT * FROM articles WHERE id = $1");
        });
        drop(dispatch);

        let first = rx.try_recv().expect("first record");
        assert_eq!(first.level, "INFO");
        assert_eq!(first.message, "request completed");
        assert_eq!(first.fields["status"], serde_json::json!(200));
        assert_eq!(first.fields["ok"], serde_json::json!(true));
        assert!(first.request_id.is_none());

        // 第二条应为 warn（sqlx 自写日志已被过滤，不在队列中）。
        let second = rx.try_recv().expect("second record");
        assert_eq!(second.level, "WARN");
        assert_eq!(second.message, "slow query");
        assert_eq!(second.span_name.as_deref(), Some("http_request"));
        assert_eq!(second.request_id.as_deref(), Some("req-123"));
        // Span 的 HTTP 上下文注入 fields。
        assert_eq!(second.fields["http.method"], serde_json::json!("POST"));
        assert_eq!(
            second.fields["http.path"],
            serde_json::json!("/api/admin/articles")
        );
        assert_eq!(second.fields["http.query"], serde_json::json!("page=1"));
        assert_eq!(
            second.fields["http.request_id"],
            serde_json::json!("req-123")
        );

        // 事件自带的 http.path 不被 Span 上下文覆盖。
        let third = rx.try_recv().expect("third record");
        assert_eq!(third.fields["http.path"], serde_json::json!("/custom"));

        // SQL 事件：参数 Span 注入 db.parameters；span_name 跳过 sql_params 取到 http_request。
        let fourth = rx.try_recv().expect("fourth record");
        assert_eq!(fourth.target, "sqlx::query");
        assert_eq!(fourth.fields["db.parameters"], serde_json::json!("$1 = 42"));
        assert_eq!(fourth.span_name.as_deref(), Some("http_request"));
        assert_eq!(fourth.request_id.as_deref(), Some("req-123"));

        // 没有更多记录（递归过滤生效）。
        assert!(rx.try_recv().is_err());
    }
}
