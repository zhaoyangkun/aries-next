<script setup lang="ts">
import type { PublicArticlePage } from '~/composables/usePublicApi'

const route = useRoute()

const slug = computed(() => String(route.params.slug))
const page = computed(() => Math.max(1, Number(route.query.page) || 1))

// 未知标签 Slug 时后端返回 404，由 usePublicApi 转为 error.vue
const { data, status } = await usePublicApi<PublicArticlePage>(
  'tag-articles',
  () => `/tags/${slug.value}/articles`,
  { query: { page }, watch: [slug, page] },
)
const { tags, resolveTags } = await useTags()
const tag = computed(() => tags.value.find((t) => t.slug === slug.value))

usePageSeo({
  title: () =>
    `${tag.value?.name ?? slug.value} 标签${page.value > 1 ? ` - 第 ${page.value} 页` : ''}`,
  description: () => `标签「${tag.value?.name ?? slug.value}」下的文章`,
})
useCanonical(
  computed(() =>
    page.value > 1 ? `/tags/${slug.value}?page=${page.value}` : `/tags/${slug.value}`,
  ),
)
</script>

<template>
  <PageHero
    :title="tag?.name ?? slug"
    label="标签"
    :description="tag ? `共 ${tag.article_count} 篇文章` : undefined"
  />

  <section class="article-list" aria-label="标签文章列表" aria-live="polite">
    <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
    <div v-else-if="!data || data.items.length === 0" class="empty-articles">该标签下暂无文章</div>
    <template v-else>
      <ArticleCard
        v-for="article in data.items"
        :key="article.id"
        :article="article"
        :tags="resolveTags(article.tag_ids)"
      />
      <AppPagination :page="data.page" :page-size="data.page_size" :total="data.total" />
    </template>
  </section>
</template>
