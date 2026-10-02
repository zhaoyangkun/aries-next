// 文章目录：客户端从渲染后正文（.article-content）提取 h2/h3 生成目录，
// 并为无 id 的标题补 toc-* 锚点（后端 comrak 未开启 heading id 且 ammonia 会剥离 id 属性）。
// ArticleToc（桌面侧栏）与 TocDrawer（移动端抽屉）共用本 composable。

export interface TocItem {
  id: string
  text: string
  level: 2 | 3
}

/** 标题元素的纯数据快照，供 buildTocItems 纯函数处理（也方便脱离 DOM 单测） */
export interface TocHeadingSnapshot {
  tagName: string
  textContent: string | null
  id: string
}

/**
 * 从标题快照生成目录项：保留已有 id，缺失或与前面重复的 id 回退为 `toc-${index}`（index 天然唯一）。
 */
export function buildTocItems(headings: TocHeadingSnapshot[]): TocItem[] {
  const used = new Set<string>()
  return headings.map((heading, index) => {
    let id = heading.id
    if (!id || used.has(id)) id = `toc-${index}`
    used.add(id)
    return {
      id,
      text: heading.textContent ?? '',
      level: heading.tagName === 'H2' ? 2 : 3,
    }
  })
}

export function useArticleToc() {
  const items = ref<TocItem[]>([])
  const activeId = ref('')
  let observer: IntersectionObserver | null = null
  let mutationObserver: MutationObserver | null = null

  async function extract() {
    await nextTick()
    const container = document.querySelector('.article-content')
    if (!container) {
      items.value = []
      return
    }

    const headings = Array.from(container.querySelectorAll('h2, h3'))
    if (headings.length === 0) {
      items.value = []
      return
    }

    items.value = buildTocItems(
      headings.map((el) => ({
        tagName: el.tagName,
        textContent: el.textContent,
        id: el.id,
      })),
    )
    // 把（可能新分配的）id 写回 DOM，锚点跳转与观察器都以最终 id 为准
    headings.forEach((el, index) => {
      const item = items.value[index]
      if (item) el.id = item.id
    })

    observer?.disconnect()
    observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) activeId.value = entry.target.id
        }
      },
      // 顶部避开吸顶导航，底部收窄使“当前位置”更接近阅读进度
      { rootMargin: '-96px 0px -70% 0px' },
    )
    for (const el of headings) observer.observe(el)
  }

  onMounted(async () => {
    await extract()
    // 客户端路由切换复用页面组件时正文 HTML 整体替换，监听 .article-content 子树变化重建目录
    // （id 写回是 attribute 变化不会触发；重建本身不改子树，不会自激循环）
    const container = document.querySelector('.article-content')
    if (!container) return
    let scheduled = false
    mutationObserver = new MutationObserver(() => {
      if (scheduled) return
      scheduled = true
      setTimeout(() => {
        scheduled = false
        extract()
      }, 0)
    })
    mutationObserver.observe(container, { childList: true, subtree: true })
  })

  onBeforeUnmount(() => {
    observer?.disconnect()
    mutationObserver?.disconnect()
  })

  return { items, activeId }
}
