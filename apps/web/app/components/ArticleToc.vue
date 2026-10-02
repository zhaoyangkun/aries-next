<script setup lang="ts">
// 桌面端（xl 侧栏）文章目录：提取逻辑在 useArticleToc（与移动端 TocDrawer 共用），
// 这里只负责把「当前文章是否存在目录」同步给浮动工具栏（hasToc 供其决定是否展示目录按钮）
const { items, activeId } = useArticleToc()
const { hasToc } = useTocDrawer()

watch(
  items,
  (list) => {
    hasToc.value = list.length > 0
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  hasToc.value = false
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
