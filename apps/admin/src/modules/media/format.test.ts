import { describe, expect, it } from 'vitest'
import { formatFileSize, formatMediaTime, providerText, usageTargetText } from './format'

describe('formatFileSize', () => {
  it('keeps bytes below 1KB as-is', () => {
    expect(formatFileSize(0)).toBe('0 B')
    expect(formatFileSize(1023)).toBe('1023 B')
  })

  it('switches to KB and MB at the 1024 boundaries', () => {
    expect(formatFileSize(1024)).toBe('1.0 KB')
    expect(formatFileSize(1024 * 1024 - 1)).toBe('1024.0 KB')
    expect(formatFileSize(1024 * 1024)).toBe('1.0 MB')
    expect(formatFileSize(5 * 1024 * 1024)).toBe('5.0 MB')
  })
})

describe('formatMediaTime', () => {
  it('formats valid timestamps in zh-CN and falls back to a dash', () => {
    // 不带时区的本地时间，避免断言结果受测试机时区影响。
    expect(formatMediaTime('2026-08-05T09:30:00')).toBe('2026/08/05 09:30')
    expect(formatMediaTime('not-a-date')).toBe('-')
    expect(formatMediaTime('')).toBe('-')
  })
})

describe('providerText / usageTargetText', () => {
  it('maps known providers and defaults the rest to 本地', () => {
    expect(providerText('s3')).toBe('S3')
    expect(providerText('legacy_url')).toBe('旧站链接')
    expect(providerText('local')).toBe('本地')
    expect(providerText('unknown')).toBe('本地')
  })

  it('maps known usage targets and passes unknown targets through', () => {
    expect(usageTargetText('article_cover')).toBe('文章封面')
    expect(usageTargetText('article_content')).toBe('文章正文')
    expect(usageTargetText('gallery_item')).toBe('相册')
    // 未知 target_type 原样展示，避免掩盖 Backend 新增类型。
    expect(usageTargetText('page_cover')).toBe('page_cover')
  })
})
