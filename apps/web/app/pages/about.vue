<script setup lang="ts">
import type { PublicPageDetail } from '~/composables/usePublicApi'

// 关于页内容复用页面（Page）机制：管理端创建并发布 slug 为 about 的页面后此处自动生效；
// 尚未创建时后端返回 404，保留静态占位
const { data: pageData } = await usePublicApi<PublicPageDetail>('about-page', () => '/pages/about', {
  ignoreError: true,
})

usePageSeo({ title: () => pageData.value?.title ?? '关于' })
</script>

<template>
  <template v-if="pageData">
    <PageHero :title="pageData.title" label="关于" />
    <article class="prose-page mt-10">
      <!-- 后端已渲染并消毒的 HTML -->
      <HighlightedContent :html="pageData.content_html" />
    </article>

    <!-- 页面评论：与 /custom/[slug] 同语义，复用页面评论接口（approved 两级树 + ?page= 分页） -->
    <section class="prose-page mt-14 border-t pt-8" aria-label="评论">
      <CommentSection
        target-type="page"
        :target-slug="pageData.slug"
        :list-path="`/pages/${pageData.slug}/comments`"
      />
    </section>
  </template>

  <PageHero v-else title="关于" label="Aries" />
</template>
