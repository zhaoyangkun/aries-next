<script setup lang="ts">
import type { PublicArticlePage } from '~/composables/usePublicApi'
import { parseKeywords } from '~/utils/highlight'

const route = useRoute()

const q = computed(() => String(route.query.q ?? '').trim())
const page = computed(() => Math.max(1, Number(route.query.page) || 1))
const keyword = ref(q.value)
// 浏览器回退/前进时同步搜索框
watch(q, (value) => {
  keyword.value = value
})

const hasKeyword = computed(() => q.value.length > 0)
// 高亮用关键词列表（空格分词、去重）
const keywords = computed(() => parseKeywords(q.value))

const { data, status } = await usePublicApi<PublicArticlePage>(
  'search-results',
  '/search',
  { query: { q, page }, watch: [q, page], enabled: hasKeyword },
)
// 标签表供结果卡片的标签 chips 查名
const { resolveTags } = await useTags()

function submit() {
  const value = keyword.value.trim()
  navigateTo({ path: '/search', query: value ? { q: value } : {} })
}

useSeoMeta({
  title: () => (q.value ? `搜索「${q.value}」` : '搜索'),
  description: '站内文章搜索',
  // 搜索结果页是动态薄内容，不参与索引
  robots: 'noindex',
})
useCanonical('/search')
</script>

<template>
  <PageHero title="搜索" label="站内搜索" />

  <form class="mt-8 flex max-w-xl gap-2" role="search" @submit.prevent="submit">
    <input
      v-model="keyword"
      type="search"
      maxlength="100"
      placeholder="输入关键词搜索文章…"
      aria-label="搜索关键词"
      class="min-w-0 flex-1 rounded-md border bg-background px-3 py-2 text-sm outline-none transition-colors focus:border-primary"
    />
    <button
      type="submit"
      class="shrink-0 cursor-pointer rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-opacity hover:opacity-90"
    >搜索</button>
  </form>

  <section class="article-list" aria-label="搜索结果" aria-live="polite">
    <div v-if="!hasKeyword" class="empty-articles">输入关键词开始搜索</div>
    <div v-else-if="status === 'pending'" class="empty-articles">搜索中…</div>
    <div v-else-if="!data || data.items.length === 0" class="empty-articles">
      没有找到与「{{ q }}」相关的文章
    </div>
    <template v-else>
      <p class="m-0 text-sm text-muted-foreground">
        共找到 {{ data.total }} 篇与「{{ q }}」相关的文章
      </p>
      <ArticleCard
        v-for="article in data.items"
        :key="article.id"
        :article="article"
        :tags="resolveTags(article.tag_ids)"
        :keywords="keywords"
      />
      <AppPagination :page="data.page" :page-size="data.page_size" :total="data.total" />
    </template>
  </section>
</template>
