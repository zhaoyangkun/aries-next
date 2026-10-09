<script setup lang="ts">
// 首页全屏封面：大标题站点名 + slogan + 下滑箭头，
// 背景取站点默认封面，未配置时用冷调深色渐变兜底
const site = await useSite()

const coverUrl = computed(() => site.value.default_cover_url || '')
</script>

<template>
  <header class="home-hero">
    <div class="home-hero-bg">
      <!-- 首屏 LCP：按需缩放 + srcset，fetchpriority 拉高抢占带宽；
           档位止于 1200 是因为后端缩放上限 1200（超界静默回退原图，反而更慢） -->
      <img
        v-if="coverUrl"
        :src="thumbUrl(coverUrl, 1200)"
        :srcset="thumbSrcset(coverUrl, [640, 960, 1200])"
        sizes="100vw"
        :alt="site.site_name"
        fetchpriority="high"
        decoding="async"
      />
      <div v-else class="home-hero-fallback" aria-hidden="true" />
      <div class="home-hero-overlay" aria-hidden="true" />
    </div>
    <div class="home-hero-content">
      <h1 class="home-hero-title">{{ site.site_name }}</h1>
      <p v-if="site.site_description" class="home-hero-slogan">{{ site.site_description }}</p>
    </div>
    <!-- 锚点跳到文章列表，滚动动画由 CSS（scroll-behavior + 跳动 keyframes）完成 -->
    <a class="hero-arrow" href="#article-list" aria-label="下滑查看文章列表">
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="32"
        height="32"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="m6 9 6 6 6-6" />
      </svg>
    </a>
  </header>
</template>
