import { describe, expect, it } from 'vitest'
import { validateCategoryName, validateLinkIconUrl, validateLinkTitle, validateLinkUrl } from './validation'

describe('validateLinkUrl', () => {
  it('accepts http and https urls', () => {
    expect(validateLinkUrl('https://example.com')).toBe('')
    expect(validateLinkUrl('http://example.com/path?q=1')).toBe('')
    // 首尾空白会先 trim 再校验。
    expect(validateLinkUrl('  https://example.com  ')).toBe('')
  })

  it('rejects empty, malformed and non-http protocols', () => {
    expect(validateLinkUrl('')).toBe('请输入链接 URL')
    expect(validateLinkUrl('  ')).toBe('请输入链接 URL')
    expect(validateLinkUrl('not-a-url')).toBe('链接 URL 格式不正确')
    expect(validateLinkUrl('ftp://example.com')).toBe('链接 URL 仅支持 http 或 https 协议')
    expect(validateLinkUrl('javascript:alert(1)')).toBe('链接 URL 仅支持 http 或 https 协议')
  })
})

describe('validateLinkIconUrl', () => {
  it('allows empty icon url but validates provided ones', () => {
    expect(validateLinkIconUrl('')).toBe('')
    expect(validateLinkIconUrl('  ')).toBe('')
    expect(validateLinkIconUrl('https://example.com/icon.png')).toBe('')
    expect(validateLinkIconUrl('ftp://example.com/icon.png')).toBe('链接 URL 仅支持 http 或 https 协议')
  })
})

describe('validateLinkTitle / validateCategoryName', () => {
  it('require non-blank values', () => {
    expect(validateLinkTitle('')).toBe('请输入友链名称')
    expect(validateLinkTitle('Vue')).toBe('')
    expect(validateCategoryName(' ')).toBe('请输入分类名称')
    expect(validateCategoryName('技术')).toBe('')
  })
})
