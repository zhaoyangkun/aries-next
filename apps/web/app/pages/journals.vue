<script setup lang="ts">
import type { PublicJournalPage } from '~/composables/usePublicApi'

const route = useRoute()
const page = computed(() => Math.max(1, Number(route.query.page) || 1))

const { data, status } = await usePublicApi<PublicJournalPage>('journal-list', '/journals', {
  query: { page },
  watch: [page],
})

// 分页页标题带上页码，保证各页 Title 唯一
usePageSeo({
  title: () => (page.value > 1 ? `日志 - 第 ${page.value} 页` : '日志'),
  description: '随手记录的只言片语',
})
useCanonical(computed(() => (page.value > 1 ? `/journals?page=${page.value}` : '/journals')))
</script>

<template>
  <PageHero title="日志" label="只言片语" />

  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
  <div v-else-if="!data || data.items.length === 0" class="empty-articles">暂无日志</div>

  <template v-else>
    <!-- 微博式时间线：左边线 + 卡片，后端已消毒的 HTML 直接渲染 -->
    <ol class="mt-10 list-none space-y-6 border-l pl-6">
      <li v-for="journal in data.items" :key="journal.id">
        <article class="rounded-lg border bg-card p-5 shadow-xs">
          <time
            class="text-xs tabular-nums text-muted-foreground"
            :datetime="journal.created_at"
          >{{ formatDate(journal.created_at) }}</time>
          <HighlightedContent class="journal-content" :html="journal.content_html" />
        </article>
      </li>
    </ol>
    <AppPagination :page="data.page" :page-size="data.page_size" :total="data.total" />
  </template>
</template>
