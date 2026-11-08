<script setup lang="ts">
/** 系统监控页：主机 CPU / 内存 / 磁盘 / 网络实时指标（SSE 推送）与历史趋势。 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

import { systemApi } from '@/modules/system/api/system'
import type { HistoryBucket, SystemMetrics } from '@/modules/system/api/system'
import Sparkline from '@/modules/system/components/Sparkline.vue'
import { formatBytes, formatDuration, formatPercent, formatRate } from '@/modules/system/utils/format'
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
const historyNetRx = computed(() => historyBuckets.value.map(b => b.network_rx_bytes_per_sec))
const historyNetTx = computed(() => historyBuckets.value.map(b => b.network_tx_bytes_per_sec))
// 网络历史纵轴上限取双序列峰值，让 RX/TX 同图可比。
const historyNetMax = computed(() => Math.max(...historyNetRx.value, ...historyNetTx.value, 1))

// 磁盘使用率（服务端只回 used/total，百分比前端计算）。
function diskPercent(disk: { used_bytes: number, total_bytes: number }) {
  if (disk.total_bytes === 0)
    return 0
  return Math.min(100, (disk.used_bytes / disk.total_bytes) * 100)
}

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
})

onBeforeUnmount(closeStream)
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

    <div class="grid gap-4 lg:grid-cols-3">
      <!-- CPU -->
      <Card>
        <CardHeader class="pb-2">
          <CardTitle class="text-sm font-medium text-muted-foreground">CPU</CardTitle>
        </CardHeader>
        <CardContent class="space-y-3">
          <div class="flex items-baseline gap-2">
            <span class="text-3xl font-semibold tabular-nums">
              {{ metrics ? formatPercent(metrics.cpu.percent) : '-' }}
            </span>
            <span class="text-xs text-muted-foreground">
              {{ metrics ? `${metrics.cpu.per_core.length} 核` : '' }}
            </span>
          </div>
          <Sparkline :points="cpuSeries" :max="100" :height="56" />
          <!-- 每核占用格：高度映射使用率。 -->
          <div class="flex h-6 items-end gap-0.5">
            <div
              v-for="(usage, index) in metrics?.cpu.per_core ?? []"
              :key="index"
              class="min-w-1 flex-1 rounded-sm bg-primary/80 transition-all duration-500"
              :style="{ height: `${Math.min(100, Math.max(0, usage))}%` }"
            />
          </div>
        </CardContent>
      </Card>

      <!-- 内存 -->
      <Card>
        <CardHeader class="pb-2">
          <CardTitle class="text-sm font-medium text-muted-foreground">内存</CardTitle>
        </CardHeader>
        <CardContent class="space-y-3">
          <div class="flex items-baseline justify-between gap-2">
            <span class="text-3xl font-semibold tabular-nums">
              {{ metrics ? formatPercent(metrics.memory.percent) : '-' }}
            </span>
            <span class="text-xs tabular-nums text-muted-foreground">
              {{ metrics ? `${formatBytes(metrics.memory.used_bytes)} / ${formatBytes(metrics.memory.total_bytes)}` : '' }}
            </span>
          </div>
          <Progress :model-value="metrics?.memory.percent ?? 0" class="h-2" />
          <Sparkline :points="memorySeries" :max="100" :height="56" />
          <div v-if="metrics && metrics.swap.total_bytes > 0" class="text-xs text-muted-foreground">
            Swap {{ formatBytes(metrics.swap.used_bytes) }} / {{ formatBytes(metrics.swap.total_bytes) }}
          </div>
        </CardContent>
      </Card>

      <!-- 网络 -->
      <Card>
        <CardHeader class="pb-2">
          <CardTitle class="text-sm font-medium text-muted-foreground">网络</CardTitle>
        </CardHeader>
        <CardContent class="space-y-3">
          <div class="grid grid-cols-2 gap-2">
            <div>
              <p class="text-xs text-muted-foreground">接收</p>
              <p class="text-xl font-semibold tabular-nums">
                {{ metrics ? formatRate(metrics.network.rx_bytes_per_sec) : '-' }}
              </p>
            </div>
            <div>
              <p class="text-xs text-muted-foreground">发送</p>
              <p class="text-xl font-semibold tabular-nums">
                {{ metrics ? formatRate(metrics.network.tx_bytes_per_sec) : '-' }}
              </p>
            </div>
          </div>
          <Sparkline :points="netRxSeries" :height="24" />
          <Sparkline :points="netTxSeries" :height="24" stroke="var(--chart-2)" />
          <p class="text-xs text-muted-foreground">
            累计 {{ metrics ? formatBytes(metrics.network.total_received_bytes) : '-' }} ↓ /
            {{ metrics ? formatBytes(metrics.network.total_transmitted_bytes) : '-' }} ↑
          </p>
        </CardContent>
      </Card>
    </div>

    <div class="mt-4 grid gap-4 lg:grid-cols-3">
      <!-- 磁盘 -->
      <Card class="lg:col-span-2">
        <CardHeader>
          <CardTitle class="text-sm font-medium">磁盘占用</CardTitle>
          <CardDescription>各挂载点容量使用情况。</CardDescription>
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
                （{{ formatPercent(diskPercent(disk)) }}）
              </span>
            </div>
            <Progress :model-value="diskPercent(disk)" class="h-2" />
          </div>
          <p v-if="!metrics" class="text-sm text-muted-foreground">
            等待数据…
          </p>
        </CardContent>
      </Card>

      <!-- 系统与进程信息 -->
      <Card>
        <CardHeader>
          <CardTitle class="text-sm font-medium">系统信息</CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 text-sm">
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">主机名</span>
            <span class="truncate">{{ metrics?.host_name ?? '-' }}</span>
          </div>
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">系统</span>
            <span class="truncate">{{ metrics?.os_version ?? '-' }}</span>
          </div>
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">内核</span>
            <span class="truncate">{{ metrics?.kernel_version ?? '-' }}</span>
          </div>
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">运行时长</span>
            <span class="tabular-nums">{{ metrics ? formatDuration(metrics.uptime_seconds) : '-' }}</span>
          </div>
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">进程数</span>
            <span class="tabular-nums">{{ metrics?.process_count ?? '-' }}</span>
          </div>
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">负载（1/5/15 分）</span>
            <span class="tabular-nums">
              {{ metrics ? `${metrics.load.one.toFixed(2)} / ${metrics.load.five.toFixed(2)} / ${metrics.load.fifteen.toFixed(2)}` : '-' }}
            </span>
          </div>
          <Separator class="my-2" />
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">服务 RSS</span>
            <span class="tabular-nums">{{ metrics ? formatBytes(metrics.server.rss_bytes) : '-' }}</span>
          </div>
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">服务运行时长</span>
            <span class="tabular-nums">{{ metrics ? formatDuration(metrics.server.uptime_seconds) : '-' }}</span>
          </div>
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">数据库大小</span>
            <span class="tabular-nums">{{ metrics?.pg ? formatBytes(metrics.pg.database_size_bytes) : '-' }}</span>
          </div>
          <div class="flex justify-between gap-2">
            <span class="text-muted-foreground">数据库连接</span>
            <span class="tabular-nums">{{ metrics?.pg?.active_connections ?? '-' }}</span>
          </div>
        </CardContent>
      </Card>
    </div>

    <!-- 历史趋势 -->
    <Card class="mt-4">
      <CardHeader class="flex-row items-center justify-between gap-4 space-y-0">
        <div>
          <CardTitle class="text-sm font-medium">历史趋势</CardTitle>
          <CardDescription>
            内存环形缓冲采样（10 秒间隔、保留 24 小时，服务重启清空）。
          </CardDescription>
        </div>
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
        <div v-else class="grid gap-6 md:grid-cols-3">
          <div class="space-y-1">
            <p class="text-xs text-muted-foreground">CPU 使用率（%）</p>
            <Sparkline :points="historyCpu" :max="100" :height="72" />
          </div>
          <div class="space-y-1">
            <p class="text-xs text-muted-foreground">内存使用率（%）</p>
            <Sparkline :points="historyMemory" :max="100" :height="72" />
          </div>
          <div class="space-y-1">
            <p class="text-xs text-muted-foreground">网络速率（RX / TX）</p>
            <Sparkline :points="historyNetRx" :max="historyNetMax" :height="72" />
            <Sparkline :points="historyNetTx" :max="historyNetMax" :height="72" stroke="var(--chart-2)" />
          </div>
        </div>
      </CardContent>
    </Card>
  </BasicPage>
</template>

<route lang="yaml">
meta:
  permission: settings:manage
</route>
