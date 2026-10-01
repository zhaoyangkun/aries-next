<script setup lang="ts">
import type { PublicArchiveMonth } from '~/composables/usePublicApi'

const { data: archives, status } = await usePublicApi<PublicArchiveMonth[]>(
  'archive-list',
  '/archives',
)

// 后端返回按 year/month 倒序的分组，这里再按年份聚合成时间线
const years = computed(() => {
  const map = new Map<number, PublicArchiveMonth[]>()
  for (const month of archives.value ?? []) {
    const list = map.get(month.year) ?? []
    list.push(month)
    map.set(month.year, list)
  }
  return Array.from(map.entries()).map(([year, months]) => ({ year, months }))
})

usePageSeo({
  title: '归档',
  description: '按时间浏览全部文章',
})
useCanonical('/archives')
</script>

<template>
  <PageHero title="归档" label="按时间浏览" />

  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
  <div v-else-if="years.length === 0" class="empty-articles">暂无归档</div>

  <div v-else class="mt-10 space-y-12">
    <section v-for="group in years" :key="group.year" :aria-label="`${group.year} 年`">
      <h2 class="m-0 text-2xl font-semibold">{{ group.year }}</h2>
      <div v-for="month in group.months" :key="month.month" class="mt-6">
        <h3 class="m-0 text-sm font-medium text-muted-foreground">
          {{ month.year }} 年 {{ month.month }} 月（{{ month.count }} 篇）
        </h3>
        <ul class="mt-3 space-y-2 border-l pl-5">
          <li v-for="item in month.articles" :key="item.slug" class="list-none">
            <div class="flex items-baseline gap-3">
              <time
                class="shrink-0 text-xs tabular-nums text-muted-foreground"
                :datetime="item.published_at"
              >{{ formatDateShort(item.published_at) }}</time>
              <NuxtLink
                :to="`/articles/${item.slug}`"
                class="min-w-0 truncate no-underline hover:text-primary"
              >{{ item.title }}</NuxtLink>
            </div>
          </li>
        </ul>
      </div>
    </section>
  </div>
</template>
