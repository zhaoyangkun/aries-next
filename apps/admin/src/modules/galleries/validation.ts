// 图库表单校验：返回 '' 表示通过，否则返回中文错误文案。
// Slug 规则与 Backend slug 约束保持一致：小写字母/数字/连字符，1–160 字符。

const SLUG_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/

export function validateGallerySlug(slug: string): string {
  const value = slug.trim()
  if (!value) return '请输入 Slug'
  if (value.length > 160) return 'Slug 不能超过 160 个字符'
  if (!SLUG_PATTERN.test(value))
    return 'Slug 只能包含小写字母、数字和连字符，且不能以连字符开头或结尾'
  return ''
}

export function validateGalleryTitle(title: string): string {
  if (!title.trim()) return '请输入图库标题'
  return ''
}

export function validateGalleryCategoryId(categoryId: number | null): string {
  if (categoryId === null) return '请选择所属分类'
  return ''
}
