<script setup lang="ts">
import type { PublicTag } from '~/composables/usePublicApi'

const { data: tags, status } = await usePublicApi<PublicTag[]>('tag-list', '/tags')

// 标签云：字号按文章数对数缩放（0.85rem ~ 2rem），对数让低频标签间的差距可辨、
// 高频标签不至于过大；无 appear，SSR 首屏直接出终态，避免 hydration 不一致
const MIN_SIZE = 0.85
const MAX_SIZE = 2

interface CloudTag extends PublicTag {
  size: number
  /** 列表序，供入场动画的 transition-delay 使用 */
  index: number
}

const cloudTags = computed<CloudTag[]>(() => {
  const list = tags.value ?? []
  if (list.length === 0) return []
  const counts = list.map((tag) => Math.max(1, tag.article_count))
  const min = Math.min(...counts)
  const max = Math.max(...counts)
  return list.map((tag, index) => {
    let size = (MIN_SIZE + MAX_SIZE) / 2
    if (max > min) {
      const t = (Math.log(Math.max(1, tag.article_count)) - Math.log(min)) / (Math.log(max) - Math.log(min))
      size = MIN_SIZE + t * (MAX_SIZE - MIN_SIZE)
    }
    // 同文章数的标签字号一致（size 不依赖 map 顺序）；--i 供入场交错动画使用
    return { ...tag, size, index }
  })
})

useSeoMeta({
  title: '标签',
  description: '按标签浏览文章',
})
useCanonical('/tags')
</script>

<template>
  <PageHero title="标签" label="按关键词浏览" />

  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
  <div v-else-if="cloudTags.length === 0" class="empty-articles">暂无标签</div>
  <TransitionGroup v-else name="tag-cloud" tag="div" class="mt-10 flex flex-wrap items-baseline gap-x-4 gap-y-3">
    <NuxtLink
      v-for="tag in cloudTags"
      :key="tag.id"
      :to="`/tags/${tag.slug}`"
      class="tag-chip"
      :style="{ fontSize: `${tag.size}rem`, '--i': tag.index }"
    >
      {{ tag.name }}
      <span class="tag-chip-count">{{ tag.article_count }}</span>
    </NuxtLink>
  </TransitionGroup>
</template>

<style scoped>
/* 入场交错：transition-delay 按 --i 递增，出现时从下方淡入上浮（客户端路由切换时生效） */
.tag-cloud-enter-active {
  transition:
    opacity 0.4s ease,
    transform 0.4s ease;
  transition-delay: calc(var(--i) * 30ms);
}

.tag-cloud-enter-from {
  opacity: 0;
  transform: translateY(10px);
}

@media (prefers-reduced-motion: reduce) {
  .tag-cloud-enter-active {
    transition: none;
  }
}
</style>
