import { defineStore } from 'pinia'

export interface PageTab {
  /** 以 route.path 为唯一键（忽略 query），同一路径只保留一个标签。 */
  path: string
  title: string
  closable: boolean
}

// 多标签导航：记录访问过的页面，支持关闭/关闭其他/关闭全部；sessionStorage 持久化，刷新不丢。
export const useTabsStore = defineStore('page-tabs', () => {
  const tabs = ref<PageTab[]>([])

  function open(tab: PageTab) {
    const existing = tabs.value.find(item => item.path === tab.path)
    if (existing) {
      existing.title = tab.title
      return
    }
    tabs.value.push(tab)
  }

  /** 关闭指定标签；返回关闭后应跳转的相邻标签路径（当前页被关时由调用方跳转）。 */
  function close(path: string): string | undefined {
    const index = tabs.value.findIndex(item => item.path === path)
    if (index === -1)
      return undefined
    tabs.value.splice(index, 1)
    const next = tabs.value[index] ?? tabs.value[index - 1]
    return next?.path
  }

  function closeOthers(path: string) {
    tabs.value = tabs.value.filter(item => item.path === path || !item.closable)
  }

  function closeAll() {
    tabs.value = tabs.value.filter(item => !item.closable)
  }

  return {
    tabs,
    open,
    close,
    closeOthers,
    closeAll,
  }
}, {
  persist: true,
})
