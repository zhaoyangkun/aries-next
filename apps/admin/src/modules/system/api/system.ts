import { api } from '@/shared/api/client'

export interface CpuMetrics {
  /** 全局使用率（0–100）。 */
  percent: number
  /** 每核使用率。 */
  per_core: number[]
}

export interface MemoryMetrics {
  used_bytes: number
  total_bytes: number
  /** 服务端预计算的使用率（0–100）。 */
  percent: number
}

export interface SwapMetrics {
  used_bytes: number
  total_bytes: number
}

export interface DiskMetrics {
  mount_point: string
  name: string
  file_system: string
  total_bytes: number
  used_bytes: number
}

export interface NetworkMetrics {
  /** 接收速率（B/s）。 */
  rx_bytes_per_sec: number
  /** 发送速率（B/s）。 */
  tx_bytes_per_sec: number
  total_received_bytes: number
  total_transmitted_bytes: number
  /** 各网卡明细（名称升序）。 */
  interfaces: NetworkInterfaceMetrics[]
}

export interface NetworkInterfaceMetrics {
  name: string
  rx_bytes_per_sec: number
  tx_bytes_per_sec: number
  total_received_bytes: number
  total_transmitted_bytes: number
}

export interface ProcessMetrics {
  pid: number
  name: string
  /** CPU 使用率（0–100，单核口径：100 = 打满一个核）。 */
  cpu_percent: number
  /** 常驻内存（RSS）。 */
  memory_bytes: number
}

export interface TopProcessesResponse {
  /** 采样时刻，RFC 3339。 */
  ts: string
  /** 按 CPU 降序。 */
  processes: ProcessMetrics[]
}

export interface LoadMetrics {
  one: number
  five: number
  fifteen: number
}

export interface ServerMetrics {
  /** 进程常驻内存（RSS）。 */
  rss_bytes: number
  uptime_seconds: number
}

export interface PgMetrics {
  database_size_bytes: number
  active_connections: number
}

/** 系统指标快照（SSE 推送的 data 形状与本结构一致）。 */
export interface SystemMetrics {
  /** 采样时刻，RFC 3339。 */
  ts: string
  cpu: CpuMetrics
  memory: MemoryMetrics
  swap: SwapMetrics
  disks: DiskMetrics[]
  network: NetworkMetrics
  load: LoadMetrics
  process_count: number
  uptime_seconds: number
  host_name: string | null
  os_version: string | null
  kernel_version: string | null
  server: ServerMetrics
  /** PostgreSQL 指标（查询失败时为 null）。 */
  pg: PgMetrics | null
}

export interface HistoryBucket {
  /** 桶起点，RFC 3339。 */
  bucket: string
  cpu_percent: number
  memory_used_bytes: number
  memory_total_bytes: number
  disk_used_bytes: number
  network_rx_bytes_per_sec: number
  network_tx_bytes_per_sec: number
}

export interface SystemMetricsHistory {
  hours: number
  /** 相邻数据点名义间隔秒数（聚合后为桶宽）。 */
  interval_seconds: number
  buckets: HistoryBucket[]
}

export const systemApi = {
  /** 当前系统指标快照：主机 + 进程自身 + PostgreSQL。 */
  async get(): Promise<SystemMetrics> {
    const { data } = await api.get<SystemMetrics>('/api/admin/system/metrics')
    return data
  },

  /** 历史趋势：hours 默认 1、上限 24（越界钳位）；返回时间升序采样点。 */
  async history(hours: number): Promise<SystemMetricsHistory> {
    const { data } = await api.get<SystemMetricsHistory>('/api/admin/system/metrics/history', {
      params: { hours },
    })
    return data
  },

  /** 实时推送 SSE URL：用原生 EventSource 消费（同域自动带 cookie），事件名 `metrics`。 */
  buildStreamUrl() {
    return '/api/admin/system/metrics/stream'
  },

  /**
   * 资源占用 Top 进程（按 CPU 降序）；服务端 15 秒缓存。
   * 首次调用所有进程 CPU 为 0（差值语义），约 15 秒后第二次调用即准确。
   */
  async processes(limit = 10): Promise<TopProcessesResponse> {
    const { data } = await api.get<TopProcessesResponse>('/api/admin/system/processes', {
      params: { limit },
    })
    return data
  },
}
