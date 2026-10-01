<script setup lang="ts">
import type { PublicPageDetail } from '~/composables/usePublicApi'

const route = useRoute()
const site = await useSite()

const slug = computed(() => String(route.params.slug))

// 对齐旧版 xue 主题路由 /custom/{slug}；未发布或不存在的页面后端返回 404，落入 error.vue
const { data: pageData, status } = await usePublicApi<PublicPageDetail>(
  'custom-page',
  () => `/pages/${slug.value}`,
  { watch: [slug] },
)

useSeoMeta({
  title: () => pageData.value?.title,
  // 页面接口没有摘要字段，退回站点描述
  description: () => site.value.site_description || pageData.value?.title,
  ogTitle: () => pageData.value?.title,
  ogType: 'article',
})
useCanonical(computed(() => `/custom/${slug.value}`))
</script>

<template>
  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>

  <template v-else-if="pageData">
    <PageHero :title="pageData.title" label="页面" />
    <article class="prose-page mt-10">
      <p class="m-0 text-xs text-muted-foreground">
        更新于 <time :datetime="pageData.updated_at">{{ formatDate(pageData.updated_at) }}</time>
      </p>
      <!-- 后端已渲染并消毒的 HTML -->
      <HighlightedContent :html="pageData.content_html" />
    </article>

    <!-- 页面评论：与文章同语义的列表接口（approved 两级树 + ?page= 分页） -->
    <section class="prose-page mt-14 border-t pt-8" aria-label="评论">
      <CommentSection
        target-type="page"
        :target-slug="pageData.slug"
        :list-path="`/pages/${pageData.slug}/comments`"
      />
    </section>
  </template>
</template>
