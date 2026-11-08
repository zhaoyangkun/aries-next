<script setup lang="ts">
/** 系统监控页：主机 CPU / 内存 / 磁盘 / 网络实时指标（SSE 推送）、进程 Top 与历史趋势。 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import dayjs from 'dayjs'
import { CpuIcon, HardDriveIcon, MemoryStickIcon, NetworkIcon, ServerIcon } from '@lucide/vue'

import { systemApi } from '@/modules/system/api/system'
import type { HistoryBucket, ProcessMetrics, SystemMetrics } from '@/modules/system/api/system'
import GaugeRing from '@/modules/system/components/GaugeRing.vue'
import Sparkline from '@/modules/system/components/Sparkline.vue'
import { formatBitrate, formatBytes, formatDuration, formatPercent, formatRate, toneSolidClass, toneTextClass, usageTone } from '@/modules/system/utils/format'
import { getApiError } from '@/shared/api/client'

// 实时序列保留点数：5s × 120 = 最近 10 分钟。
const LIVE_POINTS = 120

type ConnectionState = 'connecting' | 'open' | 'closed'
const connectionState = ref<ConnectionState>('connecting')
const connectionLabel = computed(() => {
  if (connectionState.value === 'open')
    return '实时监控中'
  if (connectionState.value === 'closed')
    return '连接已断开'
  return '连接中…'
})

const metrics = ref<SystemMetrics | null>(null)
const error = ref('')

// 最近一帧的本地时间（页面右上角显示）。
const lastUpdated = computed(() => {
  const ts = metrics.value?.ts
  return ts ? dayjs(ts).format('HH:mm:ss') : ''
})

// 实时曲线序列（SSE 推送驱动追加）。
const cpuSeries = ref<number[]>([])
const memorySeries = ref<number[]>([])
const netRxSeries = ref<number[]>([])
const netTxSeries = ref<number[]>([])

function pushSeries(series: number[], value: number) {
  series.push(value)
  if (series.length > LIVE_POINTS)
    series.shift()
}

let source: EventSource | undefined

function closeStream() {
  source?.close()
  source = undefined
}

// SSE：5s 一帧快照；浏览器断线自动重连，连接态变化同步到状态徽章。
function openStream() {
  closeStream()
  connectionState.value = 'connecting'
  const stream = new EventSource(systemApi.buildStreamUrl())
  source = stream
  stream.addEventListener('metrics', (event) => {
    try {
      const snapshot = JSON.parse((event as MessageEvent).data) as SystemMetrics
      metrics.value = snapshot
      pushSeries(cpuSeries.value, snapshot.cpu.percent)
      pushSeries(memorySeries.value, snapshot.memory.percent)
      pushSeries(netRxSeries.value, snapshot.network.rx_bytes_per_sec)
      pushSeries(netTxSeries.value, snapshot.network.tx_bytes_per_sec)
    }
    catch {
      // 单帧解析失败直接忽略，等待下一帧。
    }
  })
  stream.addEventListener('error', () => {
    if (stream.readyState === EventSource.OPEN) {
      connectionState.value = 'open'
    }
    else if (stream.readyState === EventSource.CONNECTING) {
      connectionState.value = 'connecting'
    }
    else {
      connectionState.value = 'closed'
    }
  })
  stream.addEventListener('open', () => {
    connectionState.value = 'open'
  })
}

// 历史趋势：区间切换重新拉取。
const historyHours = ref(1)
const historyBuckets = ref<HistoryBucket[]>([])
const historyLoading = ref(false)

async function loadHistory() {
  historyLoading.value = true
  try {
    const history = await systemApi.history(historyHours.value)
    historyBuckets.value = history.buckets
  }
  catch (e) {
    error.value = getApiError(e, '历史趋势加载失败')
  }
  finally {
    historyLoading.value = false
  }
}

function switchHistory(hours: number | string) {
  historyHours.value = Number(hours)
  loadHistory()
}

const historyCpu = computed(() => historyBuckets.value.map(b => b.cpu_percent))
const historyMemory = computed(() => historyBuckets.value.map(b => b.memory_total_bytes > 0
  ? (b.memory_used_bytes / b.memory_total_bytes) * 100
  : 0,
))
const historyDisk = computed(() => historyBuckets.value.map(b => b.disk_used_bytes))
const historyNetRx = computed(() => historyBuckets.value.map(b => b.network_rx_bytes_per_sec))
const historyNetTx = computed(() => historyBuckets.value.map(b => b.network_tx_bytes_per_sec))
// 网络历史纵轴上限取双序列峰值，让 RX/TX 同图可比。
const historyNetMax = computed(() => Math.max(...historyNetRx.value, ...historyNetTx.value, 1))

// 历史趋势时间轴：窗口内首尾采样点的本地时间。
const historyRange = computed(() => {
  const first = historyBuckets.value[0]?.bucket
  const last = historyBuckets.value[historyBuckets.value.length - 1]?.bucket
  if (!first || !last)
    return ''
  return `${dayjs(first).format('MM-DD HH:mm')} — ${dayjs(last).format('MM-DD HH:mm')}`
})

// 磁盘使用率（服务端只回 used/total，百分比前端计算）。
function diskPercent(disk: { used_bytes: number, total_bytes: number }) {
  if (disk.total_bytes === 0)
    return 0
  return Math.min(100, (disk.used_bytes / disk.total_bytes) * 100)
}

// 主频格式化：MHz → GHz（>=1000）或 MHz。
function formatFrequency(mhz: number | null): string {
  if (mhz == null || mhz <= 0)
    return '-'
  if (mhz >= 1000)
    return `${(mhz / 1000).toFixed(2)} GHz`
  return `${mhz} MHz`
}

// 温度配色：>=90℃ 告警、>=75℃ 注意。
function tempClass(celsius: number): string {
  if (celsius >= 90)
    return 'text-destructive'
  if (celsius >= 75)
    return 'text-amber-600 dark:text-amber-400'
  return ''
}

// 进程 Top：服务端 15 秒缓存，前端 10 秒轮询跟随（权限/失败静默，不打扰主指标）。
const processes = ref<ProcessMetrics[]>([])
const processesLoading = ref(false)

async function loadProcesses() {
  processesLoading.value = true
  try {
    // 固定按 CPU 降序：表格已同时展示 CPU 与内存两列，无需排序切换。
    const top = await systemApi.processes(10)
    processes.value = top.processes
  }
  catch {
    // 进程列表属增强信息，加载失败保留现状即可。
  }
  finally {
    processesLoading.value = false
  }
}

let processesTimer: ReturnType<typeof setInterval> | undefined

// 负载解读：load1 与核数对比（Windows 无 loadavg，恒 0 显示不可用）。
const loadStatus = computed(() => {
  if (!metrics.value)
    return null
  const { one } = metrics.value.load
  if (one === 0)
    return null
  const cores = metrics.value.cpu.per_core.length || 1
  return {
    ratio: one / cores,
    label: one >= cores ? '偏高' : '正常',
  }
})

onMounted(async () => {
  // 首帧前先用快照端点填充页面，避免打开时空白等待。
  try {
    metrics.value = await systemApi.get()
    connectionState.value = 'connecting'
  }
  catch (e) {
    error.value = getApiError(e, '系统指标加载失败')
  }
  openStream()
  loadHistory()
  loadProcesses()
  processesTimer = setInterval(loadProcesses, 10_000)
})

onBeforeUnmount(() => {
  closeStream()
  if (processesTimer) {
    clearInterval(processesTimer)
    processesTimer = undefined
  }
})
</script>

<template>
  <BasicPage title="系统监控" description="服务器 CPU、内存、磁盘与网络实时指标和历史趋势。" sticky>
    <template #actions>
      <Badge variant="secondary" class="h-7 gap-2 px-2.5 font-normal">
        <span
          class="size-1.5 rounded-full"
          :class="[connectionState === 'open' ? 'bg-green-500' : connectionState === 'connecting' ? 'bg-amber-500 animate-pulse' : 'bg-destructive']"
        />
        {{ connectionLabel }}
      </Badge>
    </template>

    <p v-if="error" class="mb-4 text-sm text-destructive">{{ error }}</p>

    <div class="space-y-6">
      <!-- 分区标题：左侧色条 + 标题 + 贯穿分隔线，明确模块边界。 -->
      <div class="flex items-center gap-3">
        <span class="h-4 w-1 rounded-full bg-primary" />
        <h2 class="text-sm font-semibold">
          实时资源
        </h2>
        <Separator class="flex-1" />
        <span class="text-xs text-muted-foreground">
          SSE 每 5 秒推送<template v-if="lastUpdated"> · 更新于 {{ lastUpdated }}</template>
        </span>
      </div>

      <div class="grid gap-4 lg:grid-cols-3">
        <!-- CPU -->
        <Card>
          <CardHeader class="flex-row items-center gap-3 space-y-0 pb-3">
            <div class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <CpuIcon class="size-4" />
            </div>
            <div class="min-w-0 flex-1">
              <CardTitle class="text-sm font-medium">处理器</CardTitle>
              <p class="truncate text-xs text-muted-foreground">
                {{ metrics?.cpu.brand || '型号未知' }}
              </p>
            </div>
            <Badge variant="outline" class="h-5 shrink-0 px-1.5 text-xs font-normal tabular-nums">
              {{ metrics ? `${metrics.cpu.per_core.length} 核 · ${formatFrequency(metrics.cpu.frequency_mhz)}` : '-' }}
            </Badge>
          </CardHeader>
          <CardContent class="space-y-3">
            <div class="flex items-center gap-5">
              <GaugeRing
                :percent="metrics?.cpu.percent ?? 0"
                :tone="metrics ? usageTone(metrics.cpu.percent) : 'ok'"
              >
                <span
                  class="text-2xl font-semibold tabular-nums"
                  :class="metrics ? toneTextClass[usageTone(metrics.cpu.percent)] : ''"
                >
                  {{ metrics ? formatPercent(metrics.cpu.percent) : '-' }}
                </span>
                <span class="text-[10px] text-muted-foreground">已使用</span>
              </GaugeRing>
              <div class="min-w-0 flex-1 space-y-1.5">
                <p class="text-[10px] text-muted-foreground">
                  各核占用
                </p>
                <!-- 每核 heatmap：高度映射使用率，颜色映射阈值档位。 -->
                <div class="flex h-16 items-end gap-0.5">
                  <div
                    v-for="(usage, index) in metrics?.cpu.per_core ?? []"
                    :key="index"
                    class="min-w-1 flex-1 rounded-sm transition-all duration-500"
                    :class="toneSolidClass[usageTone(usage)]"
                    :style="{ height: `${Math.min(100, Math.max(0, usage))}%` }"
                  />
                </div>
              </div>
            </div>
            <div class="space-y-1 border-t pt-2">
              <p class="text-[10px] text-muted-foreground">
                最近 10 分钟
              </p>
              <Sparkline :points="cpuSeries" :max="100" :height="40" />
            </div>
          </CardContent>
        </Card>

        <!-- 内存 -->
        <Card>
          <CardHeader class="flex-row items-center gap-3 space-y-0 pb-3">
            <div class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <MemoryStickIcon class="size-4" />
            </div>
            <div class="min-w-0 flex-1">
              <CardTitle class="text-sm font-medium">内存</CardTitle>
              <p class="truncate text-xs text-muted-foreground">
                物理内存 + Swap
              </p>
            </div>
            <Badge variant="outline" class="h-5 shrink-0 px-1.5 text-xs font-normal tabular-nums">
              {{ metrics ? formatBytes(metrics.memory.total_bytes) : '-' }}
            </Badge>
          </CardHeader>
          <CardContent class="space-y-3">
            <div class="flex items-center gap-5">
              <GaugeRing
                :percent="metrics?.memory.percent ?? 0"
                :tone="metrics ? usageTone(metrics.memory.percent) : 'ok'"
              >
                <span
                  class="text-2xl font-semibold tabular-nums"
                  :class="metrics ? toneTextClass[usageTone(metrics.memory.percent)] : ''"
                >
                  {{ metrics ? formatPercent(metrics.memory.percent) : '-' }}
                </span>
                <span class="text-[10px] text-muted-foreground">已使用</span>
              </GaugeRing>
              <div class="min-w-0 flex-1 space-y-2 text-xs">
                <div>
                  <p class="text-[10px] text-muted-foreground">
                    已用 / 总量
                  </p>
                  <p class="font-medium tabular-nums">
                    {{ metrics ? formatBytes(metrics.memory.used_bytes) : '-' }}
                    <span class="text-muted-foreground">/ {{ metrics ? formatBytes(metrics.memory.total_bytes) : '-' }}</span>
                  </p>
                </div>
                <div>
                  <p class="text-[10px] text-muted-foreground">
                    可用
                  </p>
                  <p class="font-medium tabular-nums">
                    {{ metrics ? formatBytes(metrics.memory.available_bytes) : '-' }}
                  </p>
                </div>
                <div v-if="metrics && metrics.swap.total_bytes > 0">
                  <p class="text-[10px] text-muted-foreground">
                    Swap
                  </p>
                  <p class="font-medium tabular-nums">
                    {{ formatBytes(metrics.swap.used_bytes) }}
                    <span class="text-muted-foreground">/ {{ formatBytes(metrics.swap.total_bytes) }}</span>
                  </p>
                </div>
              </div>
            </div>
            <div class="space-y-1 border-t pt-2">
              <p class="text-[10px] text-muted-foreground">
                最近 10 分钟
              </p>
              <Sparkline :points="memorySeries" :max="100" :height="40" />
            </div>
          </CardContent>
        </Card>

        <!-- 网络 -->
        <Card>
          <CardHeader class="flex-row items-center gap-3 space-y-0 pb-3">
            <div class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <NetworkIcon class="size-4" />
            </div>
            <div class="min-w-0 flex-1">
              <CardTitle class="text-sm font-medium">网络 IO</CardTitle>
              <p class="truncate text-xs text-muted-foreground">
                全部网卡合计
              </p>
            </div>
            <Badge variant="outline" class="h-5 shrink-0 px-1.5 text-xs font-normal">
              {{ metrics?.network.interfaces.length ?? 0 }} 个接口
            </Badge>
          </CardHeader>
          <CardContent class="space-y-2">
            <div class="grid grid-cols-2 gap-3">
              <div>
                <p class="text-[10px] text-muted-foreground">
                  ↓ 接收
                </p>
                <p class="text-lg font-semibold tabular-nums">
                  {{ metrics ? formatBitrate(metrics.network.rx_bytes_per_sec) : '-' }}
                </p>
              </div>
              <div>
                <p class="text-[10px] text-muted-foreground">
                  ↑ 发送
                </p>
                <p class="text-lg font-semibold tabular-nums">
                  {{ metrics ? formatBitrate(metrics.network.tx_bytes_per_sec) : '-' }}
                </p>
              </div>
            </div>
            <Sparkline :points="netRxSeries" :height="28" />
            <Sparkline :points="netTxSeries" :height="28" stroke="var(--chart-2)" />
            <!-- 各网卡明细：接口多时限制高度滚动。 -->
            <div v-if="metrics && metrics.network.interfaces.length > 0" class="max-h-24 space-y-1 overflow-y-auto border-t pt-2">
              <div
                v-for="iface in metrics.network.interfaces"
                :key="iface.name"
                class="flex items-center justify-between gap-2 text-xs"
              >
                <span class="truncate font-mono text-muted-foreground">{{ iface.name }}</span>
                <span class="shrink-0 tabular-nums">
                  ↓{{ formatRate(iface.rx_bytes_per_sec) }} ↑{{ formatRate(iface.tx_bytes_per_sec) }}
                </span>
              </div>
            </div>
            <p class="text-xs text-muted-foreground">
              累计 {{ metrics ? formatBytes(metrics.network.total_received_bytes) : '-' }} ↓ /
              {{ metrics ? formatBytes(metrics.network.total_transmitted_bytes) : '-' }} ↑
            </p>
          </CardContent>
        </Card>
      </div>

      <div class="flex items-center gap-3">
        <span class="h-4 w-1 rounded-full bg-primary" />
        <h2 class="text-sm font-semibold">
          存储与系统
        </h2>
        <Separator class="flex-1" />
      </div>

      <div class="grid gap-4 lg:grid-cols-3">
        <!-- 磁盘 -->
        <Card class="lg:col-span-2">
          <CardHeader class="flex-row items-center gap-3 space-y-0 pb-3">
            <div class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <HardDriveIcon class="size-4" />
            </div>
            <div class="min-w-0 flex-1">
              <CardTitle class="text-sm font-medium">磁盘占用</CardTitle>
              <p class="truncate text-xs text-muted-foreground">
                {{ metrics ? `${metrics.disks.length} 个挂载点` : '' }}
              </p>
            </div>
          </CardHeader>
          <CardContent class="space-y-4">
            <div v-for="disk in metrics?.disks ?? []" :key="disk.mount_point" class="space-y-1.5">
              <div class="flex items-baseline justify-between gap-2 text-sm">
                <span class="truncate font-medium">
                  {{ disk.mount_point }}
                  <span class="ml-2 text-xs font-normal text-muted-foreground">
                    {{ disk.file_system }} · {{ disk.name }}
                  </span>
                </span>
                <span class="shrink-0 text-xs tabular-nums text-muted-foreground">
                  {{ formatBytes(disk.used_bytes) }} / {{ formatBytes(disk.total_bytes) }}
                </span>
              </div>
              <div class="flex items-center gap-3">
                <!-- 自绘百分比条（width 驱动）：填充为主题色，比 transform 方案行为更可控。 -->
                <div class="h-2 flex-1 overflow-hidden rounded-full bg-primary/15">
                  <div
                    class="h-full rounded-full bg-primary transition-[width] duration-500"
                    :style="{ width: `${diskPercent(disk)}%` }"
                  />
                </div>
                <span
                  class="w-12 text-right text-xs font-medium tabular-nums"
                  :class="toneTextClass[usageTone(diskPercent(disk))]"
                >
                  {{ formatPercent(diskPercent(disk)) }}
                </span>
              </div>
            </div>
            <p v-if="!metrics" class="text-sm text-muted-foreground">
              等待数据…
            </p>
          </CardContent>
        </Card>

        <!-- 系统与服务信息 -->
        <Card>
          <CardHeader class="flex-row items-center gap-3 space-y-0 pb-3">
            <div class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <ServerIcon class="size-4" />
            </div>
            <div class="min-w-0 flex-1">
              <CardTitle class="text-sm font-medium">系统与服务</CardTitle>
              <p class="truncate text-xs text-muted-foreground">
                {{ metrics?.host_name || '主机信息' }}
              </p>
            </div>
          </CardHeader>
          <CardContent class="space-y-2 text-sm">
            <div class="flex justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">系统</span>
              <span class="truncate text-right">{{ metrics?.os_version ?? '-' }}</span>
            </div>
            <div class="flex justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">内核</span>
              <span class="truncate text-right">{{ metrics?.kernel_version ?? '-' }}</span>
            </div>
            <div class="flex justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">运行时长</span>
              <span class="tabular-nums">{{ metrics ? formatDuration(metrics.uptime_seconds) : '-' }}</span>
            </div>
            <div class="flex justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">进程数</span>
              <span class="tabular-nums">{{ metrics?.process_count ?? '-' }}</span>
            </div>
            <div class="flex items-center justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">负载 1/5/15 分</span>
              <span class="tabular-nums">
                {{ metrics ? `${metrics.load.one.toFixed(2)} / ${metrics.load.five.toFixed(2)} / ${metrics.load.fifteen.toFixed(2)}` : '-' }}
              </span>
            </div>
            <p v-if="loadStatus" class="text-right text-xs">
              <span
                :class="loadStatus.label === '偏高' ? 'text-amber-600 dark:text-amber-400' : 'text-green-600 dark:text-green-400'"
              >
                {{ loadStatus.label }}（{{ loadStatus.ratio.toFixed(2) }}×核数）
              </span>
            </p>
            <!-- 温度传感器：无传感器环境不渲染。 -->
            <template v-if="metrics && metrics.sensors.length > 0">
              <Separator class="my-2" />
              <div
                v-for="sensor in metrics.sensors.slice(0, 3)"
                :key="sensor.label"
                class="flex justify-between gap-2"
              >
                <span class="truncate text-muted-foreground" :title="sensor.label">{{ sensor.label }}</span>
                <span class="shrink-0 tabular-nums" :class="tempClass(sensor.temperature_celsius)">
                  {{ sensor.temperature_celsius.toFixed(1) }} ℃
                </span>
              </div>
            </template>
            <Separator class="my-2" />
            <div class="flex justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">服务 RSS</span>
              <span class="tabular-nums">{{ metrics ? formatBytes(metrics.server.rss_bytes) : '-' }}</span>
            </div>
            <div class="flex justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">服务运行时长</span>
              <span class="tabular-nums">{{ metrics ? formatDuration(metrics.server.uptime_seconds) : '-' }}</span>
            </div>
            <div class="flex justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">数据库大小</span>
              <span class="tabular-nums">{{ metrics?.pg ? formatBytes(metrics.pg.database_size_bytes) : '-' }}</span>
            </div>
            <div class="flex justify-between gap-2">
              <span class="shrink-0 text-muted-foreground">数据库连接</span>
              <span class="tabular-nums">{{ metrics?.pg?.active_connections ?? '-' }}</span>
            </div>
          </CardContent>
        </Card>
      </div>

      <div class="flex items-center gap-3">
        <span class="h-4 w-1 rounded-full bg-primary" />
        <h2 class="text-sm font-semibold">
          进程
        </h2>
        <Separator class="flex-1" />
        <span class="text-xs text-muted-foreground">每 10 秒自动刷新</span>
      </div>

      <!-- 进程 Top：10s 轮询（服务端 15s 缓存），支持按 CPU / 内存排序。 -->
      <Card>
        <CardHeader class="flex-row items-center gap-3 space-y-0 pb-3">
          <div class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <CpuIcon class="size-4" />
          </div>
          <div class="min-w-0 flex-1">
            <CardTitle class="text-sm font-medium">资源占用 Top 10</CardTitle>
            <p class="truncate text-xs text-muted-foreground">
              按 CPU 使用率降序 · 单核口径（100% = 打满一个核）
            </p>
          </div>
        </CardHeader>
        <CardContent>
          <div v-if="processes.length === 0" class="py-6 text-center text-sm text-muted-foreground">
            <Skeleton v-if="processesLoading" class="mx-auto h-5 w-48" />
            <template v-else>暂无进程数据</template>
          </div>
          <div v-else class="overflow-x-auto">
            <table class="w-full text-sm">
              <thead>
                <tr class="border-b text-left text-xs text-muted-foreground">
                  <th class="pb-2 pr-4 font-medium">进程</th>
                  <th class="pb-2 pr-4 text-right font-medium">PID</th>
                  <th class="pb-2 pr-4 text-right font-medium">CPU</th>
                  <th class="pb-2 text-right font-medium">内存（RSS）</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="process in processes" :key="process.pid" class="border-b last:border-0">
                  <td class="max-w-64 truncate py-2 pr-4 font-medium" :title="process.name">
                    {{ process.name || '(未命名)' }}
                  </td>
                  <td class="py-2 pr-4 text-right tabular-nums text-muted-foreground">
                    {{ process.pid }}
                  </td>
                  <td
                    class="py-2 pr-4 text-right tabular-nums"
                    :class="toneTextClass[usageTone(process.cpu_percent)]"
                  >
                    {{ formatPercent(process.cpu_percent) }}
                  </td>
                  <td class="py-2 text-right tabular-nums">
                    {{ formatBytes(process.memory_bytes) }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </CardContent>
      </Card>

      <div class="flex items-center gap-3">
        <span class="h-4 w-1 rounded-full bg-primary" />
        <h2 class="text-sm font-semibold">
          历史趋势
        </h2>
        <Separator class="flex-1" />
        <span class="text-xs text-muted-foreground">10 秒采样 · 保留 24 小时 · 重启清空</span>
      </div>

      <!-- 历史趋势 -->
      <Card>
        <CardHeader class="flex-row items-center justify-between gap-4 space-y-0">
          <CardTitle class="text-sm font-medium">资源用量走势</CardTitle>
          <Tabs :model-value="historyHours" class="ml-auto" @update:model-value="switchHistory">
            <TabsList class="w-full lg:w-auto">
              <TabsTrigger class="flex-1 lg:flex-none" :value="1">1 小时</TabsTrigger>
              <TabsTrigger class="flex-1 lg:flex-none" :value="6">6 小时</TabsTrigger>
              <TabsTrigger class="flex-1 lg:flex-none" :value="24">24 小时</TabsTrigger>
            </TabsList>
          </Tabs>
        </CardHeader>
        <CardContent>
          <div v-if="historyLoading && historyBuckets.length === 0" class="space-y-2">
            <Skeleton v-for="index in 3" :key="index" class="h-14 w-full" />
          </div>
          <div v-else-if="historyBuckets.length < 2" class="py-8 text-center text-sm text-muted-foreground">
            窗口内暂无历史采样数据（采样任务随服务启动，运行一段时间后可用）。
          </div>
          <div v-else class="grid gap-6 md:grid-cols-2 xl:grid-cols-4">
            <div class="space-y-1">
              <p class="text-xs text-muted-foreground">CPU 使用率（%）</p>
              <Sparkline :points="historyCpu" :max="100" :height="72" />
            </div>
            <div class="space-y-1">
              <p class="text-xs text-muted-foreground">内存使用率（%）</p>
              <Sparkline :points="historyMemory" :max="100" :height="72" />
            </div>
            <div class="space-y-1">
              <p class="text-xs text-muted-foreground">磁盘已用</p>
              <Sparkline :points="historyDisk" :height="72" />
            </div>
            <div class="space-y-1">
              <p class="text-xs text-muted-foreground">网络速率（RX / TX）</p>
              <Sparkline :points="historyNetRx" :max="historyNetMax" :height="34" />
              <Sparkline :points="historyNetTx" :max="historyNetMax" :height="34" stroke="var(--chart-2)" />
            </div>
          </div>
          <p v-if="historyRange" class="mt-3 text-right text-xs tabular-nums text-muted-foreground">
            {{ historyRange }}
          </p>
        </CardContent>
      </Card>
    </div>
  </BasicPage>
</template>

<route lang="yaml">
meta:
  permission: settings:manage
</route>
