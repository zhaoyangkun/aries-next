import { describe, expect, it } from 'vitest'

import { buildTocItems } from '../../app/composables/useArticleToc'

// buildTocItems 是纯函数：输入标题元素快照（tagName/textContent/id），输出目录项。
// 锚点 id 分配与去重逻辑脱离 DOM 单测；IntersectionObserver 部分留在组件层。

describe('buildTocItems', () => {
  it('extracts h2/h3 with text and level', () => {
    const items = buildTocItems([
      { tagName: 'H2', textContent: '第一节', id: '' },
      { tagName: 'H3', textContent: '小节', id: '' },
      { tagName: 'H2', textContent: '第二节', id: '' },
    ])
    expect(items).toEqual([
      { id: 'toc-0', text: '第一节', level: 2 },
      { id: 'toc-1', text: '小节', level: 3 },
      { id: 'toc-2', text: '第二节', level: 2 },
    ])
  })

  it('keeps existing ids', () => {
    const items = buildTocItems([{ tagName: 'H2', textContent: '标题', id: 'custom-id' }])
    expect(items[0].id).toBe('custom-id')
  })

  it('falls back to toc-<index> when an id is duplicated', () => {
    const items = buildTocItems([
      { tagName: 'H2', textContent: '甲', id: 'dup' },
      { tagName: 'H2', textContent: '乙', id: 'dup' },
      { tagName: 'H2', textContent: '丙', id: '' },
    ])
    expect(items.map((item) => item.id)).toEqual(['dup', 'toc-1', 'toc-2'])
  })

  it('handles empty textContent as empty string', () => {
    const items = buildTocItems([{ tagName: 'H2', textContent: null, id: '' }])
    expect(items[0].text).toBe('')
  })

  it('generates unique fallback ids for many headings', () => {
    const headings = Array.from({ length: 20 }, (_, i) => ({
      tagName: 'H3',
      textContent: `标题 ${i}`,
      id: '',
    }))
    const ids = buildTocItems(headings).map((item) => item.id)
    expect(new Set(ids).size).toBe(20)
  })
})
