//! Admin 运行日志查询与 SQL 日志开关（仅 Owner / ManageSettings）。
//! 数据来自 `server_logs` 表（tracing 事件批量落库，见 `crate::log_store`），
//! 经 `LogRepository` 访问；运行时开关（SQL 日志、级别覆盖）走 `LogHandle`。

use aries_core::auth::Permission;
use aries_core::logs::{LogEntry, LogError, LogFilter, LogLevelCounts};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    response::Sse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tracing::Instrument;

use crate::log_store::QUIET_INTERNAL_SPAN;
use crate::state::AppState;

use super::auth::CurrentUser;
use super::error::ApiError;
use super::extract::ApiJson;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/logs", get(list_logs))
        .route("/logs/targets", get(list_log_targets))
        .route("/logs/stats", get(list_log_stats))
        .route("/logs/sql", get(get_sql_logging).put(set_sql_logging))
        .route(
            "/logs/filter",
            get(get_filter_override).put(set_filter_override),
        )
        .route("/logs/tail", get(tail_logs))
}

// ============================================================
// 日志查询
// ============================================================

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub(crate) struct LogListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 最低级别：WARN 含 WARN+ERROR；空为全部。
    level: Option<String>,
    target: Option<String>,
    /// message 与 fields（JSON 文本）模糊匹配（ILIKE，%/_ 会被转义）。
    keyword: Option<String>,
    /// 请求链路追踪：精确匹配 request_id，空串忽略。
    request_id: Option<String>,
    /// 精确排除该 target（如 `sqlx::query`）；与 `target` 互斥。
    exclude_target: Option<String>,
    /// 上下文模式锚点：以该行的 ts 为中心取前后日志。
    around_id: Option<i64>,
    /// 上下文模式窗口：前后各取 context 条，默认 20，上限 100。
    context: Option<u32>,
    /// 排序方向：asc/desc，默认 desc；链路场景用 asc 按时间正序看流转。
    order: Option<String>,
    /// keyset 游标：上一页响应的 `next_cursor`（`<ts unix 纳秒>_<id>`），仅 desc 有效；
    /// 存在时忽略 `page`，与 `order=asc` 互斥（同用返回 400 `INVALID_CURSOR`）。
    cursor: Option<String>,
    /// 时间范围左闭右开，RFC 3339。
    start: Option<String>,
    end: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LogItem {
    id: i64,
    /// RFC 3339 字符串（time 的默认 serde 是数组形式，显式格式化）。
    ts: String,
    level: String,
    target: String,
    message: String,
    span_name: Option<String>,
    request_id: Option<String>,
    fields: serde_json::Value,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LogLevelCountsResponse {
    #[serde(rename = "ERROR")]
    error: i64,
    #[serde(rename = "WARN")]
    warn: i64,
    #[serde(rename = "INFO")]
    info: i64,
    #[serde(rename = "DEBUG")]
    debug: i64,
    #[serde(rename = "TRACE")]
    trace: i64,
}

impl From<LogLevelCounts> for LogLevelCountsResponse {
    fn from(counts: LogLevelCounts) -> Self {
        Self {
            error: counts.error,
            warn: counts.warn,
            info: counts.info,
            debug: counts.debug,
            trace: counts.trace,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LogPageResponse {
    items: Vec<LogItem>,
    total: i64,
    page: u32,
    page_size: u32,
    /// keyset 下一页游标（`<ts unix 纳秒>_<id>`）：本页满页时取末行生成，否则为 null；
    /// 仅 desc 列表路径产生，asc（链路视图）与上下文模式恒为 null。
    next_cursor: Option<String>,
    /// 最近 24h 各级别条数（GROUP BY 聚合）。
    level_counts: LogLevelCountsResponse,
}

/// 最低级别过滤：返回该级别及以上的集合。
fn level_set(minimum: &str) -> Option<Vec<String>> {
    match minimum.to_ascii_uppercase().as_str() {
        "ERROR" => Some(vec!["ERROR".to_owned()]),
        "WARN" => Some(vec!["WARN".to_owned(), "ERROR".to_owned()]),
        "INFO" => Some(vec![
            "INFO".to_owned(),
            "WARN".to_owned(),
            "ERROR".to_owned(),
        ]),
        "DEBUG" => Some(vec![
            "DEBUG".to_owned(),
            "INFO".to_owned(),
            "WARN".to_owned(),
            "ERROR".to_owned(),
        ]),
        "TRACE" => None, // 全部
        _ => None,       // 未知值按全部处理（宽松，不报错）
    }
}

/// 将 RFC 3339 字符串解析为 `OffsetDateTime`，非法值返回统一 400。
fn parse_rfc3339(value: &str) -> Result<OffsetDateTime, ApiError> {
    OffsetDateTime::parse(value.trim(), &Rfc3339).map_err(|_| {
        ApiError::bad_request(
            "INVALID_DATE_RANGE",
            "start/end must be valid RFC 3339 timestamps",
        )
    })
}

/// keyset 游标：`<ts unix 纳秒>_<id>`（上一页末行），解析失败返回 400 `INVALID_CURSOR`。
/// unix 纳秒整数避免 RFC 3339 精度与 URL 转义问题；timestamptz 为微秒精度，
/// 从 DB 读出再编码的纳秒值回传后比较语义不变。
fn parse_cursor(value: &str) -> Result<(OffsetDateTime, i64), ApiError> {
    let invalid = || {
        ApiError::bad_request(
            "INVALID_CURSOR",
            "cursor must be the next_cursor value from a previous response",
        )
    };
    let (ts_part, id_part) = value.trim().split_once('_').ok_or_else(invalid)?;
    let nanos: i128 = ts_part.parse().map_err(|_| invalid())?;
    let id: i64 = id_part.parse().map_err(|_| invalid())?;
    let ts = OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|_| invalid())?;
    Ok((ts, id))
}

fn encode_cursor(ts: OffsetDateTime, id: i64) -> String {
    format!("{}_{id}", ts.unix_timestamp_nanos())
}

fn entry_to_item(entry: LogEntry) -> LogItem {
    LogItem {
        id: entry.id,
        ts: entry
            .ts
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
        level: entry.level,
        target: entry.target,
        message: entry.message,
        span_name: entry.span_name,
        request_id: entry.request_id,
        fields: entry.fields,
    }
}

fn build_filter(
    level: Option<String>,
    target: Option<String>,
    exclude_target: Option<String>,
    keyword: Option<String>,
    request_id: Option<String>,
    start: Option<String>,
    end: Option<String>,
) -> Result<LogFilter, ApiError> {
    let filter = LogFilter {
        levels: level.as_deref().map(level_set).unwrap_or(None),
        target: target.filter(|value| !value.trim().is_empty()),
        exclude_target: exclude_target.filter(|value| !value.trim().is_empty()),
        keyword: keyword
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("%{}%", aries_infra::like::escape_like(value.trim()))),
        request_id: request_id
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_owned()),
        start: start.as_deref().map(parse_rfc3339).transpose()?,
        end: end.as_deref().map(parse_rfc3339).transpose()?,
    };
    // target 与 exclude_target 互斥。
    if filter.target.is_some() && filter.exclude_target.is_some() {
        return Err(ApiError::bad_request(
            "INVALID_FILTER",
            "target and exclude_target are mutually exclusive",
        ));
    }
    Ok(filter)
}

#[utoipa::path(
    get,
    path = "/api/admin/logs",
    tag = "Admin Logs",
    operation_id = "listLogs",
    summary = "运行日志查询",
    description = "数据来自 `server_logs` 表（tracing 事件批量落库）。`start`/`end` 按 `ts` 左闭右开过滤，格式 RFC 3339，非法值返回 400 `INVALID_DATE_RANGE`；`target` 与 `exclude_target` 互斥，同用返回 400 `INVALID_FILTER`。默认 OFFSET 分页（`page`/`page_size`）；携带 `cursor`（上一页响应的 `next_cursor`，格式 `<ts unix 纳秒>_<id>`）进入 keyset 分页：仅支持 desc 排序，忽略 `page`，避免深翻页 OFFSET 扫描，`total` 与 `level_counts` 照常返回；游标非法或与 `order=asc` 同用返回 400 `INVALID_CURSOR`。提供 `around_id` 时进入上下文模式：以锚点行的 `ts` 为中心前后各取 `context` 条，忽略分页/时间范围/排序参数，锚点不存在返回 404 `LOG_ENTRY_NOT_FOUND`。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    params(LogListParams),
    responses(
        (status = 200, description = "运行日志分页，附最近 24 小时各级别条数", body = LogPageResponse),
        (status = 400, description = "时间格式非法、target/exclude_target 同用或游标非法/与 asc 同用", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "上下文模式锚点不存在（LOG_ENTRY_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_logs(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<LogListParams>,
) -> Result<Json<LogPageResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let filter = build_filter(
        params.level,
        params.target,
        params.exclude_target,
        params.keyword,
        params.request_id,
        params.start,
        params.end,
    )?;

    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 200);
    // 排序白名单：只允许两个方向，未知值按 desc 宽松处理。
    let order_asc = params.order.as_deref() == Some("asc");

    // keyset 游标：空串忽略；与 asc 互斥（链路视图日志量级小，保持 OFFSET 路径）。
    let cursor = params
        .cursor
        .filter(|value| !value.trim().is_empty())
        .map(|value| parse_cursor(&value))
        .transpose()?;
    if cursor.is_some() && order_asc {
        return Err(ApiError::bad_request(
            "INVALID_CURSOR",
            "cursor pagination requires desc order; order=asc uses offset pagination",
        ));
    }

    // 上下文模式：以锚点行 ts 为中心取前后日志，忽略分页/时间范围/排序参数。
    let (list, level_counts) = if let Some(around_id) = params.around_id {
        let context = params.context.unwrap_or(20).clamp(1, 100);
        let list = state.logs.list_around(&filter, around_id, context).await?;
        (list, state.logs.level_counts_24h().await?)
    } else {
        let list = state
            .logs
            .list(&filter, page, page_size, order_asc, cursor)
            .await?;
        (list, state.logs.level_counts_24h().await?)
    };

    // keyset 下一页游标：仅 desc 列表路径产生；本页满页时取末行，否则没有下一页。
    let next_cursor =
        if params.around_id.is_none() && !order_asc && list.items.len() as u32 == page_size {
            list.items
                .last()
                .map(|entry| encode_cursor(entry.ts, entry.id))
        } else {
            None
        };

    Ok(Json(LogPageResponse {
        items: list.items.into_iter().map(entry_to_item).collect(),
        total: list.total,
        page: list.page,
        page_size: list.page_size,
        next_cursor,
        level_counts: level_counts.into(),
    }))
}

// ============================================================
// 分桶统计
// ============================================================

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub(crate) struct LogStatsParams {
    /// 统计窗口（小时）：默认 24，范围 1–168；≤48h 按小时分桶，>48h 按天。
    hours: Option<u32>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LogStatsBucketResponse {
    /// 桶起点，RFC 3339。
    bucket: String,
    #[serde(flatten)]
    counts: LogLevelCountsResponse,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct LogStatsResponse {
    hours: u32,
    buckets: Vec<LogStatsBucketResponse>,
    /// 运行日志 channel 累计丢弃条数（channel 满时丢弃，属可丢弃数据）。
    channel_dropped: u64,
}

#[utoipa::path(
    get,
    path = "/api/admin/logs/stats",
    tag = "Admin Logs",
    operation_id = "listLogStats",
    summary = "运行日志分桶统计",
    description = "趋势图数据：`hours` 默认 24、范围 1–168；≤48 小时按小时分桶，>48 小时按天分桶（`date_trunc`）；桶按时间升序，五个级别键恒在（无数据为 0）。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    params(LogStatsParams),
    responses(
        (status = 200, description = "分桶统计", body = LogStatsResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_log_stats(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<LogStatsParams>,
) -> Result<Json<LogStatsResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let hours = params.hours.unwrap_or(24).clamp(1, 168);
    let buckets = state.logs.stats(hours).await?;

    Ok(Json(LogStatsResponse {
        hours,
        buckets: buckets
            .into_iter()
            .map(|bucket| LogStatsBucketResponse {
                bucket: bucket
                    .bucket
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
                counts: bucket.counts.into(),
            })
            .collect(),
        channel_dropped: state.log_handle.dropped_count(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/admin/logs/targets",
    tag = "Admin Logs",
    operation_id = "listLogTargets",
    summary = "已出现过的 tracing target 列表",
    description = "去重列表（升序，最多 200 条），用于筛选下拉框。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "target 列表", body = serde_json::Value),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn list_log_targets(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let targets = state.logs.list_targets().await?;
    Ok(Json(serde_json::json!({ "items": targets })))
}

// ============================================================
// SQL 日志运行时开关
// ============================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct SqlLoggingRequest {
    enabled: bool,
}

#[utoipa::path(
    get,
    path = "/api/admin/logs/sql",
    tag = "Admin Logs",
    operation_id = "getSqlLogging",
    summary = "查询 SQL 日志运行时开关状态",
    description = "查询 SQL 语句日志（`sqlx::query` DEBUG 级）的运行时开关状态。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "当前开关状态（`{ \"enabled\": bool }`）", body = serde_json::Value),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn get_sql_logging(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    current.require(Permission::ManageSettings)?;
    Ok(Json(serde_json::json!({
        "enabled": state.log_handle.sql_enabled(),
    })))
}

#[utoipa::path(
    put,
    path = "/api/admin/logs/sql",
    tag = "Admin Logs",
    operation_id = "setSqlLogging",
    summary = "运行时切换 SQL 日志开关",
    description = "运行时切换 SQL 语句日志开关（tracing_subscriber reload），无需重启；`LOG_SQL` 环境变量仅作为启动初始值。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    request_body(content = SqlLoggingRequest, content_type = "application/json", description = "目标开关状态"),
    responses(
        (status = 200, description = "切换后的开关状态（`{ \"enabled\": bool }`）", body = serde_json::Value),
        (status = 400, description = "请求体非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 500, description = "过滤器 reload 失败", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn set_sql_logging(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<SqlLoggingRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    current.require(Permission::ManageSettings)?;
    state
        .log_handle
        .set_sql_logging(request.enabled)
        .map_err(|_| ApiError::internal())?;
    Ok(Json(serde_json::json!({
        "enabled": state.log_handle.sql_enabled(),
    })))
}

// ============================================================
// 运行时日志级别覆盖（自定义 EnvFilter directives + 自动复位）
// ============================================================

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct FilterOverrideResponse {
    /// 当前生效的覆盖 directives，空串为无覆盖。
    directives: String,
    /// 自动复位剩余秒数；None 表示持续到被显式清除。
    restore_seconds_remaining: Option<u64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct SetFilterOverrideRequest {
    /// EnvFilter directives（逗号分隔），如 `aries_server=debug,tower_http=debug`；空串清除覆盖。
    directives: String,
    /// 自动复位时间（分钟）：到期未再变更则恢复默认过滤器；省略或 0 表示不复位。
    restore_minutes: Option<u32>,
}

fn override_response(handle: &crate::logging::LogHandle) -> FilterOverrideResponse {
    FilterOverrideResponse {
        directives: handle.custom_directives(),
        restore_seconds_remaining: handle.restore_in().map(|d| d.as_secs()),
    }
}

#[utoipa::path(
    get,
    path = "/api/admin/logs/filter",
    tag = "Admin Logs",
    operation_id = "getFilterOverride",
    summary = "查询运行时日志级别覆盖状态",
    description = "查询自定义 EnvFilter directives 与自动复位剩余时间，空 `directives` 表示无覆盖。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "当前覆盖状态", body = FilterOverrideResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn get_filter_override(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<FilterOverrideResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    Ok(Json(override_response(&state.log_handle)))
}

#[utoipa::path(
    put,
    path = "/api/admin/logs/filter",
    tag = "Admin Logs",
    operation_id = "setFilterOverride",
    summary = "运行时设置日志级别覆盖",
    description = "运行时设置日志级别覆盖（tracing_subscriber reload，无需重启），用于临时排障（如 `aries_server=debug`）。可带自动复位：到期未再变更则恢复默认过滤器；空 `directives` 清除覆盖（含未到期自动复位一并失效）。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    request_body(content = SetFilterOverrideRequest, content_type = "application/json", description = "覆盖 directives 与可选自动复位时间"),
    responses(
        (status = 200, description = "设置后的覆盖状态", body = FilterOverrideResponse),
        (status = 400, description = "directives 非法（INVALID_DIRECTIVES）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn set_filter_override(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<SetFilterOverrideRequest>,
) -> Result<Json<FilterOverrideResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    // 自动复位封顶一天：级别覆盖是排障临时手段，不应长期悬挂。
    let restore = request
        .restore_minutes
        .filter(|minutes| *minutes > 0)
        .map(|minutes| std::time::Duration::from_secs(u64::from(minutes.min(24 * 60)) * 60));
    state
        .log_handle
        .set_custom_directives(&request.directives, restore)
        .map_err(|error| {
            tracing::warn!(error = %error, "rejected invalid log filter directives");
            ApiError::bad_request(
                "INVALID_DIRECTIVES",
                "directives must be comma-separated EnvFilter directives like aries_server=debug",
            )
        })?;
    Ok(Json(override_response(&state.log_handle)))
}

// ============================================================
// 实时 tail：按筛选条件 SSE 推送连接后的新日志
// ============================================================

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub(crate) struct LogTailParams {
    /// 最低级别（同列表语义：WARN 含 WARN+ERROR）。
    level: Option<String>,
    target: Option<String>,
    /// 精确排除该 target（与 target 互斥）。
    exclude_target: Option<String>,
    keyword: Option<String>,
    request_id: Option<String>,
}

/// 每轮拉取的最大条数：突发高峰时分批推送，避免单条 SSE 消息过大。
const TAIL_BATCH_LIMIT: i64 = 500;
/// 兜底拉取间隔：正常路径由 writer 落库后的 broadcast 唤醒驱动（空闲时零 DB 查询），
/// 该间隔仅防御理论上的唤醒丢失（如 broadcast 缓冲溢出与订阅窗口叠加）。
const TAIL_FALLBACK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

#[utoipa::path(
    get,
    path = "/api/admin/logs/tail",
    tag = "Admin Logs",
    operation_id = "tailLogs",
    summary = "实时推送运行日志（SSE）",
    description = "新连接以连接时刻的最大日志 id 为锚点，推送之后写入且匹配筛选的新日志；EventSource 断线自动重连时携带 `Last-Event-ID` 请求头，服务端从该 id 之后继续推送，断开窗口的日志全部补齐（头部解析失败按新连接处理）。推送由日志落库的唤醒信号驱动，空闲时不查询数据库（30 秒兜底拉取一次）。事件为 `event: log`、`data` 为单个日志条目（形状同 `LogItem`），SSE `id:` 字段为日志行 id；锚点查询失败时推送 `event: error` 后结束。筛选参数语义与列表接口一致（`level` 为最低级别；`target` 与 `exclude_target` 互斥，同用返回 400 `INVALID_FILTER`）。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    params(LogTailParams),
    responses(
        (status = 200, description = "SSE 流（持续响应，客户端断开即结束）", content_type = "text/event-stream", body = LogItem),
        (status = 400, description = "target/exclude_target 同用等筛选参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
pub(crate) async fn tail_logs(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<LogTailParams>,
    headers: HeaderMap,
) -> Result<
    Sse<
        impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    ApiError,
> {
    use axum::response::sse::Event;

    current.require(Permission::ManageSettings)?;
    let filter = build_filter(
        params.level,
        params.target,
        params.exclude_target,
        params.keyword,
        params.request_id,
        None,
        None,
    )?;

    // EventSource 断线自动重连时携带 Last-Event-ID（上次收到的 SSE id，即日志行 id）：
    // 从该 id 之后继续推送，补齐断开窗口；头部缺失或解析失败按新连接处理。
    let resume_after = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<i64>().ok());

    // 先订阅唤醒再定锚点/首次拉取：两次之间的插入要么被首次拉取捞到，
    // 要么唤醒已滞留在 broadcast 缓冲里（订阅后立即触发一轮拉取），不会漏。
    let mut wake = state.log_wake.subscribe();

    let logs = state.logs.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(256);
    tokio::spawn(async move {
        // 连接锚点：续传直接用 Last-Event-ID（无需查询）；新连接取当前最大 id，
        // 只推送连接之后的新日志。锚点查询包进静默 Span：tail 的例行 SQL 不应刷进 server_logs。
        let mut last_id = match resume_after {
            Some(id) => id,
            None => {
                let anchor = async { logs.tail_anchor().await }
                    .instrument(tracing::info_span!(QUIET_INTERNAL_SPAN))
                    .await;
                match anchor {
                    Ok(max_id) => max_id,
                    Err(error @ LogError::StoreUnavailable) => {
                        let event = Event::default().event("error").data(format!(
                            "{{\"code\":\"TAIL_ANCHOR_FAILED\",\"message\":\"{error}\"}}"
                        ));
                        let _ = tx.send(Ok(event)).await;
                        return;
                    }
                    Err(LogError::NotFound) => unreachable!("tail anchor never returns NotFound"),
                }
            }
        };

        let mut fallback = tokio::time::interval(TAIL_FALLBACK_INTERVAL);
        fallback.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // interval 首个 tick 立即完成：消费掉，真正的兜底从 30 秒后开始。
        fallback.tick().await;

        // 发送端全部释放（writer 退出）后退化为纯兜底轮询，避免 Closed 立即返回造成忙等。
        let mut wake_open = true;
        loop {
            // 拉取并推送一轮；满批时继续拉，把积压（如续传大窗口）一次补齐。
            // 拉取查询包进静默 Span：同锚点查询，避免例行 SQL 刷进 server_logs。
            loop {
                let rows = async { logs.tail_poll(&filter, last_id, TAIL_BATCH_LIMIT).await }
                    .instrument(tracing::info_span!(QUIET_INTERNAL_SPAN))
                    .await;
                let rows = match rows {
                    Ok(rows) => rows,
                    Err(error) => {
                        // 单轮失败不终止流：DB 抖动时等下一次唤醒/兜底重试。
                        // 该 error 在静默 Span 之外记录，WARN/ERROR 本就放行，照常入库可见。
                        tracing::error!(error = %error, "log tail poll failed");
                        break;
                    }
                };
                if rows.is_empty() {
                    break;
                }
                let full_batch = rows.len() as i64 >= TAIL_BATCH_LIMIT;
                last_id = rows.last().map(|row| row.id).unwrap_or(last_id);
                for entry in rows {
                    let id = entry.id;
                    let item = entry_to_item(entry);
                    let Ok(data) = serde_json::to_string(&item) else {
                        continue;
                    };
                    // SSE id = 日志行 id：EventSource 重连时经 Last-Event-ID 回传，据此续传。
                    let event = Event::default().event("log").id(id.to_string()).data(data);
                    if tx.send(Ok(event)).await.is_err() {
                        // 客户端断开：终止推送任务。
                        return;
                    }
                }
                if !full_batch {
                    break;
                }
            }

            // 等待下一次唤醒或兜底 tick；Lagged（缓冲溢出）视为一次唤醒，唤醒是幂等的。
            tokio::select! {
                result = wake.recv(), if wake_open => {
                    if let Err(tokio::sync::broadcast::error::RecvError::Closed) = result {
                        wake_open = false;
                    }
                }
                _ = fallback.tick() => {}
            }
        }
    });

    Ok(
        Sse::new(futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)))
            .keep_alive(axum::response::sse::KeepAlive::default()),
    )
}
