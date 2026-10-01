// 自定义页面表单校验：返回 '' 表示通过，否则返回中文错误文案。
// Slug 规则：小写字母/数字/连字符，1–160 字符，与 Backend slug 约束保持一致。

const SLUG_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/

export function validatePageSlug(slug: string): string {
  const value = slug.trim()
  if (!value) return '请输入 Slug'
  if (value.length > 160) return 'Slug 不能超过 160 个字符'
  if (!SLUG_PATTERN.test(value))
    return 'Slug 只能包含小写字母、数字和连字符，且不能以连字符开头或结尾'
  return ''
}

export function validatePageTitle(title: string): string {
  if (!title.trim()) return '请输入页面标题'
  return ''
}
