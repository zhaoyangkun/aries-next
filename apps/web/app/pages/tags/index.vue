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
  /** 标签云配色（浅色/深色各一），由 slug 稳定哈希选取，SSR 与 hydration 一致 */
  color: { light: string, dark: string }
}

// 标签云配色板：Tailwind v4 默认色板（OKLCH 感知均匀色彩空间，P3 广色域）中
// 12 个高明度色系——浅色模式取 500 档、深色模式取 400 档；色相间隔均匀，
// 用 slug 稳定哈希取色（同一标签颜色固定，不用随机数）
const TAG_COLORS: Array<{ light: string, dark: string }> = [
  { light: 'oklch(0.645 0.246 16.4)', dark: 'oklch(0.704 0.191 22.2)' }, // rose
  { light: 'oklch(0.705 0.213 47.6)', dark: 'oklch(0.75 0.183 55.9)' }, // orange
  { light: 'oklch(0.769 0.188 70.1)', dark: 'oklch(0.828 0.189 84.4)' }, // amber
  { light: 'oklch(0.768 0.233 130.9)', dark: 'oklch(0.841 0.238 128.9)' }, // lime
  { light: 'oklch(0.696 0.17 162.5)', dark: 'oklch(0.765 0.177 163.2)' }, // emerald
  { light: 'oklch(0.704 0.14 182.5)', dark: 'oklch(0.777 0.152 181.9)' }, // teal
  { light: 'oklch(0.715 0.143 215.2)', dark: 'oklch(0.789 0.154 211.5)' }, // cyan
  { light: 'oklch(0.685 0.169 237.3)', dark: 'oklch(0.746 0.16 232.7)' }, // sky
  { light: 'oklch(0.623 0.214 259.8)', dark: 'oklch(0.707 0.165 254.6)' }, // blue
  { light: 'oklch(0.606 0.25 292.7)', dark: 'oklch(0.702 0.183 293.5)' }, // violet
  { light: 'oklch(0.667 0.295 322.2)', dark: 'oklch(0.74 0.238 322.2)' }, // fuchsia
  { light: 'oklch(0.656 0.241 354.3)', dark: 'oklch(0.718 0.202 349.8)' }, // pink
]

function hashColor(value: string): { light: string, dark: string } {
  let hash = 0x811c9dc5
  for (let i = 0; i < value.length; i++) {
    hash ^= value.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193)
  }
  // 取模保证下标必在界内，non-null 断言安全
  return TAG_COLORS[(hash >>> 0) % TAG_COLORS.length]!
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
    return { ...tag, size, index, color: hashColor(tag.slug) }
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
      class="tag-chip tag-chip-cloud"
      :style="{ fontSize: `${tag.size}rem`, '--i': tag.index, '--tag-c': tag.color.light, '--tag-c-dark': tag.color.dark }"
    >
      {{ tag.name }}
      <span class="tag-chip-count">{{ tag.article_count }}</span>
    </NuxtLink>
  </TransitionGroup>
</template>

<style scoped>
/* 标签云配色：--tag-c（浅色模式）/ --tag-c-dark（深色模式）由 Tailwind v4 OKLCH 色板
   按 slug 稳定哈希注入；边框、计数徽章与 hover 底色用 color-mix 取同色的透明度变体 */
.tag-chip-cloud {
  color: var(--tag-c);
  border-color: color-mix(in oklch, var(--tag-c) 45%, transparent);
}

.tag-chip-cloud:hover {
  color: var(--tag-c);
  border-color: color-mix(in oklch, var(--tag-c) 70%, transparent);
  background-color: color-mix(in oklch, var(--tag-c) 10%, transparent);
  box-shadow: 0 4px 14px color-mix(in oklch, var(--tag-c) 25%, transparent);
}

.tag-chip-cloud .tag-chip-count {
  color: color-mix(in oklch, var(--tag-c) 80%, transparent);
}

.tag-chip-cloud:hover .tag-chip-count {
  color: var(--tag-c);
}

:global(.dark) .tag-chip-cloud {
  color: var(--tag-c-dark);
  border-color: color-mix(in oklch, var(--tag-c-dark) 45%, transparent);
}

:global(.dark) .tag-chip-cloud:hover {
  color: var(--tag-c-dark);
  border-color: color-mix(in oklch, var(--tag-c-dark) 70%, transparent);
  background-color: color-mix(in oklch, var(--tag-c-dark) 14%, transparent);
  box-shadow: 0 4px 14px color-mix(in oklch, var(--tag-c-dark) 28%, transparent);
}

:global(.dark) .tag-chip-cloud .tag-chip-count {
  color: color-mix(in oklch, var(--tag-c-dark) 80%, transparent);
}

:global(.dark) .tag-chip-cloud:hover .tag-chip-count {
  color: var(--tag-c-dark);
}

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
