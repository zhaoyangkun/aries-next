//! 服务器系统指标采集：主机 CPU / 内存 / 磁盘 / 网络 / 负载 / 进程数，
//! 外加 aries-server 进程自身（RSS）与 PostgreSQL 侧运维指标（库大小、活跃连接数）。
//!
//! 采集基于 `sysinfo` 用户态采样，无需特殊权限；CPU 使用率与网络速率依赖
//! 相邻两次 refresh 的差值，由 `MetricsCollector` 统一持有采样状态，保证
//! GET 快照与 SSE 推送共用同一套差值语义。历史趋势由 `MetricsHistory`
//! 内存环形缓冲承载（10s 采样、保留 24h），重启即清空，属可丢弃数据。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use sysinfo::{
    CpuRefreshKind, Disks, MemoryRefreshKind, Networks, Pid, ProcessRefreshKind, RefreshKind,
    System,
};
use time::OffsetDateTime;

/// PostgreSQL 侧运维指标快照；查询失败时不阻塞主机指标，整段置空。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PgSnapshot {
    /// 当前数据库磁盘占用（`pg_database_size`）。
    pub database_size_bytes: i64,
    /// 当前数据库的活跃连接数（`pg_stat_activity`）。
    pub active_connections: i64,
}

/// 单个磁盘挂载点指标。
#[derive(Debug, Clone, PartialEq)]
pub struct DiskSnapshot {
    /// 挂载路径（Windows 为盘符根路径）。
    pub mount_point: String,
    /// 设备名。
    pub name: String,
    /// 文件系统类型（NTFS / ext4 / overlay …）。
    pub file_system: String,
    /// 分区总容量。
    pub total_bytes: u64,
    /// 已用容量（total - available，与 df 口径一致）。
    pub used_bytes: u64,
}

/// 系统指标单次快照（纯数据；响应 DTO 与 SSE 事件在 `http::system` 定义）。
#[derive(Debug, Clone, PartialEq)]
pub struct SystemSnapshot {
    /// 采样时刻（UTC）。
    pub ts: OffsetDateTime,
    /// 全局 CPU 使用率（0–100）。
    pub cpu_percent: f32,
    /// 每核使用率（顺序与系统枚举一致），供前端画核占用格。
    pub per_core_percent: Vec<f32>,
    /// 已用内存（不含 Page Cache 等可回收部分，与 free 口径一致）。
    pub memory_used_bytes: u64,
    /// 物理内存总量。
    pub memory_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    /// 各挂载点（容器内通常为 overlay 单挂载）。
    pub disks: Vec<DiskSnapshot>,
    /// 全部网卡合计收发速率（B/s，相邻采样差值 / 间隔）。
    pub network_rx_bytes_per_sec: u64,
    pub network_tx_bytes_per_sec: u64,
    /// 全部网卡累计收发字节数（进程启动至今的计数器）。
    pub network_total_received_bytes: u64,
    pub network_total_transmitted_bytes: u64,
    /// 系统负载均值（Windows 恒为 0，无 loadavg 概念）。
    pub load_avg_one: f64,
    pub load_avg_five: f64,
    pub load_avg_fifteen: f64,
    /// 系统进程总数。
    pub process_count: usize,
    /// 主机已运行秒数（系统级 uptime）。
    pub uptime_seconds: u64,
    pub host_name: Option<String>,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
    /// aries-server 进程自身常驻内存（RSS）。
    pub server_rss_bytes: u64,
    /// aries-server 进程已运行秒数（进程级 uptime）。
    pub server_uptime_seconds: u64,
    pub pg: Option<PgSnapshot>,
}

/// 系统指标采集器：持有 `sysinfo` 刷新状态与 PostgreSQL 快照缓存。
///
/// `System` / `Networks` 的 refresh 都是 `&mut self`，并发采集用 Mutex 串行化；
/// 采集本身在内存中完成，临界区极短，GET 与 SSE 互不争用。
pub struct MetricsCollector {
    system: Mutex<System>,
    networks: Mutex<Networks>,
    disks: Disks,
    server_pid: Pid,
    server_started_at: Instant,
    pg_pool: PgPool,
    /// PostgreSQL 快照缓存：库大小/连接数变化缓慢，30s TTL 足够且避免
    /// SSE 每 5s 一轮的两条标量查询刷进慢查询日志。
    pg_cache: Mutex<Option<(Instant, PgSnapshot)>>,
}

/// PostgreSQL 快照缓存有效期。
const PG_CACHE_TTL: Duration = Duration::from_secs(30);

impl MetricsCollector {
    /// 新建采集器并做一次预热 refresh：sysinfo 的 CPU 使用率与网络速率基于
    /// 相邻采样差值，首次立即读取会得到 0；预热后首个真实采集窗口才有意义。
    pub fn new(pg_pool: PgPool) -> Self {
        let kind = RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything())
            .with_processes(ProcessRefreshKind::nothing());
        let mut system =
            System::new_with_specifics(kind).expect("sysinfo system initialization failed");
        system.refresh_cpu_usage();
        let networks =
            Networks::new_with_refreshed_list().expect("sysinfo networks initialization failed");
        Self {
            system: Mutex::new(system),
            networks: Mutex::new(networks),
            disks: Disks::new_with_refreshed_list().expect("sysinfo disks initialization failed"),
            server_pid: Pid::from_u32(std::process::id()),
            server_started_at: Instant::now(),
            pg_pool,
            pg_cache: Mutex::new(None),
        }
    }

    /// 采集一次完整快照：refresh CPU / 内存 / 网络（差值语义），磁盘与
    /// 系统静态信息低频变化，随进程启动时刷新即可。
    pub async fn collect(&self) -> SystemSnapshot {
        let (cpu_percent, per_core_percent, memory, swap, process_count, server_rss_bytes) = {
            let mut system = self.system.lock().unwrap_or_else(|e| e.into_inner());
            system.refresh_cpu_usage();
            system.refresh_memory();
            // 只刷新本进程内存：ProcessRefreshKind::nothing() 下进程对象的
            // memory 字段不会被填充，RSS 恒为 0。
            system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::Some(&[self.server_pid]),
                false,
                ProcessRefreshKind::nothing().with_memory(),
            );
            let cpu_percent = system.global_cpu_usage();
            let per_core_percent = system.cpus().iter().map(|cpu| cpu.usage()).collect();
            let server_rss_bytes = system
                .process(self.server_pid)
                .map(|process| process.memory())
                .unwrap_or(0);
            (
                cpu_percent,
                per_core_percent,
                (system.used_memory(), system.total_memory()),
                (system.used_swap(), system.total_swap()),
                system.processes().len(),
                server_rss_bytes,
            )
        };
        let (network_rx, network_tx, network_total_rx, network_total_tx) = {
            let mut networks = self.networks.lock().unwrap_or_else(|e| e.into_inner());
            networks.refresh(true);
            let mut rx = 0_u64;
            let mut tx = 0_u64;
            let mut total_rx = 0_u64;
            let mut total_tx = 0_u64;
            for network in networks.values() {
                rx = rx.saturating_add(network.received());
                tx = tx.saturating_add(network.transmitted());
                total_rx = total_rx.saturating_add(network.total_received());
                total_tx = total_tx.saturating_add(network.total_transmitted());
            }
            (rx, tx, total_rx, total_tx)
        };
        let load = System::load_average().unwrap_or(sysinfo::LoadAvg {
            one: 0.0,
            five: 0.0,
            fifteen: 0.0,
        });

        SystemSnapshot {
            ts: OffsetDateTime::now_utc(),
            cpu_percent,
            per_core_percent,
            memory_used_bytes: memory.0,
            memory_total_bytes: memory.1,
            swap_used_bytes: swap.0,
            swap_total_bytes: swap.1,
            disks: self
                .disks
                .iter()
                .map(|disk| DiskSnapshot {
                    mount_point: disk.mount_point().to_string_lossy().into_owned(),
                    name: disk.name().to_string_lossy().into_owned(),
                    file_system: disk.file_system().to_string_lossy().into_owned(),
                    total_bytes: disk.total_space(),
                    used_bytes: disk.total_space().saturating_sub(disk.available_space()),
                })
                .collect(),
            network_rx_bytes_per_sec: network_rx,
            network_tx_bytes_per_sec: network_tx,
            network_total_received_bytes: network_total_rx,
            network_total_transmitted_bytes: network_total_tx,
            load_avg_one: load.one,
            load_avg_five: load.five,
            load_avg_fifteen: load.fifteen,
            process_count,
            uptime_seconds: System::uptime().unwrap_or(0),
            host_name: System::host_name().ok(),
            os_version: System::long_os_version().ok(),
            kernel_version: System::kernel_version().ok(),
            server_rss_bytes,
            server_uptime_seconds: self.server_started_at.elapsed().as_secs(),
            pg: self.pg_snapshot().await,
        }
    }

    /// PostgreSQL 快照（30s 缓存）：库大小与活跃连接数；查询失败记 WARN
    /// 并清空缓存（下次重试），不影响主机指标的返回。
    async fn pg_snapshot(&self) -> Option<PgSnapshot> {
        if let Some((fetched_at, snapshot)) = self.pg_cache.lock().unwrap().as_ref() {
            if fetched_at.elapsed() < PG_CACHE_TTL {
                return Some(*snapshot);
            }
        }
        let size = sqlx::query_scalar::<_, i64>("SELECT pg_database_size(current_database())")
            .fetch_one(&self.pg_pool)
            .await;
        let connections = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database()",
        )
        .fetch_one(&self.pg_pool)
        .await;
        match (size, connections) {
            (Ok(database_size_bytes), Ok(active_connections)) => {
                let snapshot = PgSnapshot {
                    database_size_bytes,
                    active_connections,
                };
                *self.pg_cache.lock().unwrap() = Some((Instant::now(), snapshot));
                Some(snapshot)
            }
            (size_result, conn_result) => {
                if let Err(error) = size_result {
                    tracing::warn!(error = %error, "failed to collect postgres database size");
                }
                if let Err(error) = conn_result {
                    tracing::warn!(error = %error, "failed to collect postgres connections");
                }
                self.pg_cache.lock().unwrap().take();
                None
            }
        }
    }
}

/// 历史采样点（环形缓冲元素）：只保留趋势图所需的精简字段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricsSamplePoint {
    /// 采样时刻（UTC）。
    pub ts: OffsetDateTime,
    /// 全局 CPU 使用率（0–100）。
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    /// 全部挂载点合计已用（求 total - available）。
    pub disk_used_bytes: u64,
    pub network_rx_bytes_per_sec: u64,
    pub network_tx_bytes_per_sec: u64,
}

/// 采样周期：10s。
pub const HISTORY_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
/// 环形缓冲容量：10s × 8640 = 24h。
const HISTORY_CAPACITY: usize = 24 * 60 * 60 / HISTORY_SAMPLE_INTERVAL.as_secs() as usize;
/// 历史查询单次返回的桶数上限：前端按桶画折线，>360 个点收益递减。
const HISTORY_MAX_POINTS: usize = 360;

/// 历史趋势环形缓冲（内存态，重启即清空，属可丢弃数据）。
pub struct MetricsHistory {
    points: Mutex<VecDeque<MetricsSamplePoint>>,
}

impl Default for MetricsHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsHistory {
    pub fn new() -> Self {
        Self {
            points: Mutex::new(VecDeque::with_capacity(HISTORY_CAPACITY)),
        }
    }

    /// 追加一个采样点；缓冲满时丢弃最旧点。
    pub fn push(&self, point: MetricsSamplePoint) {
        let mut points = self.points.lock().unwrap_or_else(|e| e.into_inner());
        if points.len() >= HISTORY_CAPACITY {
            points.pop_front();
        }
        points.push_back(point);
    }

    /// 最近 `hours` 小时的采样点；超过 `HISTORY_MAX_POINTS` 时按时间均匀
    /// 分桶聚合为平均值（趋势图不需要逐点精度）。
    pub fn query(&self, hours: u32) -> Vec<MetricsSamplePoint> {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::hours(i64::from(hours));
        let points: Vec<MetricsSamplePoint> = self
            .points
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .copied()
            .filter(|point| point.ts >= cutoff)
            .collect();
        if points.len() <= HISTORY_MAX_POINTS {
            return points;
        }
        let bucket_count = HISTORY_MAX_POINTS.min(points.len());
        let bucket_size = points.len() as f64 / bucket_count as f64;
        let mut buckets = Vec::with_capacity(bucket_count);
        for index in 0..bucket_count {
            let start = (index as f64 * bucket_size) as usize;
            let end = (((index + 1) as f64) * bucket_size) as usize;
            let slice = &points[start..end.max(start + 1).min(points.len())];
            let Some(first) = slice.first() else {
                continue;
            };
            let len = slice.len() as f64;
            buckets.push(MetricsSamplePoint {
                ts: first.ts,
                cpu_percent: slice.iter().map(|p| p.cpu_percent).sum::<f32>() / len as f32,
                memory_used_bytes: avg_u64(slice.iter().map(|p| p.memory_used_bytes)),
                memory_total_bytes: first.memory_total_bytes,
                disk_used_bytes: avg_u64(slice.iter().map(|p| p.disk_used_bytes)),
                network_rx_bytes_per_sec: avg_u64(slice.iter().map(|p| p.network_rx_bytes_per_sec)),
                network_tx_bytes_per_sec: avg_u64(slice.iter().map(|p| p.network_tx_bytes_per_sec)),
            });
        }
        buckets
    }
}

/// u64 字段的桶内平均（先求和再除，避免每点精度损失；饱和加法兜底异常值）。
fn avg_u64(values: impl Iterator<Item = u64>) -> u64 {
    let (sum, count) = values.fold((0_u128, 0_u64), |(sum, count), value| {
        (sum + u128::from(value), count + 1)
    });
    if count == 0 {
        return 0;
    }
    (sum / u128::from(count)).min(u128::from(u64::MAX)) as u64
}

/// 快照 → 历史采样点（历史只保留趋势所需字段）。
pub fn sample_point(snapshot: &SystemSnapshot) -> MetricsSamplePoint {
    MetricsSamplePoint {
        ts: snapshot.ts,
        cpu_percent: snapshot.cpu_percent,
        memory_used_bytes: snapshot.memory_used_bytes,
        memory_total_bytes: snapshot.memory_total_bytes,
        disk_used_bytes: snapshot
            .disks
            .iter()
            .map(|disk| disk.used_bytes)
            .fold(0_u64, u64::saturating_add),
        network_rx_bytes_per_sec: snapshot.network_rx_bytes_per_sec,
        network_tx_bytes_per_sec: snapshot.network_tx_bytes_per_sec,
    }
}

/// 启动历史采样任务：每 `HISTORY_SAMPLE_INTERVAL` 采集一次入环形缓冲。
/// 只在 `main.rs` 启动（与 worker 同等地位）；集成测试不启动，按需直接 push。
/// 采样循环无排空需求，停机时与 worker 一样直接 abort。
pub fn spawn_history_sampler(state: crate::state::AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(HISTORY_SAMPLE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let snapshot = state.metrics.collect().await;
            state.metrics_history.push(sample_point(&snapshot));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 无数据库参与的路径：仅验证历史缓冲的容量、过滤与聚合行为。
    #[test]
    fn history_keeps_capacity_and_drops_oldest() {
        let history = MetricsHistory::new();
        let base = OffsetDateTime::now_utc();
        for index in 0..(HISTORY_CAPACITY + 10) {
            history.push(MetricsSamplePoint {
                ts: base + time::Duration::seconds(index as i64),
                cpu_percent: 1.0,
                memory_used_bytes: 100,
                memory_total_bytes: 200,
                disk_used_bytes: 50,
                network_rx_bytes_per_sec: 7,
                network_tx_bytes_per_sec: 9,
            });
        }
        assert_eq!(history.points.lock().unwrap().len(), HISTORY_CAPACITY);
        // 满容量查询按 HISTORY_MAX_POINTS 均匀聚合：360 桶，最早桶取首个采样点的时间
        //（最旧 10 个点已被丢弃，最早保留点序号 = 10）。
        let points = history.query(24);
        assert_eq!(points.len(), HISTORY_MAX_POINTS);
        assert_eq!(
            points.first().unwrap().ts,
            base + time::Duration::seconds(10)
        );
        assert!(
            points.windows(2).all(|w| w[0].ts <= w[1].ts),
            "aggregated buckets stay time-ordered"
        );
    }

    #[test]
    fn history_query_filters_by_hours_and_aggregates() {
        let history = MetricsHistory::new();
        let base = OffsetDateTime::now_utc() - time::Duration::hours(2);
        // 2 小时前起每分钟一个点（121 点），查询最近 1 小时应只剩 ~60 点，无需聚合。
        for index in 0..=120 {
            history.push(MetricsSamplePoint {
                ts: base + time::Duration::minutes(index),
                cpu_percent: f32::from(index as u16 % 100),
                memory_used_bytes: 1_000,
                memory_total_bytes: 2_000,
                disk_used_bytes: 500,
                network_rx_bytes_per_sec: 100,
                network_tx_bytes_per_sec: 200,
            });
        }
        let hour = history.query(1);
        let all = history.query(24);
        assert!(
            hour.len() < all.len(),
            "1h window must filter out older points"
        );
        assert!(
            hour.len() <= 60,
            "1h at 1/min must be ≤ 60 points: {}",
            hour.len()
        );
        assert!(
            hour.windows(2).all(|w| w[0].ts <= w[1].ts),
            "points stay time-ordered"
        );

        // 恒值序列聚合后仍等于原值：单独用只含恒值 50 的缓冲验证
        // （避免与上面的递增序列混合，干扰第一桶的平均断言）。
        let constant = MetricsHistory::new();
        for index in 0..(HISTORY_MAX_POINTS as i64 * 2) {
            constant.push(MetricsSamplePoint {
                ts: base + time::Duration::minutes(index),
                cpu_percent: 50.0,
                memory_used_bytes: 1_000,
                memory_total_bytes: 2_000,
                disk_used_bytes: 500,
                network_rx_bytes_per_sec: 100,
                network_tx_bytes_per_sec: 200,
            });
        }
        let aggregated = constant.query(24);
        assert_eq!(aggregated.len(), HISTORY_MAX_POINTS);
        assert_eq!(aggregated[0].cpu_percent, 50.0);
        assert_eq!(aggregated[0].memory_used_bytes, 1_000);
        assert_eq!(aggregated[0].network_tx_bytes_per_sec, 200);
    }
}
