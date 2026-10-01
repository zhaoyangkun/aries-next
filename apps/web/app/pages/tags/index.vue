<script setup lang="ts">
import type { PublicTag } from '~/composables/usePublicApi'

const { data: tags, status } = await usePublicApi<PublicTag[]>('tag-list', '/tags')

useSeoMeta({
  title: '标签',
  description: '按标签浏览文章',
})
useCanonical('/tags')
</script>

<template>
  <PageHero title="标签" label="按关键词浏览" />

  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
  <div v-else-if="!tags || tags.length === 0" class="empty-articles">暂无标签</div>
  <div v-else class="mt-10 flex flex-wrap gap-3">
    <NuxtLink
      v-for="tag in tags"
      :key="tag.id"
      :to="`/tags/${tag.slug}`"
      class="tag-chip"
    >
      {{ tag.name }}
      <span class="tag-chip-count">{{ tag.article_count }}</span>
    </NuxtLink>
  </div>
</template>
