//! 运行日志域：`server_logs` 表（tracing 事件批量落库）的查询契约。
//! 运行时开关（SQL 日志、级别覆盖、tail 通道丢弃计数）走 `LogHandle`，不经 Repository。

use async_trait::async_trait;
use thiserror::Error;
use time::OffsetDateTime;

/// 一条运行日志（与 `server_logs` 行一一对应）。
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub id: i64,
    pub ts: OffsetDateTime,
    pub level: String,
    pub target: String,
    pub message: String,
    pub span_name: Option<String>,
    pub request_id: Option<String>,
    pub fields: serde_json::Value,
}

/// 各级别条数（24h 聚合与统计分桶共用，五键恒在）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogLevelCounts {
    pub error: i64,
    pub warn: i64,
    pub info: i64,
    pub debug: i64,
    pub trace: i64,
}

impl LogLevelCounts {
    /// 按级别字符串累加一条 GROUP BY 结果；未知级别忽略。
    pub fn accumulate(&mut self, level: &str, count: i64) {
        match level {
            "ERROR" => self.error = count,
            "WARN" => self.warn = count,
            "INFO" => self.info = count,
            "DEBUG" => self.debug = count,
            "TRACE" => self.trace = count,
            _ => {}
        }
    }
}

/// 统计分桶：桶起点 + 该桶内各级别条数。
#[derive(Debug, Clone)]
pub struct LogStatsBucket {
    pub bucket: OffsetDateTime,
    pub counts: LogLevelCounts,
}

/// 列表查询过滤条件：`keyword` 为已转义并加 `%` 通配的 ILIKE 模式；
/// `target` 与 `exclude_target` 互斥（由调用方校验）。
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// 最低级别集合（如 `["WARN", "ERROR"]`）；None 表示全部。
    pub levels: Option<Vec<String>>,
    pub target: Option<String>,
    pub exclude_target: Option<String>,
    pub keyword: Option<String>,
    pub request_id: Option<String>,
    /// 时间范围左闭右开。
    pub start: Option<OffsetDateTime>,
    pub end: Option<OffsetDateTime>,
}

/// 一页日志（不含 24h 级别聚合，由 `level_counts_24h` 单独查询）。
#[derive(Debug, Clone)]
pub struct LogList {
    pub items: Vec<LogEntry>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LogError {
    #[error("log entry not found")]
    NotFound,
    #[error("log store unavailable")]
    StoreUnavailable,
}

#[async_trait]
pub trait LogRepository: Send + Sync {
    /// 分页列表：按 filter 过滤，`order_asc` 决定 ts/id 排序方向。
    /// `cursor` 为 keyset 游标 `(ts, id)`（上一页末行）：存在时忽略 `page`，
    /// 取严格早于该行的下一页（仅 desc 语义，由调用方保证不与 asc 同用）。
    async fn list(
        &self,
        filter: &LogFilter,
        page: u32,
        page_size: u32,
        order_asc: bool,
        cursor: Option<(OffsetDateTime, i64)>,
    ) -> Result<LogList, LogError>;

    /// 上下文模式：以 `around_id` 行的 ts 为中心取前后各 `context` 条，按 ts asc 返回。
    /// 锚点不存在返回 `NotFound`。
    async fn list_around(
        &self,
        filter: &LogFilter,
        around_id: i64,
        context: u32,
    ) -> Result<LogList, LogError>;

    /// 最近 24h 各级别条数（一次 GROUP BY 聚合）。
    async fn level_counts_24h(&self) -> Result<LogLevelCounts, LogError>;

    /// 分桶统计：`hours` 为窗口（小时），≤48 按小时分桶、>48 按天分桶。
    async fn stats(&self, hours: u32) -> Result<Vec<LogStatsBucket>, LogError>;

    /// 出现过的 target 去重列表（上限 200）。
    async fn list_targets(&self) -> Result<Vec<String>, LogError>;

    /// tail 锚点：当前最大 id，只推送连接之后的新日志。
    /// 调用方负责把查询包进消音 Span（tail 高频轮询不应刷进 server_logs）。
    async fn tail_anchor(&self) -> Result<i64, LogError>;

    /// tail 轮询：`after_id` 之后的新日志，按 id 正序，最多 `limit` 条。
    /// 调用方负责把查询包进消音 Span。
    async fn tail_poll(
        &self,
        filter: &LogFilter,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<LogEntry>, LogError>;

    /// `since` 之后的 ERROR 条数：worker 周期尖峰检测用。
    /// 调用方负责把查询包进消音 Span（例行轮询不应刷进 server_logs）。
    async fn count_errors_since(&self, since: OffsetDateTime) -> Result<i64, LogError>;
}
