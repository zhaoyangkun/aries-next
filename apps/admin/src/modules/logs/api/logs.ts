import { api } from '@/shared/api/client'

export type LogLevel = 'ERROR' | 'WARN' | 'INFO' | 'DEBUG' | 'TRACE'

export interface LogEntry {
  id: number
  // RFC 3339 时间戳（UTC），展示时转本地时区。
  ts: string
  level: LogLevel
  target: string
  message: string
  span_name: string | null
  request_id: string | null
  fields: Record<string, unknown>
}

export interface LogLevelCounts {
  ERROR: number
  WARN: number
  INFO: number
  DEBUG: number
  TRACE: number
}

export interface LogPage {
  items: LogEntry[]
  total: number
  page: number
  page_size: number
  // keyset 下一页游标：满页时非空，末页为 null；asc（链路视图）与上下文模式恒为 null。
  next_cursor: string | null
  // 最近 24h 各级别条数，与当前筛选无关。
  level_counts: LogLevelCounts
}

// 趋势分桶：bucket 为 RFC 3339 起点；五个级别键恒在。
export interface LogStatsBucket {
  bucket: string
  ERROR: number
  WARN: number
  INFO: number
  DEBUG: number
  TRACE: number
}

// 运行时级别覆盖（tracing directives）状态；restore_seconds_remaining 为到自动复位的剩余秒数，null 表示不复位。
export interface LogFilterOverride {
  directives: string
  restore_seconds_remaining: number | null
}

/** level 语义为「最低级别」（选 WARN 返回 WARN+ERROR）；时间范围为 RFC 3339，可空。 */
export const logsApi = {
  async list(params: {
    page: number
    page_size: number
    level?: LogLevel
    target?: string
    keyword?: string
    start?: string
    end?: string
    // 请求链路追踪：精确匹配单个请求的日志；配合 order=asc 按链路流转正序展示。
    request_id?: string
    order?: 'asc' | 'desc'
    // keyset 游标：上一页响应的 next_cursor，仅 desc 有效，存在时忽略 page，与 asc 互斥。
    cursor?: string
    // 精确排除某 target（与 target 互斥），如隐藏 sqlx::query。
    exclude_target?: string
    // 上下文模式：以该行为锚点前后各 context 条，忽略分页与时间筛选，asc 返回；锚点不存在 404。
    around_id?: number
    context?: number
  }) {
    const { data } = await api.get<LogPage>('/api/admin/logs', { params })
    return data
  },

  // 日志来源模块列表，供筛选下拉。
  async targets() {
    const { data } = await api.get<{ items: string[] }>('/api/admin/logs/targets')
    return data
  },

  // 级别趋势分桶：≤48h 按小时、>48h 按天，升序，五个级别键恒在。
  async stats(hours = 24) {
    const { data } = await api.get<{
      hours: number
      buckets: LogStatsBucket[]
      // 运行日志 channel 满累计丢弃条数。
      channel_dropped: number
    }>('/api/admin/logs/stats', { params: { hours } })
    return data
  },

  // 运行时级别覆盖（tracing directives）：directives 为空串表示清除覆盖，restore_minutes 省略表示不复位。
  async getFilterOverride(): Promise<LogFilterOverride> {
    const { data } = await api.get<LogFilterOverride>('/api/admin/logs/filter')
    return data
  },

  async setFilterOverride(input: { directives: string, restore_minutes?: number }): Promise<LogFilterOverride> {
    const { data } = await api.put<LogFilterOverride>('/api/admin/logs/filter', input)
    return data
  },

  // 实时跟踪 SSE：返回完整 URL（含查询串），用原生 EventSource 消费（同域自动带 cookie）。
  buildTailUrl(params: {
    level?: LogLevel
    target?: string
    keyword?: string
    exclude_target?: string
    request_id?: string
  }) {
    const query = new URLSearchParams()
    for (const [key, value] of Object.entries(params)) {
      if (value)
        query.set(key, value)
    }
    const search = query.toString()
    return `/api/admin/logs/tail${search ? `?${search}` : ''}`
  },

  async getSqlLog() {
    const { data } = await api.get<{ enabled: boolean }>('/api/admin/logs/sql')
    return data
  },

  // SQL 日志运行时开关：输出 DEBUG 级 SQL 语句，量大时影响性能。
  async setSqlLog(enabled: boolean) {
    const { data } = await api.put<{ enabled: boolean }>('/api/admin/logs/sql', { enabled })
    return data
  },
}
