<script setup lang="ts">
// 客户端从渲染后正文的 h2/h3 生成侧边目录
// 后端 comrak 未开启 heading id 且 ammonia 会剥离 id 属性，这里在挂载后补齐锚点 id
interface TocItem {
  id: string
  text: string
  level: 2 | 3
}

const items = ref<TocItem[]>([])
const activeId = ref('')
let observer: IntersectionObserver | null = null

onMounted(async () => {
  await nextTick()
  const container = document.querySelector('.article-content')
  if (!container) return

  const headings = Array.from(container.querySelectorAll('h2, h3'))
  if (headings.length === 0) return

  items.value = headings.map((el, index) => {
    if (!el.id) el.id = `toc-${index}`
    return { id: el.id, text: el.textContent ?? '', level: el.tagName === 'H2' ? 2 : 3 }
  })

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
})

onBeforeUnmount(() => {
  observer?.disconnect()
})
</script>

<template>
  <nav v-if="items.length > 0" aria-label="文章目录" class="text-sm">
    <p class="m-0 text-xs font-medium tracking-widest text-primary uppercase">目录</p>
    <ul class="mt-3 space-y-2 border-l border-border pl-0">
      <li v-for="item in items" :key="item.id" class="list-none">
        <a
          :href="`#${item.id}`"
          class="-ml-px block border-l-2 no-underline transition-colors"
          :class="[
            item.level === 3 ? 'pl-5' : 'pl-3',
            activeId === item.id
              ? 'border-primary font-medium text-title'
              : 'border-transparent text-muted-foreground hover:text-foreground',
          ]"
        >{{ item.text }}</a>
      </li>
    </ul>
  </nav>
</template>
