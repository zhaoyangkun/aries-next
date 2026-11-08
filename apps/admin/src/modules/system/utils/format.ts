/** 字节数格式化为人类可读（B/KB/MB/GB/TB，二进制进位，与系统工具显示习惯一致）。 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0)
    return '-'
  if (bytes < 1024)
    return `${Math.round(bytes)} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 'B'
  for (const next of units) {
    if (value < 1024)
      break
    value /= 1024
    unit = next
  }
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${unit}`
}

/** 速率格式化：`formatBytes` + `/s`。 */
export function formatRate(bytesPerSec: number): string {
  return `${formatBytes(bytesPerSec)}/s`
}

/** 速率按网络带宽惯例格式化（bit/s：Kbps/Mbps/Gbps，1 Mbps = 10^6 bit/s）。 */
export function formatBitrate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec < 0)
    return '-'
  const mbps = bytesPerSec * 8 / 1_000_000
  if (mbps >= 1000)
    return `${(mbps / 1000).toFixed(2)} Gbps`
  if (mbps >= 1)
    return `${mbps.toFixed(1)} Mbps`
  return `${Math.max(0, mbps * 1000).toFixed(0)} Kbps`
}

/** 用量阈值档位：<70% 正常、70–90% 注意、≥90% 告警。 */
export type UsageTone = 'ok' | 'warning' | 'critical'

/** 使用率 → 阈值档位，用于数字与进度条配色。 */
export function usageTone(percent: number): UsageTone {
  if (!Number.isFinite(percent))
    return 'ok'
  if (percent >= 90)
    return 'critical'
  if (percent >= 70)
    return 'warning'
  return 'ok'
}

/** 阈值档位 → 进度条填充色（作用于 shadcn Progress 的 indicator 子元素）。 */
export const toneBarClass: Record<UsageTone, string> = {
  ok: '[&>div]:bg-green-500',
  warning: '[&>div]:bg-amber-500',
  critical: '[&>div]:bg-destructive',
}

/** 阈值档位 → 文本色。 */
export const toneTextClass: Record<UsageTone, string> = {
  ok: 'text-green-600 dark:text-green-400',
  warning: 'text-amber-600 dark:text-amber-400',
  critical: 'text-destructive',
}

/** 阈值档位 → 实心填充色（heatmap 竖条 / 徽标底）。 */
export const toneSolidClass: Record<UsageTone, string> = {
  ok: 'bg-green-500/80',
  warning: 'bg-amber-500/80',
  critical: 'bg-destructive/80',
}

/** 阈值档位 → 圆环描边色（GaugeRing 进度弧）。 */
export const toneRingClass: Record<UsageTone, string> = {
  ok: 'stroke-green-500',
  warning: 'stroke-amber-500',
  critical: 'stroke-destructive',
}

/** 秒数格式化为中文时长（系统 uptime / 进程运行时长展示用）。 */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0)
    return '-'
  const total = Math.floor(seconds)
  const days = Math.floor(total / 86400)
  const hours = Math.floor((total % 86400) / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const secs = total % 60
  if (days > 0)
    return hours > 0 ? `${days} 天 ${hours} 小时` : `${days} 天`
  if (hours > 0)
    return minutes > 0 ? `${hours} 小时 ${minutes} 分` : `${hours} 小时`
  if (minutes > 0)
    return secs > 0 ? `${minutes} 分 ${secs} 秒` : `${minutes} 分`
  return `${secs} 秒`
}

/** 使用率数字格式化：1 位小数，范围外值钳位到 0–100。 */
export function formatPercent(percent: number): string {
  if (!Number.isFinite(percent))
    return '-'
  return `${Math.min(100, Math.max(0, percent)).toFixed(1)}%`
}
