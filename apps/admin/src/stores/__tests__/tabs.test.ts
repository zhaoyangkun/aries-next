import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it } from 'vitest'

import { useTabsStore } from '../tabs'

describe('useTabsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('dedupes tabs by path and refreshes the title on revisit', () => {
    const store = useTabsStore()

    store.open({ path: '/articles', title: '文章', closable: true })
    store.open({ path: '/articles', title: '文章列表', closable: true })
    store.open({ path: '/media', title: '媒体库', closable: true })

    expect(store.tabs).toHaveLength(2)
    expect(store.tabs[0].title).toBe('文章列表')
  })

  it('returns the adjacent tab when closing one', () => {
    const store = useTabsStore()
    store.open({ path: '/dashboard', title: '概览', closable: false })
    store.open({ path: '/articles', title: '文章', closable: true })
    store.open({ path: '/media', title: '媒体库', closable: true })

    // 关闭中间的标签，相邻右侧标签顶上。
    expect(store.close('/articles')).toBe('/media')
    expect(store.tabs.map(tab => tab.path)).toEqual(['/dashboard', '/media'])

    // 关闭末尾标签，回落到左侧标签。
    expect(store.close('/media')).toBe('/dashboard')
  })

  it('closeOthers keeps the current and pinned tabs, closeAll keeps pinned only', () => {
    const store = useTabsStore()
    store.open({ path: '/dashboard', title: '概览', closable: false })
    store.open({ path: '/articles', title: '文章', closable: true })
    store.open({ path: '/media', title: '媒体库', closable: true })

    store.closeOthers('/articles')
    expect(store.tabs.map(tab => tab.path)).toEqual(['/dashboard', '/articles'])

    store.closeAll()
    expect(store.tabs.map(tab => tab.path)).toEqual(['/dashboard'])
  })
})
