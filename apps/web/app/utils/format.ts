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
