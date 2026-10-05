<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import dayjs from 'dayjs'
import {
  AlignHorizontalDistributeCenterIcon,
  CheckIcon,
  ChevronDownIcon,
  CopyIcon,
  DatabaseIcon,
  EyeOffIcon,
  RadioIcon,
  SearchIcon,
  SlidersHorizontalIcon,
  SquareTerminalIcon,
  WaypointsIcon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { AppDataTablePagination } from '@/shared/components/data-table'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import { BasicPage } from '@/components/global-layout'
import { logsApi, type LogEntry, type LogFilterOverride, type LogLevel, type LogLevelCounts, type LogStatsBucket } from '@/modules/logs/api/logs'
import { highlightJson, highlightSql } from '@/modules/logs/utils/highlight'
import { getApiError } from '@/shared/api/client'
import { toast } from 'vue-sonner'
import { toRfc3339 } from '@/utils/to-rfc3339'

const page = ref(1)
const pageSize = ref(20)
const level = ref<'' | LogLevel>('')
const target = ref('')
const keyword = ref('')
const start = ref('')
const end = ref('')
const logs = ref<LogEntry[]>([])
const total = ref(0)
const levelCounts = ref<LogLevelCounts | null>(null)
const targets = ref<string[]>([])
const loading = ref(true)
const error = ref('')

// 展开详情的行：message 过长截断，点击展开 span/request/fields。
const expandedId = ref<number | null>(null)

// 请求链路追踪视图：非空时按 request_id 精确过滤 + 正序展示链路流转。
const traceRequestId = ref('')

// 上下文视图：以锚点行前后各 N 条展示（服务端忽略分页与时间筛选）；与链路视图互斥。
const contextAnchor = ref<LogEntry | null>(null)

// 隐藏 SQL 日志开关：勾选后查询带 exclude_target=sqlx::query。
const hideSql = ref(false)

// 24h 级别趋势分桶（迷你柱状图）。
const statsBuckets = ref<LogStatsBucket[]>([])

// 运行日志 channel 满累计丢弃条数（stats 响应随趋势图一起返回）。
const channelDropped = ref(0)

// 实时跟踪（SSE）：开启后由服务端推送新日志，新条目前插列表顶部，与自动刷新互斥。
const liveTail = ref(false)
const tailError = ref('')
let tailSource: EventSource | undefined

// 实时跟踪模式下列表最多保留的条数，超出从尾部截掉。
const TAIL_MAX_ENTRIES = 500

// 自动刷新：每 5 秒静默拉取当前页（不切 loading 骨架，避免闪烁）。
const autoRefresh = ref(false)
let refreshTimer: ReturnType<typeof setInterval> | undefined

// SQL 日志运行时开关
const sqlLogEnabled = ref(false)
const sqlLogLoading = ref(true)
const sqlLogSaving = ref(false)
const sqlLogError = ref('')

// 运行时级别覆盖（tracing directives）控件：临时提升某 target 的日志级别，可设置自动复位。
const filterDirectives = ref('')
// ''=不复位，其余为自动复位分钟数。
const restoreMinutes = ref('')
const filterOverride = ref<LogFilterOverride | null>(null)
const filterSaving = ref(false)
const filterError = ref('')
const filterCountdown = ref<number | null>(null)
let countdownTimer: ReturnType<typeof setInterval> | undefined

const LEVELS: LogLevel[] = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE']

// 级别语义色：ERROR 红 / WARN 黄 / INFO 蓝 / DEBUG 灰 / TRACE 更浅。
const levelMeta: Record<LogLevel, { badgeClass: string, cardClass: string }> = {
  ERROR: {
    badgeClass: 'border-transparent bg-destructive/15 text-destructive',
    cardClass: 'border-destructive/40 bg-destructive/5 text-destructive',
  },
  WARN: {
    badgeClass: 'border-transparent bg-amber-500/15 text-amber-700 dark:text-amber-400',
    cardClass: 'border-amber-500/40 bg-amber-500/5 text-amber-700 dark:text-amber-400',
  },
  INFO: {
    badgeClass: 'border-transparent bg-blue-500/15 text-blue-700 dark:text-blue-400',
    cardClass: 'border-blue-500/40 bg-blue-500/5 text-blue-700 dark:text-blue-400',
  },
  DEBUG: {
    badgeClass: 'border-transparent bg-muted text-muted-foreground',
    cardClass: 'bg-muted/40 text-muted-foreground',
  },
  TRACE: {
    badgeClass: 'bg-transparent text-muted-foreground',
    cardClass: 'bg-muted/20 text-muted-foreground',
  },
}

async function loadLogs(silent = false) {
  if (!silent) {
    loading.value = true
    error.value = ''
  }
  try {
    // 上下文模式：只传 around_id，分页与时间筛选由服务端忽略，展示锚点前后的完整现场。
    const params = contextAnchor.value
      ? { page: 1, page_size: pageSize.value, around_id: contextAnchor.value.id }
      : {
          page: page.value,
          page_size: pageSize.value,
          level: level.value || undefined,
          target: target.value || undefined,
          keyword: keyword.value.trim() || undefined,
          start: toRfc3339(start.value),
          end: toRfc3339(end.value),
          request_id: traceRequestId.value || undefined,
          // 链路视图正序（时间升序体现流转），默认倒序（最新在前）。
          order: (traceRequestId.value ? 'asc' : 'desc') as 'asc' | 'desc',
          // 与 target 下拉选择 sqlx::query 互斥。
          exclude_target:
            hideSql.value && target.value !== 'sqlx::query' ? 'sqlx::query' : undefined,
        }
    const result = await logsApi.list(params)
    logs.value = result.items
    total.value = result.total
    levelCounts.value = result.level_counts
  }
  catch (e) {
    // 静默刷新失败不打断当前列表，只在手动操作时展示错误。
    if (!silent)
      error.value = getApiError(e, '加载运行日志失败')
  }
  finally {
    if (!silent)
      loading.value = false
  }
}

function applyFilter() {
  page.value = 1
  expandedId.value = null
  loadLogs()
  rebuildTail()
}

function resetFilter() {
  level.value = ''
  target.value = ''
  keyword.value = ''
  start.value = ''
  end.value = ''
  page.value = 1
  expandedId.value = null
  hideSql.value = false
  // 重置同时退出链路视图与上下文视图。
  traceRequestId.value = ''
  contextAnchor.value = null
  loadLogs()
  rebuildTail()
}

// 时间快捷选项：填充 start/end（datetime-local 本地时间格式）后立即查询。
function applyQuickRange(option: '15m' | '1h' | '24h' | 'today') {
  const now = dayjs()
  const from = option === 'today'
    ? now.startOf('day')
    : option === '15m'
      ? now.subtract(15, 'minute')
      : option === '1h'
        ? now.subtract(1, 'hour')
        : now.subtract(24, 'hour')
  start.value = from.format('YYYY-MM-DDTHH:mm')
  end.value = now.format('YYYY-MM-DDTHH:mm')
  applyFilter()
}

// 进入/退出请求链路视图：只切换 request_id 与排序，其余筛选（级别/关键词/时间）保留可用。
function enterTrace(requestId: string) {
  traceRequestId.value = requestId
  // 链路视图与上下文视图互斥。
  contextAnchor.value = null
  page.value = 1
  expandedId.value = null
  loadLogs()
  rebuildTail()
}

function exitTrace() {
  traceRequestId.value = ''
  page.value = 1
  loadLogs()
  rebuildTail()
}

// 进入/退出上下文视图：以点击行为锚点，服务端返回前后各 context 条（asc）。
function enterContext(entry: LogEntry) {
  contextAnchor.value = entry
  traceRequestId.value = ''
  page.value = 1
  expandedId.value = null
  loadLogs()
  rebuildTail()
}

function exitContext() {
  contextAnchor.value = null
  page.value = 1
  loadLogs()
  rebuildTail()
}

// 翻页与每页数量变化时重新请求；pageSize 变化且不在第一页时先回到第一页（由 page 变化触发加载）。
watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadLogs()
  rebuildTail()
})

async function loadTargets() {
  try {
    const result = await logsApi.targets()
    targets.value = result.items
  }
  catch {
    // target 下拉加载失败不阻塞主列表。
  }
}

async function loadStats() {
  try {
    const result = await logsApi.stats(24)
    statsBuckets.value = result.buckets
    channelDropped.value = result.channel_dropped
  }
  catch {
    // 趋势图加载失败不阻塞主列表。
  }
}

// 迷你柱状图：各分段高度按全局最大桶总量归一，跨桶可比。
const statsMax = computed(() =>
  Math.max(1, ...statsBuckets.value.map(bucket =>
    bucket.ERROR + bucket.WARN + bucket.INFO + bucket.DEBUG + bucket.TRACE,
  )),
)

function bucketSegmentStyle(count: number) {
  // 非零分段给最小高度，避免 1 条日志完全不可见。
  return { height: count > 0 ? `${Math.max((count / statsMax.value) * 100, 4)}%` : '0' }
}

function bucketTitle(bucket: LogStatsBucket) {
  const parsed = dayjs(bucket.bucket)
  const time = parsed.isValid() ? parsed.format('MM-DD HH:mm') : bucket.bucket
  return `${time} · ERROR ${bucket.ERROR} · WARN ${bucket.WARN} · INFO ${bucket.INFO} · DEBUG ${bucket.DEBUG} · TRACE ${bucket.TRACE}`
}

function bucketEdgeTime(index: number) {
  const bucket = statsBuckets.value[index]
  if (!bucket) return ''
  const parsed = dayjs(bucket.bucket)
  return parsed.isValid() ? parsed.format('HH:mm') : ''
}

async function loadSqlLog() {
  sqlLogLoading.value = true
  sqlLogError.value = ''
  try {
    const result = await logsApi.getSqlLog()
    sqlLogEnabled.value = result.enabled
  }
  catch (e) {
    sqlLogError.value = getApiError(e, 'SQL 日志状态加载失败')
  }
  finally {
    sqlLogLoading.value = false
  }
}

async function toggleSqlLog(enabled: boolean) {
  if (sqlLogSaving.value)
    return
  sqlLogSaving.value = true
  sqlLogError.value = ''
  try {
    const result = await logsApi.setSqlLog(enabled)
    sqlLogEnabled.value = result.enabled
    toast.success(result.enabled ? 'SQL 日志已开启' : 'SQL 日志已关闭')
  }
  catch (e) {
    toast.error(getApiError(e, 'SQL 日志开关保存失败'))
  }
  finally {
    sqlLogSaving.value = false
  }
}

function closeTail() {
  tailSource?.close()
  tailSource = undefined
}

// 建立 SSE 连接：查询串跟随当前筛选（含链路视图的 request_id），只推送连接后的新日志。
function openTail() {
  closeTail()
  tailError.value = ''
  const source = new EventSource(logsApi.buildTailUrl({
    level: level.value || undefined,
    target: target.value || undefined,
    keyword: keyword.value.trim() || undefined,
    exclude_target:
      hideSql.value && target.value !== 'sqlx::query' ? 'sqlx::query' : undefined,
    request_id: traceRequestId.value || undefined,
  }))
  tailSource = source
  source.addEventListener('log', (event) => {
    try {
      const entry = JSON.parse((event as MessageEvent).data) as LogEntry
      logs.value.unshift(entry)
      total.value += 1
      // 列表最多保留 TAIL_MAX_ENTRIES 条，超出从尾部截掉。
      if (logs.value.length > TAIL_MAX_ENTRIES)
        logs.value.splice(TAIL_MAX_ENTRIES)
    }
    catch {
      // 单条推送解析失败直接忽略，不影响后续推送。
    }
  })
  source.addEventListener('error', (event) => {
    if (event instanceof MessageEvent) {
      // 服务端显式 error 事件：非阻断提示，连接保持（浏览器自动重连）。
      tailError.value = (event as MessageEvent).data || '实时跟踪出现异常'
      return
    }
    if (source.readyState === EventSource.CLOSED) {
      // 连接已终止（如 401 未认证），原生重试无效，停止跟踪并提示。
      tailError.value = '实时跟踪连接已断开，请重新登录后重试'
      liveTail.value = false
    }
    else {
      tailError.value = '实时跟踪连接中断，正在自动重连…'
    }
  })
}

// 筛选/视图/分页变化后重建连接，仅在实时跟踪开启时生效。
function rebuildTail() {
  if (liveTail.value)
    openTail()
}

watch(liveTail, (enabled) => {
  if (enabled) {
    // 实时跟踪与自动刷新轮询互斥：开启推送时停掉 5s 轮询。
    autoRefresh.value = false
    openTail()
  }
  else {
    closeTail()
  }
})

watch(autoRefresh, (enabled) => {
  if (refreshTimer)
    clearInterval(refreshTimer)
  refreshTimer = undefined
  if (enabled) {
    // 自动刷新与实时跟踪互斥：开启轮询时停掉 SSE。
    if (liveTail.value)
      liveTail.value = false
    refreshTimer = setInterval(() => loadLogs(true), 5000)
  }
})

onBeforeUnmount(() => {
  if (refreshTimer)
    clearInterval(refreshTimer)
  if (copiedTimer)
    clearTimeout(copiedTimer)
  if (countdownTimer)
    clearInterval(countdownTimer)
  closeTail()
})

function formatTime(value: string) {
  const parsed = dayjs(value)
  return parsed.isValid() ? parsed.format('YYYY-MM-DD HH:mm:ss') : '-'
}

function toggleExpand(entry: LogEntry) {
  expandedId.value = expandedId.value === entry.id ? null : entry.id
}

// 请求内日志的 fields 可能带 http.method/http.path/http.query，拼成一行请求信息展示。
function httpRequestLine(entry: LogEntry) {
  const method = entry.fields['http.method']
  const path = entry.fields['http.path']
  if (typeof method !== 'string' || typeof path !== 'string')
    return ''
  const query = entry.fields['http.query']
  const suffix = typeof query === 'string' && query ? `?${query}` : ''
  return `${method} ${path}${suffix}`
}

// sqlx 0.8 的 query 事件 message 为空，SQL 摘要在 fields.summary：列表行回退显示它。
function rowMessage(entry: LogEntry) {
  if (entry.message)
    return entry.message
  const summary = entry.fields.summary
  return typeof summary === 'string' ? summary : ''
}

// SQL 日志的完整语句（db.statement），用于展开详情的独立高亮块。
function sqlStatement(entry: LogEntry) {
  if (entry.target !== 'sqlx::query')
    return ''
  const statement = entry.fields['db.statement']
  if (typeof statement === 'string' && statement.trim())
    return statement.trim()
  const summary = entry.fields.summary
  return typeof summary === 'string' ? summary.trim() : ''
}

// SQL 语句的绑定参数（db.parameters，由 logged_query Span 注入），与语句配对展示。
function sqlParameters(entry: LogEntry) {
  if (entry.target !== 'sqlx::query')
    return ''
  const params = entry.fields['db.parameters']
  return typeof params === 'string' && params.trim() ? params.trim() : ''
}

// 展开详情的 fields：SQL 日志的语句/摘要/参数已单独渲染，从 JSON 中剔除避免重复。
function displayFields(entry: LogEntry) {
  if (!sqlStatement(entry))
    return entry.fields
  const {
    'db.statement': _statement,
    'db.parameters': _parameters,
    summary: _summary,
    ...rest
  } = entry.fields
  return rest
}

// 复制反馈：copiedKey 标记刚复制的目标（如 sql-12 / req-12 / json-12），2 秒后复位。
const copiedKey = ref('')
let copiedTimer: ReturnType<typeof setTimeout> | undefined

async function copyText(key: string, text: string) {
  try {
    await navigator.clipboard.writeText(text)
    copiedKey.value = key
    if (copiedTimer)
      clearTimeout(copiedTimer)
    copiedTimer = setTimeout(() => (copiedKey.value = ''), 2000)
  }
  catch {
    copiedKey.value = ''
  }
}

onMounted(() => {
  loadLogs()
  loadTargets()
  loadSqlLog()
  loadStats()
  loadFilterOverride()
})

// 覆盖生效后同步展示状态并启动剩余秒数倒计时，到 0 自动重新拉取覆盖状态。
function syncOverrideState(override: LogFilterOverride) {
  filterOverride.value = override.directives ? override : null
  filterCountdown.value = override.directives ? override.restore_seconds_remaining : null
  if (countdownTimer) {
    clearInterval(countdownTimer)
    countdownTimer = undefined
  }
  if (filterCountdown.value != null) {
    countdownTimer = setInterval(() => {
      const current = filterCountdown.value
      if (current == null) {
        if (countdownTimer) {
          clearInterval(countdownTimer)
          countdownTimer = undefined
        }
        return
      }
      const next = current - 1
      filterCountdown.value = next > 0 ? next : 0
      if (next <= 0) {
        if (countdownTimer) {
          clearInterval(countdownTimer)
          countdownTimer = undefined
        }
        // 倒计时归零：覆盖应已自动复位，重新同步状态。
        loadFilterOverride()
      }
    }, 1000)
  }
}

async function loadFilterOverride() {
  try {
    syncOverrideState(await logsApi.getFilterOverride())
  }
  catch {
    // 覆盖状态加载失败不阻塞主列表。
  }
}

async function applyFilterOverride() {
  if (filterSaving.value)
    return
  filterSaving.value = true
  filterError.value = ''
  try {
    const input: { directives: string, restore_minutes?: number } = {
      directives: filterDirectives.value.trim(),
    }
    if (restoreMinutes.value)
      input.restore_minutes = Number(restoreMinutes.value)
    syncOverrideState(await logsApi.setFilterOverride(input))
    toast.success('级别覆盖已应用')
  }
  catch (e) {
    // 非法 directives 后端返回 INVALID_DIRECTIVES 400，这里展示具体错误文案。
    filterError.value = getApiError(e, '级别覆盖保存失败')
  }
  finally {
    filterSaving.value = false
  }
}

async function clearFilterOverride() {
  if (filterSaving.value)
    return
  filterSaving.value = true
  filterError.value = ''
  try {
    syncOverrideState(await logsApi.setFilterOverride({ directives: '' }))
    filterDirectives.value = ''
    toast.success('级别覆盖已清除')
  }
  catch (e) {
    filterError.value = getApiError(e, '级别覆盖清除失败')
  }
  finally {
    filterSaving.value = false
  }
}
</script>

<template>
  <BasicPage title="运行日志" description="查看服务端结构化日志，仅 Owner 可见。">
    <!-- 24h 级别趋势迷你图：纯 CSS 等高条，无图表库依赖。 -->
    <div v-if="statsBuckets.length" class="rounded-lg border bg-card p-3">
      <div class="flex h-16 items-stretch gap-0.5">
        <div
          v-for="bucket in statsBuckets"
          :key="bucket.bucket"
          class="group flex min-w-0 flex-1 flex-col justify-end"
          :title="bucketTitle(bucket)"
        >
          <div class="w-full rounded-t-sm bg-destructive/70" :style="bucketSegmentStyle(bucket.ERROR)" />
          <div class="w-full bg-amber-500/70" :style="bucketSegmentStyle(bucket.WARN)" />
          <div
            class="w-full rounded-b-sm bg-muted-foreground/30 group-hover:bg-muted-foreground/50"
            :style="bucketSegmentStyle(bucket.INFO + bucket.DEBUG + bucket.TRACE)"
          />
        </div>
      </div>
      <div class="mt-1 flex justify-between text-[10px] tabular-nums text-muted-foreground">
        <span>{{ bucketEdgeTime(0) }}</span>
        <span>最近 24 小时（红=ERROR 黄=WARN 灰=其余）</span>
        <span>{{ bucketEdgeTime(statsBuckets.length - 1) }}</span>
      </div>
      <!-- channel 满累计丢弃条数（含为 0 的情况），由 stats 响应返回。 -->
      <p class="mt-1 text-xs text-muted-foreground">日志通道已丢弃 {{ channelDropped }} 条</p>
    </div>

    <!-- 最近 24h 级别统计 -->
    <div v-if="levelCounts" class="grid grid-cols-2 gap-3 sm:grid-cols-5">
      <div
        v-for="item in LEVELS"
        :key="item"
        class="rounded-lg border px-3 py-2.5"
        :class="levelMeta[item].cardClass"
      >
        <p class="text-xs font-medium">{{ item }}</p>
        <p class="mt-0.5 text-lg font-semibold tabular-nums">{{ levelCounts[item] }}</p>
      </div>
    </div>

    <Card>
      <CardHeader class="gap-4">
        <div class="flex flex-wrap items-end gap-3">
          <div class="grid w-36 gap-2">
            <Label for="log-level">级别（最低）</Label>
            <select
              id="log-level"
              v-model="level"
              class="flex h-8 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
            >
              <option value="">全部</option>
              <option v-for="item in LEVELS" :key="item" :value="item">{{ item }}</option>
            </select>
          </div>
          <div class="grid w-44 gap-2">
            <Label for="log-target">来源模块</Label>
            <select
              id="log-target"
              v-model="target"
              class="flex h-8 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
            >
              <option value="">全部</option>
              <option v-for="item in targets" :key="item" :value="item">{{ item }}</option>
            </select>
          </div>
          <div class="grid w-48 gap-2">
            <Label for="log-keyword">关键词</Label>
            <Input
              id="log-keyword"
              v-model="keyword"
              placeholder="匹配日志内容与字段"
              class="h-8 text-sm"
              @keyup.enter="applyFilter"
            />
          </div>
          <div class="grid w-44 gap-2">
            <Label for="log-start">开始时间</Label>
            <Input id="log-start" v-model="start" type="datetime-local" class="h-8 text-sm" />
          </div>
          <div class="grid w-44 gap-2">
            <Label for="log-end">结束时间</Label>
            <Input id="log-end" v-model="end" type="datetime-local" class="h-8 text-sm" />
          </div>
          <Button variant="outline" size="sm" class="h-8" @click="applyFilter">
            <SearchIcon />
            查询
          </Button>
          <Button variant="ghost" size="sm" class="h-8" @click="resetFilter">
            重置
          </Button>
        </div>

        <!-- 时间快捷选项：自动填充 start/end 后立即查询。 -->
        <div class="flex flex-wrap items-center gap-1.5 text-xs">
          <span class="text-muted-foreground">快捷时间：</span>
          <Button variant="outline" size="sm" class="h-7 px-2 text-xs" @click="applyQuickRange('15m')">15 分钟</Button>
          <Button variant="outline" size="sm" class="h-7 px-2 text-xs" @click="applyQuickRange('1h')">1 小时</Button>
          <Button variant="outline" size="sm" class="h-7 px-2 text-xs" @click="applyQuickRange('24h')">24 小时</Button>
          <Button variant="outline" size="sm" class="h-7 px-2 text-xs" @click="applyQuickRange('today')">今天</Button>
        </div>

        <div class="flex flex-wrap items-center gap-x-6 gap-y-2 border-t pt-3">
          <!-- 开关复用既有原生 Checkbox 样式，行为与原生控件一致。 -->
          <label class="flex cursor-pointer items-center gap-2 text-xs font-medium">
            <RadioIcon class="size-3.5 text-muted-foreground" />
            实时跟踪（SSE 推送）
            <input v-model="liveTail" type="checkbox" class="peer sr-only" />
            <span class="relative h-4.5 w-8 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-3.5 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-3.5" />
          </label>
          <span v-if="liveTail && page !== 1" class="text-xs text-amber-600 dark:text-amber-500">
            实时跟踪从列表顶部推送最新日志，当前不在第 1 页，可切回跟踪最新。
          </span>
          <span v-if="tailError" role="alert" class="text-xs font-medium text-amber-600 dark:text-amber-500">{{ tailError }}</span>
          <label class="flex cursor-pointer items-center gap-2 text-xs font-medium" :class="sqlLogLoading || sqlLogSaving ? 'opacity-50' : ''">
            <DatabaseIcon class="size-3.5 text-muted-foreground" />
            SQL 日志
            <input
              type="checkbox"
              class="peer sr-only"
              :checked="sqlLogEnabled"
              :disabled="sqlLogLoading || sqlLogSaving"
              @change="toggleSqlLog(($event.target as HTMLInputElement).checked)"
            />
            <span class="relative h-4.5 w-8 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-3.5 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-3.5 peer-disabled:opacity-50" />
          </label>
          <span class="text-xs text-muted-foreground">
            输出 DEBUG 级 SQL 语句，量大时影响性能，建议排查后关闭。
          </span>
          <span v-if="sqlLogError" role="alert" class="text-xs font-medium text-destructive">{{ sqlLogError }}</span>
          <!-- 隐藏 SQL 日志：与 target 下拉选择 sqlx::query 互斥（exclude_target 与 target 参数互斥）。 -->
          <label
            class="flex items-center gap-2 text-xs font-medium"
            :class="target === 'sqlx::query' ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'"
          >
            <EyeOffIcon class="size-3.5 text-muted-foreground" />
            隐藏 SQL 日志
            <input
              v-model="hideSql"
              type="checkbox"
              class="peer sr-only"
              :disabled="target === 'sqlx::query'"
              @change="applyFilter"
            />
            <span class="relative h-4.5 w-8 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-3.5 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-3.5 peer-disabled:opacity-50" />
          </label>
          <span v-if="target === 'sqlx::query'" class="text-xs text-muted-foreground">
            当前已按 sqlx::query 精确筛选，「隐藏 SQL 日志」不可用。
          </span>
        </div>

        <!-- 运行时级别覆盖：临时提升某 target 的日志级别，可设置自动复位时间。 -->
        <div class="flex flex-wrap items-center gap-2 border-t pt-3 text-xs">
          <span class="flex items-center gap-1.5 font-medium text-muted-foreground">
            <SlidersHorizontalIcon class="size-3.5" />
            级别覆盖
          </span>
          <Input
            id="log-filter-directives"
            v-model="filterDirectives"
            placeholder="aries_server=debug,tower_http=debug"
            class="h-8 w-72 font-mono text-xs"
            :disabled="filterSaving"
            @keyup.enter="applyFilterOverride"
          />
          <select
            v-model="restoreMinutes"
            class="flex h-8 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
            :disabled="filterSaving"
          >
            <option value="">不复位</option>
            <option value="15">15 分钟后复位</option>
            <option value="30">30 分钟后复位</option>
            <option value="60">60 分钟后复位</option>
          </select>
          <Button variant="outline" size="sm" class="h-8" :disabled="filterSaving" @click="applyFilterOverride">
            应用
          </Button>
          <template v-if="filterOverride">
            <Badge class="border-transparent bg-emerald-500/15 text-xs text-emerald-700 dark:text-emerald-400">
              生效中
            </Badge>
            <code class="max-w-64 truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]" :title="filterOverride.directives">
              {{ filterOverride.directives }}
            </code>
            <span v-if="filterCountdown != null" class="tabular-nums text-muted-foreground">
              {{ filterCountdown }} 秒后自动复位
            </span>
            <Button variant="ghost" size="sm" class="h-8" :disabled="filterSaving" @click="clearFilterOverride">
              立即清除
            </Button>
          </template>
          <span v-if="filterError" role="alert" class="font-medium text-destructive">{{ filterError }}</span>
        </div>
      </CardHeader>
      <CardContent class="p-0">
        <!-- 链路视图横幅：提示当前仅展示单个请求的日志（正序）。 -->
        <div
          v-if="traceRequestId"
          class="flex flex-wrap items-center gap-x-3 gap-y-2 border-b bg-blue-500/5 px-4 py-2.5 text-xs"
        >
          <span class="flex items-center gap-1.5 font-medium text-blue-700 dark:text-blue-400">
            <WaypointsIcon class="size-3.5" />
            正在查看请求链路
            <code class="rounded bg-background px-1.5 py-0.5 font-mono">{{ traceRequestId }}</code>
            （共 {{ total }} 条，按时间正序）
          </span>
          <Button variant="outline" size="sm" class="ml-auto h-7 text-xs" @click="exitTrace">
            退出链路视图
          </Button>
        </div>
        <!-- 上下文视图横幅：以锚点行为中心，服务端忽略分页与时间筛选。 -->
        <div
          v-if="contextAnchor"
          class="flex flex-wrap items-center gap-x-3 gap-y-2 border-b bg-violet-500/5 px-4 py-2.5 text-xs"
        >
          <span class="flex items-center gap-1.5 font-medium text-violet-700 dark:text-violet-400">
            <AlignHorizontalDistributeCenterIcon class="size-3.5" />
            正在查看 {{ formatTime(contextAnchor.ts) }} 前后的日志上下文（共 {{ total }} 条，按时间正序）
          </span>
          <Button variant="outline" size="sm" class="ml-auto h-7 text-xs" @click="exitContext">
            退出上下文视图
          </Button>
        </div>
        <p v-if="error" class="border-b px-4 py-3 text-sm text-destructive">{{ error }}</p>

        <div v-if="loading" class="space-y-2 p-4">
          <div v-for="index in 6" :key="index" class="h-10 w-full rounded-md bg-muted/60" />
        </div>

        <AppEmptyState
          v-else-if="logs.length === 0"
          :icon="SquareTerminalIcon"
          title="没有匹配的日志"
          description="调整筛选条件后重新查询。"
        />

        <div v-else class="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead class="w-40">时间</TableHead>
                <TableHead class="w-20">级别</TableHead>
                <TableHead class="w-48">来源</TableHead>
                <TableHead>内容</TableHead>
                <TableHead class="w-10" />
              </TableRow>
            </TableHeader>
            <TableBody>
              <template v-for="entry in logs" :key="entry.id">
                <TableRow class="cursor-pointer" @click="toggleExpand(entry)">
                  <TableCell class="whitespace-nowrap text-xs tabular-nums text-muted-foreground">
                    {{ formatTime(entry.ts) }}
                  </TableCell>
                  <TableCell>
                    <Badge :class="levelMeta[entry.level].badgeClass" class="text-xs">
                      {{ entry.level }}
                    </Badge>
                  </TableCell>
                  <TableCell class="max-w-48 truncate font-mono text-xs text-muted-foreground" :title="entry.target">
                    {{ entry.target }}
                  </TableCell>
                  <TableCell class="max-w-md">
                    <span class="block truncate text-xs" :class="entry.message ? '' : 'font-mono'" :title="rowMessage(entry)">
                      {{ rowMessage(entry) }}
                    </span>
                    <span
                      v-if="httpRequestLine(entry)"
                      class="mt-0.5 block truncate font-mono text-[11px] text-muted-foreground"
                      :title="httpRequestLine(entry)"
                    >
                      {{ httpRequestLine(entry) }}
                    </span>
                  </TableCell>
                  <TableCell>
                    <ChevronDownIcon
                      class="size-4 text-muted-foreground transition-transform"
                      :class="expandedId === entry.id ? 'rotate-180' : ''"
                    />
                  </TableCell>
                </TableRow>
                <TableRow v-if="expandedId === entry.id" class="bg-muted/30 hover:bg-muted/30">
                  <TableCell colspan="5" class="p-4">
                    <p v-if="rowMessage(entry)" class="mb-3 whitespace-pre-wrap break-all text-xs leading-5">{{ rowMessage(entry) }}</p>
                    <!-- SQL 日志：完整语句单独高亮渲染（highlightSql 输出已转义的安全 HTML），附复制与绑定参数。 -->
                    <div v-if="sqlStatement(entry)" class="relative mb-3">
                      <pre
                        class="max-h-56 overflow-auto whitespace-pre-wrap break-all rounded-md bg-background p-3 pr-10 font-mono text-[11px] leading-5"
                        v-html="highlightSql(sqlStatement(entry))"
                      />
                      <button
                        type="button"
                        class="absolute right-2 top-2 rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                        :title="copiedKey === `sql-${entry.id}` ? '已复制' : '复制 SQL'"
                        @click.stop="copyText(`sql-${entry.id}`, sqlStatement(entry))"
                      >
                        <CheckIcon v-if="copiedKey === `sql-${entry.id}`" class="size-3.5 text-emerald-600" />
                        <CopyIcon v-else class="size-3.5" />
                      </button>
                      <p
                        v-if="sqlParameters(entry)"
                        class="mt-1.5 break-all font-mono text-[11px] text-muted-foreground"
                        :title="sqlParameters(entry)"
                      >
                        参数 {{ sqlParameters(entry) }}
                      </p>
                    </div>
                    <dl class="grid gap-2 text-xs sm:grid-cols-2">
                      <div v-if="entry.span_name" class="flex gap-2">
                        <dt class="shrink-0 text-muted-foreground">Span</dt>
                        <dd class="font-mono">{{ entry.span_name }}</dd>
                      </div>
                      <div v-if="entry.request_id" class="flex items-center gap-2">
                        <dt class="shrink-0 text-muted-foreground">Request ID</dt>
                        <dd class="font-mono">{{ entry.request_id }}</dd>
                        <button
                          type="button"
                          class="shrink-0 rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                          :title="copiedKey === `req-${entry.id}` ? '已复制' : '复制 Request ID'"
                          @click.stop="copyText(`req-${entry.id}`, entry.request_id!)"
                        >
                          <CheckIcon v-if="copiedKey === `req-${entry.id}`" class="size-3 text-emerald-600" />
                          <CopyIcon v-else class="size-3" />
                        </button>
                        <!-- 点击行会展开/收起，链路入口需阻止冒泡。 -->
                        <button
                          v-if="traceRequestId !== entry.request_id"
                          type="button"
                          class="shrink-0 font-medium text-primary underline underline-offset-2 hover:text-primary/80"
                          @click.stop="enterTrace(entry.request_id!)"
                        >
                          查看请求链路
                        </button>
                      </div>
                    </dl>
                    <!-- fields 的 JSON 高亮（highlightJson 输出已转义的安全 HTML），附复制。 -->
                    <div v-if="Object.keys(displayFields(entry)).length > 0" class="relative mt-3">
                      <pre
                        class="max-h-56 overflow-auto whitespace-pre-wrap break-all rounded-md bg-background p-3 pr-10 font-mono text-[11px] leading-5"
                        v-html="highlightJson(displayFields(entry))"
                      />
                      <button
                        type="button"
                        class="absolute right-2 top-2 rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                        :title="copiedKey === `json-${entry.id}` ? '已复制' : '复制字段 JSON'"
                        @click.stop="copyText(`json-${entry.id}`, JSON.stringify(displayFields(entry), null, 2))"
                      >
                        <CheckIcon v-if="copiedKey === `json-${entry.id}`" class="size-3.5 text-emerald-600" />
                        <CopyIcon v-else class="size-3.5" />
                      </button>
                    </div>
                    <div v-if="!contextAnchor" class="mt-3">
                      <button
                        type="button"
                        class="font-medium text-xs text-primary underline underline-offset-2 hover:text-primary/80"
                        @click.stop="enterContext(entry)"
                      >
                        查看前后上下文
                      </button>
                    </div>
                  </TableCell>
                </TableRow>
              </template>
            </TableBody>
          </Table>
        </div>
      </CardContent>
      <!-- 上下文模式下分页被服务端忽略，隐藏分页控件。 -->
      <div v-if="!contextAnchor" class="border-t p-4">
        <AppDataTablePagination
          v-model:page="page"
          v-model:page-size="pageSize"
          :total="total"
          :loading="loading"
          unit="条日志"
        />
      </div>
    </Card>
  </BasicPage>
</template>

<route lang="yaml">
meta:
  permission: settings:manage
</route>
