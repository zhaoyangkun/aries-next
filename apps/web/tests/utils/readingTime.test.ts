import { describe, expect, it } from 'vitest'

import { readingTimeFromHtml, readingTimeFromText } from '../../app/utils/readingTime'

describe('readingTimeFromText', () => {
  it('counts CJK chars one by one and latin sequences as words', () => {
    // 100 中文字符 + 50 个英文词
    const chinese = '字'.repeat(100)
    const english = Array.from({ length: 50 }, (_, i) => `word${i}`).join(' ')
    const result = readingTimeFromText(`${chinese} ${english}`)
    expect(result.count).toBe(150)
    // 100/400 + 50/200 = 0.5 分钟，向上取整至少 1
    expect(result.minutes).toBe(1)
  })

  it('rounds minutes up with mixed content', () => {
    // 400 中文字符 = 1 分钟，200 英文词 = 1 分钟，合计 2 分钟
    const result = readingTimeFromText(`${'汉'.repeat(400)} ${'a'.repeat(200).replace(/(.{4})/g, '$1 ')}`)
    expect(result.minutes).toBeGreaterThanOrEqual(2)
  })

  it('returns at least 1 minute for empty or tiny input', () => {
    expect(readingTimeFromText('')).toEqual({ minutes: 1, count: 0 })
    expect(readingTimeFromText('短')).toEqual({ minutes: 1, count: 1 })
  })

  it('ignores whitespace and ascii punctuation', () => {
    const result = readingTimeFromText('  ，。！hello, world!  ')
    expect(result.count).toBe(5) // 3 个全角标点 + hello/world 两个词（半角逗号感叹号不计）
    expect(result.minutes).toBe(1)
  })
})

describe('readingTimeFromHtml', () => {
  it('strips tags and counts only visible text', () => {
    const html = '<h2>标题</h2><p>这是一段<strong>加粗</strong>文字。</p>'
    const result = readingTimeFromHtml(html)
    expect(result.count).toBe(11) // 标题2 + 正文9
    expect(result.minutes).toBe(1)
  })

  it('drops script and style blocks', () => {
    const html = '<p>正文</p><script>var long = "aaaa bbbb cccc dddd";</script><style>.a{color:red}</style>'
    const result = readingTimeFromHtml(html)
    expect(result.count).toBe(2)
  })

  it('returns the minimal estimate for null / undefined / empty html', () => {
    expect(readingTimeFromHtml(null)).toEqual({ minutes: 1, count: 0 })
    expect(readingTimeFromHtml(undefined)).toEqual({ minutes: 1, count: 0 })
    expect(readingTimeFromHtml('')).toEqual({ minutes: 1, count: 0 })
  })

  it('handles a realistic long article', () => {
    const paragraph = `<p>${'阅读时长估算的正文内容。'.repeat(100)}</p>`
    const result = readingTimeFromHtml(paragraph)
    expect(result.count).toBe(1200)
    expect(result.minutes).toBe(3) // 1200 / 400 = 3
  })
})
