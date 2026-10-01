<script setup lang="ts">
import type { PublicLink } from '~/composables/usePublicApi'

const { data: links, status } = await usePublicApi<PublicLink[]>('link-list', '/links')

// 按分类分组展示：保持接口返回顺序（sort_order 已排好），未分类固定排最后
const groups = computed(() => {
  const UNCATEGORIZED = '未分类'
  const map = new Map<string, PublicLink[]>()
  for (const link of links.value ?? []) {
    const name = link.category_name || UNCATEGORIZED
    const list = map.get(name) ?? []
    list.push(link)
    map.set(name, list)
  }
  const entries = Array.from(map.entries())
  entries.sort(([a], [b]) => (a === UNCATEGORIZED ? 1 : b === UNCATEGORIZED ? -1 : 0))
  return entries.map(([name, items]) => ({ name, items }))
})

usePageSeo({
  title: '友情链接',
  description: '友情链接与常去的站点',
})
useCanonical('/links')
</script>

<template>
  <PageHero title="友情链接" label="邻居们" />

  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
  <div v-else-if="groups.length === 0" class="empty-articles">暂无友情链接</div>

  <div v-else class="mt-10 space-y-12">
    <section v-for="group in groups" :key="group.name" :aria-label="group.name">
      <h2 class="m-0 text-xl font-semibold">{{ group.name }}</h2>
      <div class="mt-5 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <a
          v-for="link in group.items"
          :key="link.url"
          :href="link.url"
          target="_blank"
          rel="noopener"
          class="flex items-start gap-4 rounded-lg border p-4 no-underline transition-colors hover:bg-muted"
        >
          <img
            v-if="link.icon_url"
            :src="link.icon_url"
            :alt="`${link.title} 图标`"
            width="64"
            height="64"
            loading="lazy"
            class="h-16 w-16 shrink-0 rounded-md object-cover"
          >
          <!-- 无图标时用首字符占位，保持卡片高度一致 -->
          <span
            v-else
            class="grid h-16 w-16 shrink-0 place-items-center rounded-md bg-muted text-2xl font-semibold text-muted-foreground"
            aria-hidden="true"
          >{{ link.title.charAt(0) }}</span>
          <span class="min-w-0">
            <span class="block truncate font-medium text-foreground">{{ link.title }}</span>
            <span v-if="link.description" class="mt-1 block text-sm text-muted-foreground line-clamp-2">
              {{ link.description }}
            </span>
          </span>
        </a>
      </div>
    </section>
  </div>
</template>
