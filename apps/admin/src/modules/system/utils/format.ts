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
