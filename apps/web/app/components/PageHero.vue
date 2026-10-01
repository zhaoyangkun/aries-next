<script setup lang="ts">
// 子页半高封面：页面标题 + 可选眉标/描述 + 可选封面图。
// 封面未传时回退站点默认封面，都没有时用渐变兜底
const props = defineProps<{
  title: string
  /** 标题上方的红色系眉标，如「分类」「标签」 */
  label?: string
  description?: string
  coverUrl?: string | null
}>()

const site = await useSite()

const cover = computed(() => props.coverUrl || site.value.default_cover_url || '')
</script>

<template>
  <section class="page-hero">
    <div class="home-hero-bg">
      <img v-if="cover" :src="cover" alt="" aria-hidden="true" />
      <div v-else class="home-hero-fallback" aria-hidden="true" />
      <div class="home-hero-overlay" aria-hidden="true" />
    </div>
    <div class="page-hero-content">
      <p v-if="label" class="page-hero-label">{{ label }}</p>
      <h1 class="page-hero-title">{{ title }}</h1>
      <p v-if="description" class="page-hero-desc">{{ description }}</p>
    </div>
  </section>
</template>
