<script setup lang="ts">
import type {
  PublicArticlePage,
  PublicCategory,
  PublicTaxonomyRef,
} from '~/composables/usePublicApi'

const route = useRoute()
const site = await useSite()
const siteUrl = useSiteUrl()

const page = computed(() => Math.max(1, Number(route.query.page) || 1))

const { data, status } = await usePublicApi<PublicArticlePage>('article-feed', '/articles', {
  query: { page },
  watch: [page],
})
// 列表项只带 category_id / tag_ids，需要分类表与标签表做 id → 名称/Slug 的映射
const { data: categories } = await usePublicApi<PublicCategory[]>('category-list', '/categories')
const { resolveTags } = await useTags()

const categoryMap = computed(() => {
  const map = new Map<number, PublicTaxonomyRef>()
  for (const c of categories.value ?? []) map.set(c.id, c)
  return map
})

// 分页页标题带上页码，保证各页 Title 唯一
useSeoMeta({
  title: () => (page.value > 1 ? `最新文章 - 第 ${page.value} 页` : '最新文章'),
  description: () => site.value.site_description || '最新发布的文章',
  ogTitle: () => site.value.site_name,
  ogDescription: () => site.value.site_description,
  ogType: 'website',
  ogImage: () => toAbsoluteUrl(site.value.default_cover_url, siteUrl.value),
})
useCanonical(computed(() => (page.value > 1 ? `/?page=${page.value}` : '/')))
</script>

<template>
  <!-- 全屏封面：站点名 + slogan + 下滑箭头（xue 式首页） -->
  <HomeHero />

  <section id="article-list" class="article-list" aria-label="文章列表" aria-live="polite">
    <h2 class="list-heading">最新文章</h2>
    <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
    <div v-else-if="!data || data.items.length === 0" class="empty-articles">暂无已发布文章</div>
    <template v-else>
      <ArticleCard
        v-for="article in data.items"
        :key="article.id"
        :article="article"
        :category="article.category_id != null ? categoryMap.get(article.category_id) : undefined"
        :tags="resolveTags(article.tag_ids)"
      />
      <AppPagination :page="data.page" :page-size="data.page_size" :total="data.total" />
    </template>
  </section>
</template>
