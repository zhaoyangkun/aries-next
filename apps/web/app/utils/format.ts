// 日期格式化统一手写，避免 SSR（Node）与浏览器因 ICU/Locale 差异导致 Hydration 不一致
export function formatDate(iso: string | null | undefined): string {
  if (!iso) return ''
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  return `${date.getFullYear()}年${date.getMonth() + 1}月${date.getDate()}日`
}

export function formatDateShort(iso: string | null | undefined): string {
  if (!iso) return ''
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return ''
  const mm = String(date.getMonth() + 1).padStart(2, '0')
  const dd = String(date.getDate()).padStart(2, '0')
  return `${date.getFullYear()}-${mm}-${dd}`
}

// 文章卡片左侧日期块：日与年月分行展示（对齐旧版 xue 主题）；手写格式化以保证 SSR 与客户端一致
export function formatDateParts(
  iso: string | null | undefined,
): { day: string, yearMonth: string } | null {
  if (!iso) return null
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return null
  return {
    day: String(date.getDate()).padStart(2, '0'),
    yearMonth: `${date.getFullYear()}年${String(date.getMonth() + 1).padStart(2, '0')}月`,
  }
}

/**
 * 站点运行天数：`created_at`（RFC 3339）到 now 的整天数，按 UTC 毫秒差取整避免时区/DST 影响。
 * 至少返回 1 天；created_at 缺失或非法时返回 0（调用方据此隐藏展示）。
 * now 可注入固定值以保证单测与 SSR 行为确定。
 */
export function siteRunDays(createdAt: string | null | undefined, now: Date = new Date()): number {
  if (!createdAt) return 0
  const start = new Date(createdAt).getTime()
  if (Number.isNaN(start)) return 0
  return Math.max(1, Math.floor((now.getTime() - start) / 86_400_000))
}

/**
 * 媒体缩略图 URL：仅本站托管（/api/media/files 前缀）的图片追加 `?w=` 请求按需缩略图；
 * 外部/CDN 地址（含绝对 URL）无法走本站缩放，原样返回。
 */
export function thumbUrl(url: string | null | undefined, width: number): string {
  if (!url || !url.startsWith('/api/media/files/')) return url ?? ''
  return `${url}?w=${width}`
}

/**
 * 响应式 srcset：本站托管图片按给定宽度档生成 `?w=` 描述符列表（后端支持 16–1200 按需缩放、
   生成后落盘缓存且 immutable）；外部地址无法缩放返回 undefined（调用方省略 srcset 属性）。
 */
export function thumbSrcset(url: string | null | undefined, widths: number[]): string | undefined {
  if (!url || !url.startsWith('/api/media/files/')) return undefined
  return widths.map((width) => `${url}?w=${width} ${width}w`).join(', ')
}
