// 友链表单校验：返回 '' 表示通过，否则返回中文错误文案。
// URL 约束与 Backend core 校验保持一致：仅允许 http/https。

export function validateLinkUrl(url: string): string {
  const value = url.trim()
  if (!value) return '请输入链接 URL'
  let parsed: URL
  try {
    parsed = new URL(value)
  }
  catch {
    return '链接 URL 格式不正确'
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:')
    return '链接 URL 仅支持 http 或 https 协议'
  return ''
}

// icon_url 可选，留空通过；填写时同样要求 http/https。
export function validateLinkIconUrl(iconUrl: string): string {
  const value = iconUrl.trim()
  if (!value) return ''
  return validateLinkUrl(value)
}

export function validateLinkTitle(title: string): string {
  if (!title.trim()) return '请输入友链名称'
  return ''
}

export function validateCategoryName(name: string): string {
  if (!name.trim()) return '请输入分类名称'
  return ''
}
