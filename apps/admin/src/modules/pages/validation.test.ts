import { describe, expect, it } from 'vitest'
import { validatePageSlug, validatePageTitle } from './validation'

describe('validatePageSlug', () => {
  it('accepts lowercase slugs with hyphens', () => {
    expect(validatePageSlug('about')).toBe('')
    expect(validatePageSlug('my-page-2')).toBe('')
    // 恰好 160 个字符放行。
    expect(validatePageSlug('a'.repeat(160))).toBe('')
  })

  it('rejects empty, overlong or malformed slugs', () => {
    expect(validatePageSlug('')).toBe('请输入 Slug')
    expect(validatePageSlug('a'.repeat(161))).toBe('Slug 不能超过 160 个字符')
    expect(validatePageSlug('About')).toContain('小写字母')
    expect(validatePageSlug('-about')).toContain('小写字母')
    expect(validatePageSlug('about-')).toContain('小写字母')
    expect(validatePageSlug('about page')).toContain('小写字母')
    expect(validatePageSlug('关于')).toContain('小写字母')
  })
})

describe('validatePageTitle', () => {
  it('requires a non-blank title', () => {
    expect(validatePageTitle('')).toBe('请输入页面标题')
    expect(validatePageTitle('  ')).toBe('请输入页面标题')
    expect(validatePageTitle('关于')).toBe('')
  })
})
