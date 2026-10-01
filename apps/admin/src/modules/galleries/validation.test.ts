import { describe, expect, it } from 'vitest'
import { validateGalleryCategoryId, validateGallerySlug, validateGalleryTitle } from './validation'

describe('validateGallerySlug', () => {
  it('accepts lowercase slugs with hyphens', () => {
    expect(validateGallerySlug('travel-2026')).toBe('')
  })

  it('rejects empty or malformed slugs', () => {
    expect(validateGallerySlug('')).toBe('请输入 Slug')
    expect(validateGallerySlug('  ')).toBe('请输入 Slug')
    expect(validateGallerySlug('Travel')).toContain('小写字母')
    // 连字符不能出现在开头或结尾。
    expect(validateGallerySlug('-travel')).toContain('小写字母')
    expect(validateGallerySlug('travel-')).toContain('小写字母')
  })

  it('enforces the 160 character slug boundary', () => {
    // 恰好 160 个字符放行，161 个拒绝。
    expect(validateGallerySlug('a'.repeat(160))).toBe('')
    expect(validateGallerySlug('a'.repeat(161))).toBe('Slug 不能超过 160 个字符')
  })
})

describe('validateGalleryTitle / validateGalleryCategoryId', () => {
  it('require title and category', () => {
    expect(validateGalleryTitle('')).toBe('请输入图库标题')
    expect(validateGalleryTitle('旅行')).toBe('')
    expect(validateGalleryCategoryId(null)).toBe('请选择所属分类')
    expect(validateGalleryCategoryId(2)).toBe('')
  })
})
