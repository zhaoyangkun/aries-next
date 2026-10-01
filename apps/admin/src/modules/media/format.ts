// 媒体相关的展示格式化工具，MediaView 与 AppMediaPicker 复用。
export function formatFileSize(sizeBytes: number) {
  if (sizeBytes < 1024) return `${sizeBytes} B`
  if (sizeBytes < 1024 * 1024) return `${(sizeBytes / 1024).toFixed(1)} KB`
  return `${(sizeBytes / (1024 * 1024)).toFixed(1)} MB`
}

export function formatMediaTime(value: string) {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return '-'
  return new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date)
}

export function providerText(provider: string) {
  if (provider === 's3') return 'S3'
  if (provider === 'legacy_url') return '旧站链接'
  return '本地'
}

export function usageTargetText(targetType: string) {
  if (targetType === 'article_cover') return '文章封面'
  if (targetType === 'article_content') return '文章正文'
  if (targetType === 'gallery_item') return '相册'
  return targetType
}
