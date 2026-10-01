import { describe, expect, it } from 'vitest'
import {
  validateDisplayName,
  validateAvatarUrl,
  validateEmail,
  validatePassword,
  validateRequired,
  validateUsername,
} from './validation'

describe('authentication form validation', () => {
  it('requires a non-blank value with the field name in the message', () => {
    expect(validateRequired('', '用户名')).toBe('请输入用户名')
    expect(validateRequired('  ', '邮箱')).toBe('请输入邮箱')
    expect(validateRequired('owner', '用户名')).toBe('')
  })

  it('matches the username policy', () => {
    expect(validateUsername('owner')).toBe('')
    expect(validateUsername('a')).not.toBe('')
    expect(validateUsername('owner name')).not.toBe('')
  })

  it('enforces the 3 to 30 character username boundary', () => {
    // 边界放行：恰好 3 与 30 个字符，允许短横线与下划线。
    expect(validateUsername('ab_')).toBe('')
    expect(validateUsername('a-'.padEnd(30, '1'))).toBe('')
    // 边界拒绝：2 与 31 个字符。
    expect(validateUsername('ab')).not.toBe('')
    expect(validateUsername('a'.repeat(31))).not.toBe('')
  })

  it('rejects malformed email addresses', () => {
    expect(validateEmail('owner@example.com')).toBe('')
    expect(validateEmail('owner@example')).not.toBe('')
    expect(validateEmail('owner @example.com')).not.toBe('')
    // 多个 @、空本地段也算格式错误。
    expect(validateEmail('a@b@example.com')).not.toBe('')
    expect(validateEmail('@example.com')).not.toBe('')
  })

  it('enforces the 254 character email boundary', () => {
    // 恰好 254 个字符放行，255 个字符拒绝。
    const domain = `${'b'.repeat(249)}.cd`
    expect(validateEmail(`a@${domain}`)).toBe('')
    expect(validateEmail(`a@${domain}x`)).not.toBe('')
  })

  it('matches the password policy', () => {
    expect(validatePassword('reliable-pass-2026')).toBe('')
    expect(validatePassword('only-letters')).not.toBe('')
    expect(validatePassword('short1')).not.toBe('')
  })

  it('enforces the 10 to 128 character password boundary', () => {
    // 边界放行：恰好 10 与 128 个字符；Unicode 字母（如中文）也满足字母要求。
    expect(validatePassword('a'.repeat(9) + '1')).toBe('')
    expect(validatePassword('a'.repeat(127) + '1')).toBe('')
    expect(validatePassword('密码密码密码密码1234')).toBe('')
    // 边界拒绝：9 与 129 个字符。
    expect(validatePassword('a'.repeat(8) + '1')).not.toBe('')
    expect(validatePassword('a'.repeat(128) + '1')).not.toBe('')
  })

  it('limits display names to sixty characters', () => {
    expect(validateDisplayName('Aries Owner')).toBe('')
    expect(validateDisplayName('')).not.toBe('')
    // 恰好 60 个字符放行，61 个拒绝。
    expect(validateDisplayName('名'.repeat(60))).toBe('')
    expect(validateDisplayName('名'.repeat(61))).not.toBe('')
  })

  it('only accepts optional HTTP avatar URLs', () => {
    expect(validateAvatarUrl('')).toBe('')
    expect(validateAvatarUrl('https://example.com/avatar.png')).toBe('')
    expect(validateAvatarUrl('file:///avatar.png')).not.toBe('')
    expect(validateAvatarUrl('not-a-url')).not.toBe('')
    // 恰好 2048 个字符放行，2049 个拒绝。
    expect(validateAvatarUrl(`https://example.com/${'a'.repeat(2028)}`)).toBe('')
    expect(validateAvatarUrl(`https://example.com/${'a'.repeat(2029)}`)).not.toBe('')
  })
})
