//! Admin 系统监控端点：主机与进程指标快照、SSE 实时推送、历史趋势查询（仅 Owner / ManageSettings）。
//! 采集逻辑在 `crate::metrics`（`sysinfo` 用户态采样 + PostgreSQL 快照缓存），本模块只做 DTO 转换与协议。

use std::sync::Arc;

use aries_core::auth::Permission;
use axum::{
    Json, Router,
    extract::{Query, State},
    response::Sse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;

use crate::metrics::{
    MetricsCollector, MetricsSamplePoint, NetworkInterfaceSnapshot, ProcessSnapshot, SystemSnapshot,
};
use crate::state::AppState;

use super::auth::CurrentUser;
use super::error::ApiError;

/// 注册系统监控路由（挂载在 `/api/admin` 下）。
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/system/metrics", get(get_metrics))
        .route("/system/metrics/stream", get(stream_metrics))
        .route("/system/metrics/history", get(get_metrics_history))
        .route("/system/processes", get(get_top_processes))
}

/// SSE 推送周期：5 秒一帧，与 sysinfo 的 CPU 差值采样窗口匹配。
const STREAM_TICK: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "SystemMetricsResponse",
    description = "系统指标快照：主机 CPU / 内存 / 磁盘 / 网络 / 负载 / 进程数，加上 aries-server 进程自身与 PostgreSQL 侧指标。"
)]
struct SystemMetricsResponse {
    /// 采样时刻，RFC 3339。
    ts: String,
    cpu: CpuMetrics,
    memory: MemoryMetrics,
    swap: SwapMetrics,
    /// 各磁盘挂载点。
    disks: Vec<DiskMetrics>,
    network: NetworkMetrics,
    /// 系统负载均值（Windows 恒为 0）。
    load: LoadMetrics,
    /// 系统进程总数。
    process_count: u64,
    /// 主机已运行秒数。
    uptime_seconds: u64,
    host_name: Option<String>,
    os_version: Option<String>,
    kernel_version: Option<String>,
    /// aries-server 进程自身指标。
    server: ServerMetrics,
    /// PostgreSQL 侧指标（查询失败时为 null，不影响其他字段）。
    pg: Option<PgMetrics>,
    /// 温度传感器读数（无传感器环境为空数组）。
    sensors: Vec<TempSensor>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "TempSensor", description = "温度传感器读数。")]
struct TempSensor {
    /// 传感器标签（如 `coretemp Package id 0`）。
    label: String,
    /// 摄氏温度。
    temperature_celsius: f32,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "CpuMetrics", description = "CPU 使用率（0–100）与型号信息。")]
struct CpuMetrics {
    /// 全局使用率。
    percent: f32,
    /// 每核使用率（顺序与系统枚举一致）。
    per_core: Vec<f32>,
    /// CPU 型号名（拿不到为 null）。
    brand: Option<String>,
    /// 当前频率（各核 MHz 平均值；拿不到为 null）。
    frequency_mhz: Option<u64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "MemoryMetrics", description = "物理内存占用。")]
struct MemoryMetrics {
    used_bytes: u64,
    /// 可用内存（不触发交换的前提下可立即分配的量）。
    available_bytes: u64,
    total_bytes: u64,
    /// 使用率（0–100，由服务端算好，前端直接展示）。
    percent: f32,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "SwapMetrics",
    description = "Swap 占用（未配置 Swap 时 total 为 0）。"
)]
struct SwapMetrics {
    used_bytes: u64,
    total_bytes: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "DiskMetrics", description = "磁盘挂载点占用。")]
struct DiskMetrics {
    /// 挂载路径（Windows 为盘符根路径）。
    mount_point: String,
    name: String,
    file_system: String,
    total_bytes: u64,
    used_bytes: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "NetworkMetrics", description = "全部网卡合计网络 IO。")]
struct NetworkMetrics {
    /// 接收速率（B/s，相邻采样差值 / 间隔）。
    rx_bytes_per_sec: u64,
    /// 发送速率（B/s）。
    tx_bytes_per_sec: u64,
    /// 累计接收字节数（计数器）。
    total_received_bytes: u64,
    /// 累计发送字节数（计数器）。
    total_transmitted_bytes: u64,
    /// 各网卡明细（名称 + 速率 + 累计），名称升序。
    interfaces: Vec<NetworkInterfaceMetrics>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "NetworkInterfaceMetrics",
    description = "单网卡网络 IO 明细。"
)]
struct NetworkInterfaceMetrics {
    name: String,
    rx_bytes_per_sec: u64,
    tx_bytes_per_sec: u64,
    total_received_bytes: u64,
    total_transmitted_bytes: u64,
}

impl From<&NetworkInterfaceSnapshot> for NetworkInterfaceMetrics {
    /// 网卡采集数据 → 响应 DTO。
    fn from(interface: &NetworkInterfaceSnapshot) -> Self {
        Self {
            name: interface.name.clone(),
            rx_bytes_per_sec: interface.rx_bytes_per_sec,
            tx_bytes_per_sec: interface.tx_bytes_per_sec,
            total_received_bytes: interface.total_received_bytes,
            total_transmitted_bytes: interface.total_transmitted_bytes,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "LoadMetrics",
    description = "系统负载均值（Windows 恒为 0，无 loadavg 概念）。"
)]
struct LoadMetrics {
    one: f64,
    five: f64,
    fifteen: f64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "ServerMetrics", description = "aries-server 进程自身指标。")]
struct ServerMetrics {
    /// 进程常驻内存（RSS）。
    rss_bytes: u64,
    /// 进程已运行秒数。
    uptime_seconds: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "PgMetrics", description = "PostgreSQL 侧运维指标。")]
struct PgMetrics {
    /// 当前数据库磁盘占用（`pg_database_size`）。
    database_size_bytes: i64,
    /// 当前数据库活跃连接数（`pg_stat_activity`）。
    active_connections: i64,
}

/// 快照 → 响应 DTO（RFC 3339 时间格式化与百分比预计算在此完成）。
fn snapshot_response(snapshot: SystemSnapshot) -> SystemMetricsResponse {
    let memory_percent = percent(snapshot.memory_used_bytes, snapshot.memory_total_bytes);
    SystemMetricsResponse {
        ts: snapshot
            .ts
            .format(&Rfc3339)
            .unwrap_or_else(|_| String::new()),
        cpu: CpuMetrics {
            percent: snapshot.cpu_percent,
            per_core: snapshot.per_core_percent,
            brand: snapshot.cpu_brand,
            frequency_mhz: snapshot.cpu_frequency_mhz,
        },
        memory: MemoryMetrics {
            used_bytes: snapshot.memory_used_bytes,
            available_bytes: snapshot.memory_available_bytes,
            total_bytes: snapshot.memory_total_bytes,
            percent: memory_percent,
        },
        swap: SwapMetrics {
            used_bytes: snapshot.swap_used_bytes,
            total_bytes: snapshot.swap_total_bytes,
        },
        disks: snapshot
            .disks
            .iter()
            .map(|disk| DiskMetrics {
                mount_point: disk.mount_point.clone(),
                name: disk.name.clone(),
                file_system: disk.file_system.clone(),
                total_bytes: disk.total_bytes,
                used_bytes: disk.used_bytes,
            })
            .collect(),
        network: NetworkMetrics {
            rx_bytes_per_sec: snapshot.network_rx_bytes_per_sec,
            tx_bytes_per_sec: snapshot.network_tx_bytes_per_sec,
            total_received_bytes: snapshot.network_total_received_bytes,
            total_transmitted_bytes: snapshot.network_total_transmitted_bytes,
            interfaces: snapshot.network_interfaces.iter().map(Into::into).collect(),
        },
        load: LoadMetrics {
            one: snapshot.load_avg_one,
            five: snapshot.load_avg_five,
            fifteen: snapshot.load_avg_fifteen,
        },
        process_count: snapshot.process_count as u64,
        uptime_seconds: snapshot.uptime_seconds,
        host_name: snapshot.host_name,
        os_version: snapshot.os_version,
        kernel_version: snapshot.kernel_version,
        server: ServerMetrics {
            rss_bytes: snapshot.server_rss_bytes,
            uptime_seconds: snapshot.server_uptime_seconds,
        },
        pg: snapshot.pg.map(|pg| PgMetrics {
            database_size_bytes: pg.database_size_bytes,
            active_connections: pg.active_connections,
        }),
        sensors: snapshot
            .sensors
            .iter()
            .map(|sensor| TempSensor {
                label: sensor.label.clone(),
                temperature_celsius: sensor.celsius,
            })
            .collect(),
    }
}

/// used / total → 百分比（0–100）；total 为 0（如未配置 Swap）返回 0。
fn percent(used: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    (used as f64 / total as f64 * 100.0) as f32
}

/// 当前系统指标快照（主机 + 进程自身 + PostgreSQL）；需要 `settings:manage` 权限。
#[utoipa::path(
    get,
    path = "/api/admin/system/metrics",
    tag = "Admin System",
    operation_id = "getSystemMetrics",
    summary = "系统指标快照",
    description = "采集一次完整系统指标：主机 CPU（全局/每核）、内存、Swap、磁盘各挂载点、网络速率与累计、负载、进程数、uptime，以及 aries-server 进程 RSS / 运行时长和 PostgreSQL 库大小 / 活跃连接数。PostgreSQL 指标带 30 秒缓存，查询失败时该段为 null。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "系统指标快照", body = SystemMetricsResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_metrics(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<SystemMetricsResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let snapshot = state.metrics.collect().await;
    Ok(Json(snapshot_response(snapshot)))
}

/// 实时系统指标 SSE：每 5 秒推送一帧完整快照（事件名 `metrics`，形状同快照接口）。
#[utoipa::path(
    get,
    path = "/api/admin/system/metrics/stream",
    tag = "Admin System",
    operation_id = "streamSystemMetrics",
    summary = "实时推送系统指标（SSE）",
    description = "以 5 秒为周期采集并推送完整系统指标快照（形状同 `SystemMetricsResponse`）；`Sse::keep_alive` 每 15 秒补发注释帧，防止反向代理空闲断连。客户端断开即结束采集循环。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "SSE 流（持续响应，客户端断开即结束）", content_type = "text/event-stream", body = SystemMetricsResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn stream_metrics(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<
    Sse<
        impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    ApiError,
> {
    use axum::response::sse::{Event, KeepAlive};

    current.require(Permission::ManageSettings)?;

    let collector: Arc<MetricsCollector> = state.metrics.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(8);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(STREAM_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let snapshot = collector.collect().await;
            let Ok(data) = serde_json::to_string(&snapshot_response(snapshot)) else {
                break;
            };
            if tx
                .send(Ok(Event::default().event("metrics").data(data)))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    Ok(
        Sse::new(futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)))
            .keep_alive(KeepAlive::default()),
    )
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub(crate) struct MetricsHistoryParams {
    /// 查询窗口（小时）：默认 1，范围 1–24（越界钳位）；采样 10 秒一点，超 360 点时按时间均匀聚合为平均值。
    hours: Option<u32>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "SystemMetricsHistoryResponse",
    description = "历史趋势：内存环形缓冲的采样点序列（10s 采样、保留 24h、重启清空）。"
)]
struct SystemMetricsHistoryResponse {
    hours: u32,
    /// 相邻数据点的名义间隔秒数（聚合后为桶宽）。
    interval_seconds: u64,
    buckets: Vec<HistoryBucket>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "HistoryBucket", description = "单个历史数据点（或聚合桶）。")]
struct HistoryBucket {
    /// 桶起点，RFC 3339。
    bucket: String,
    /// 全局 CPU 使用率（0–100；聚合桶为平均值）。
    cpu_percent: f32,
    memory_used_bytes: u64,
    memory_total_bytes: u64,
    /// 全部挂载点合计已用。
    disk_used_bytes: u64,
    network_rx_bytes_per_sec: u64,
    network_tx_bytes_per_sec: u64,
}

fn history_bucket(point: MetricsSamplePoint) -> HistoryBucket {
    HistoryBucket {
        bucket: point.ts.format(&Rfc3339).unwrap_or_else(|_| String::new()),
        cpu_percent: point.cpu_percent,
        memory_used_bytes: point.memory_used_bytes,
        memory_total_bytes: point.memory_total_bytes,
        disk_used_bytes: point.disk_used_bytes,
        network_rx_bytes_per_sec: point.network_rx_bytes_per_sec,
        network_tx_bytes_per_sec: point.network_tx_bytes_per_sec,
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub(crate) struct TopProcessesParams {
    /// 返回条数：默认 10，范围 1–50（越界钳位）。
    limit: Option<u32>,
    /// 排序维度：`cpu`（默认，按 CPU 降序）或 `memory`（按 RSS 降序）；非法值 400。
    sort: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "TopProcessesResponse",
    description = "资源占用 Top 进程列表（按 CPU 降序；15 秒服务端缓存）。"
)]
struct TopProcessesResponse {
    /// 本次采样时刻，RFC 3339。
    ts: String,
    /// Top 进程列表。
    processes: Vec<ProcessMetrics>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "ProcessMetrics", description = "单进程资源占用。")]
struct ProcessMetrics {
    pid: u32,
    name: String,
    /// CPU 使用率（0–100，单核口径：100 = 打满一个核）。
    cpu_percent: f32,
    /// 常驻内存（RSS）。
    memory_bytes: u64,
}

impl From<ProcessSnapshot> for ProcessMetrics {
    /// 进程采集数据 → 响应 DTO。
    fn from(process: ProcessSnapshot) -> Self {
        Self {
            pid: process.pid,
            name: process.name,
            cpu_percent: process.cpu_percent,
            memory_bytes: process.memory_bytes,
        }
    }
}

/// 资源占用 Top 进程：按 CPU 降序，服务端 15 秒缓存（全量进程表刷新开销较大，
/// 独立于 5 秒快照循环）；需要 `settings:manage` 权限。
#[utoipa::path(
    get,
    path = "/api/admin/system/processes",
    tag = "Admin System",
    operation_id = "getTopProcesses",
    summary = "资源占用 Top 进程",
    description = "按 CPU 使用率降序返回进程列表（单核口径：100 = 打满一个核），附 RSS；服务端 15 秒缓存。首次调用（或缓存过期后首次）所有进程 CPU 为 0（sysinfo 差值语义），约 15 秒后第二次调用即准确。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    params(TopProcessesParams),
    responses(
        (status = 200, description = "Top 进程列表", body = TopProcessesResponse),
        (status = 400, description = "sort 非法（INVALID_PROCESS_SORT）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_top_processes(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<TopProcessesParams>,
) -> Result<Json<TopProcessesResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let limit = params.limit.unwrap_or(10).clamp(1, 50) as usize;
    let sort = match params.sort.as_deref() {
        None | Some("cpu") => crate::metrics::ProcessSort::Cpu,
        Some("memory") => crate::metrics::ProcessSort::Memory,
        Some(_) => {
            return Err(ApiError::bad_request(
                "INVALID_PROCESS_SORT",
                "sort must be 'cpu' or 'memory'",
            ));
        }
    };
    let (ts, processes) = state.metrics.top_processes(sort, limit).await;
    Ok(Json(TopProcessesResponse {
        ts: ts.format(&Rfc3339).unwrap_or_else(|_| String::new()),
        processes: processes.into_iter().map(Into::into).collect(),
    }))
}

/// 历史趋势：最近 N 小时的采样点序列（10s 采样、内存环形缓冲、重启清空）；需要 `settings:manage` 权限。
#[utoipa::path(
    get,
    path = "/api/admin/system/metrics/history",
    tag = "Admin System",
    operation_id = "getSystemMetricsHistory",
    summary = "系统指标历史趋势",
    description = "读取内存环形缓冲中的历史采样点（10 秒采样、保留 24 小时、服务重启即清空，属可丢弃数据）。窗口默认 1 小时、上限 24 小时；窗口内超过 360 点时按时间均匀聚合为平均值。仅 Owner（`ManageSettings`）可访问。",
    security(("cookieAuth" = [])),
    params(MetricsHistoryParams),
    responses(
        (status = 200, description = "历史采样点序列（时间升序）", body = SystemMetricsHistoryResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_metrics_history(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<MetricsHistoryParams>,
) -> Result<Json<SystemMetricsHistoryResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let hours = params.hours.unwrap_or(1).clamp(1, 24);
    let points = state.metrics_history.query(hours);
    // 名义间隔：聚合时按点数反推桶宽，前端据此标注 x 轴。
    let interval_seconds = match points.len() {
        0 | 1 => crate::metrics::HISTORY_SAMPLE_INTERVAL.as_secs(),
        len => {
            let first = points.first().map(|p| p.ts);
            let last = points.last().map(|p| p.ts);
            match (first, last) {
                (Some(first), Some(last)) if last > first => {
                    ((last - first).whole_seconds() as usize / (len - 1)).max(1) as u64
                }
                _ => crate::metrics::HISTORY_SAMPLE_INTERVAL.as_secs(),
            }
        }
    };
    Ok(Json(SystemMetricsHistoryResponse {
        hours,
        interval_seconds,
        buckets: points.into_iter().map(history_bucket).collect(),
    }))
}
