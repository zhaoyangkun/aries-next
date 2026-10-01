import { describe, expect, it } from 'vitest'

import { parseKeywords, splitByKeywords } from '../../app/utils/highlight'

describe('parseKeywords', () => {
  it('splits on whitespace, dedupes and drops blanks', () => {
    expect(parseKeywords(' rust  所有权\trust ')).toEqual(['rust', '所有权'])
  })

  it('returns empty array for blank input', () => {
    expect(parseKeywords('')).toEqual([])
    expect(parseKeywords(undefined)).toEqual([])
    expect(parseKeywords('   ')).toEqual([])
  })
})

describe('splitByKeywords', () => {
  it('marks keyword segments and keeps the rest untouched', () => {
    expect(splitByKeywords('学习 rust 所有权', ['rust'])).toEqual([
      { text: '学习 ', match: false },
      { text: 'rust', match: true },
      { text: ' 所有权', match: false },
    ])
  })

  it('is case-insensitive but preserves original casing', () => {
    expect(splitByKeywords('Rust 语言', ['rust'])).toEqual([
      { text: 'Rust', match: true },
      { text: ' 语言', match: false },
    ])
  })

  it('supports multiple keywords and longest-first matching', () => {
    const segments = splitByKeywords('rustacean 与 rust', ['rust', 'rustacean'])
    expect(segments.filter((s) => s.match).map((s) => s.text)).toEqual(['rustacean', 'rust'])
  })

  it('returns the whole text as a single non-match segment without keywords', () => {
    expect(splitByKeywords('任何文本', [])).toEqual([{ text: '任何文本', match: false }])
  })

  it('escapes regex special characters in keywords', () => {
    expect(splitByKeywords('a+b (c)', ['a+b', '(c)'])).toEqual([
      { text: 'a+b', match: true },
      { text: ' ', match: false },
      { text: '(c)', match: true },
    ])
  })
})
