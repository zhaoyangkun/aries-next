<script setup lang="ts">
import type { PublicCategory } from '~/composables/usePublicApi'

const { data: categories, status } = await usePublicApi<PublicCategory[]>(
  'category-list',
  '/categories',
)

useSeoMeta({
  title: '分类',
  description: '按分类浏览文章',
})
useCanonical('/categories')
</script>

<template>
  <PageHero title="分类" label="按主题浏览" />

  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
  <div v-else-if="!categories || categories.length === 0" class="empty-articles">暂无分类</div>
  <div v-else class="mt-10 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
    <NuxtLink
      v-for="category in categories"
      :key="category.id"
      :to="`/categories/${category.slug}`"
      class="rounded-lg border bg-card p-5 no-underline shadow-xs transition duration-300 hover:-translate-y-0.5 hover:border-primary hover:shadow-md"
    >
      <div class="flex items-baseline justify-between gap-2">
        <h2 class="m-0 text-base font-semibold">{{ category.name }}</h2>
        <span class="shrink-0 text-xs text-muted-foreground">{{ category.article_count }} 篇</span>
      </div>
      <p v-if="category.description" class="mt-2 mb-0 line-clamp-2 text-sm text-muted-foreground">
        {{ category.description }}
      </p>
    </NuxtLink>
  </div>
</template>
