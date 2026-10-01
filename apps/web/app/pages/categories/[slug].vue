<script setup lang="ts">
import type { PublicArticlePage, PublicCategory } from '~/composables/usePublicApi'

const route = useRoute()

const slug = computed(() => String(route.params.slug))
const page = computed(() => Math.max(1, Number(route.query.page) || 1))

// 未知分类 Slug 时后端返回 404，由 usePublicApi 转为 error.vue
const { data, status } = await usePublicApi<PublicArticlePage>(
  'category-articles',
  () => `/categories/${slug.value}/articles`,
  { query: { page }, watch: [slug, page] },
)
// 分类名称/描述用于标题与 SEO；标签表供卡片标签 chips 查名
const { data: categories } = await usePublicApi<PublicCategory[]>('category-list', '/categories')
const { resolveTags } = await useTags()
const category = computed(() => (categories.value ?? []).find((c) => c.slug === slug.value))

usePageSeo({
  title: () =>
    `${category.value?.name ?? slug.value} 分类${page.value > 1 ? ` - 第 ${page.value} 页` : ''}`,
  description: () =>
    category.value?.description || `分类「${category.value?.name ?? slug.value}」下的文章`,
})
useCanonical(
  computed(() =>
    page.value > 1 ? `/categories/${slug.value}?page=${page.value}` : `/categories/${slug.value}`,
  ),
)
</script>

<template>
  <PageHero
    :title="category?.name ?? slug"
    label="分类"
    :description="category?.description || undefined"
  />

  <section class="article-list" aria-label="分类文章列表" aria-live="polite">
    <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
    <div v-else-if="!data || data.items.length === 0" class="empty-articles">该分类下暂无文章</div>
    <template v-else>
      <ArticleCard
        v-for="article in data.items"
        :key="article.id"
        :article="article"
        :category="category"
        :tags="resolveTags(article.tag_ids)"
      />
      <AppPagination :page="data.page" :page-size="data.page_size" :total="data.total" />
    </template>
  </section>
</template>
