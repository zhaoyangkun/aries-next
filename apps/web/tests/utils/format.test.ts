import { describe, expect, it } from 'vitest'

import { formatDate, formatDateParts, formatDateShort, thumbUrl } from '../../app/utils/format'

// 用不带时区的本地时间字符串构造用例，避免 UTC 解析在不同时区下日期偏移导致断言不稳定
describe('formatDate', () => {
  it('should format a valid ISO date as 年月日', () => {
    expect(formatDate('2026-09-05T10:30:00')).toBe('2026年9月5日')
  })

  it('should not pad single-digit month and day', () => {
    expect(formatDate('2026-01-02T00:00:00')).toBe('2026年1月2日')
  })

  it('should return empty string for null / undefined / empty input', () => {
    expect(formatDate(null)).toBe('')
    expect(formatDate(undefined)).toBe('')
    expect(formatDate('')).toBe('')
  })

  it('should return empty string for an invalid date string', () => {
    expect(formatDate('not-a-date')).toBe('')
  })
})

describe('formatDateShort', () => {
  it('should format a valid ISO date as YYYY-MM-DD with zero padding', () => {
    expect(formatDateShort('2026-09-05T10:30:00')).toBe('2026-09-05')
    expect(formatDateShort('2026-01-02T00:00:00')).toBe('2026-01-02')
  })

  it('should return empty string for null / undefined / empty input', () => {
    expect(formatDateShort(null)).toBe('')
    expect(formatDateShort(undefined)).toBe('')
    expect(formatDateShort('')).toBe('')
  })

  it('should return empty string for an invalid date string', () => {
    expect(formatDateShort('not-a-date')).toBe('')
  })
})

describe('formatDateParts', () => {
  it('should split the date into padded day and year-month parts', () => {
    expect(formatDateParts('2026-09-05T10:30:00')).toEqual({ day: '05', yearMonth: '2026年09月' })
  })

  it('should pad two-digit values without change', () => {
    expect(formatDateParts('2026-12-25T00:00:00')).toEqual({ day: '25', yearMonth: '2026年12月' })
  })

  it('should return null for null / undefined / empty input', () => {
    expect(formatDateParts(null)).toBeNull()
    expect(formatDateParts(undefined)).toBeNull()
    expect(formatDateParts('')).toBeNull()
  })

  it('should return null for an invalid date string', () => {
    expect(formatDateParts('not-a-date')).toBeNull()
  })
})

describe('thumbUrl', () => {
  it('appends w param to locally hosted media urls', () => {
    expect(thumbUrl('/api/media/files/2026/10/cover.png', 480)).toBe(
      '/api/media/files/2026/10/cover.png?w=480',
    )
  })

  it('keeps external and absolute urls untouched', () => {
    expect(thumbUrl('https://cdn.example.com/cover.png', 480)).toBe('https://cdn.example.com/cover.png')
    expect(thumbUrl('/other/path.png', 480)).toBe('/other/path.png')
  })

  it('returns empty string for null / undefined', () => {
    expect(thumbUrl(null, 480)).toBe('')
    expect(thumbUrl(undefined, 480)).toBe('')
  })
})
