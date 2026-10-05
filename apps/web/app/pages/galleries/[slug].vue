<script setup lang="ts">
import type { PublicGalleryDetail } from '~/composables/usePublicApi'

const route = useRoute()
const siteUrl = useSiteUrl()

const slug = computed(() => String(route.params.slug))

// 灯箱状态：点击瀑布流图片原地预览（缩放/拖拽/左右切换），不再跳新窗口
const lightboxOpen = ref(false)
const lightboxIndex = ref(0)

function openLightbox(index: number) {
  lightboxIndex.value = index
  lightboxOpen.value = true
}

// 不存在或未发布的图库由后端返回 404，经 usePublicApi 转为 error.vue
const { data: gallery, status } = await usePublicApi<PublicGalleryDetail>(
  'gallery-detail',
  () => `/galleries/${slug.value}`,
  { watch: [slug] },
)

useSeoMeta({
  title: () => gallery.value?.title,
  description: () =>
    gallery.value?.description || `图库「${gallery.value?.title ?? slug.value}」`,
  ogTitle: () => gallery.value?.title,
  ogDescription: () => gallery.value?.description,
  ogType: 'website',
  // og:image 必须绝对地址，取第一张图
  ogImage: () => toAbsoluteUrl(gallery.value?.items[0]?.url, siteUrl.value),
})
useCanonical(computed(() => `/galleries/${slug.value}`))
</script>

<template>
  <PageHero
    :title="gallery?.title ?? slug"
    label="图库"
    :description="gallery?.description || undefined"
    :cover-url="gallery?.items[0]?.url"
  />

  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
  <div v-else-if="!gallery || gallery.items.length === 0" class="empty-articles">该图库暂无图片</div>

  <!-- CSS columns 瀑布流：按自然高度排列，不引布局依赖；点击开灯箱预览 -->
  <div v-else class="mt-10 columns-1 gap-4 sm:columns-2 lg:columns-3">
    <figure v-for="(item, index) in gallery.items" :key="index" class="mb-4 break-inside-avoid">
      <button
        type="button"
        class="block w-full cursor-zoom-in"
        :aria-label="`预览图片：${item.alt || gallery.title}`"
        @click="openLightbox(index)"
      >
        <img
          :src="thumbUrl(item.url, 768)"
          :srcset="thumbSrcset(item.url, [480, 768, 1200])"
          sizes="(max-width: 639px) 100vw, (max-width: 1023px) 50vw, 330px"
          :alt="item.alt || gallery.title"
          :width="item.width ?? undefined"
          :height="item.height ?? undefined"
          loading="lazy"
          class="w-full rounded-lg border transition-opacity hover:opacity-90"
        >
      </button>
      <figcaption v-if="item.location" class="mt-1.5 text-xs text-muted-foreground">
        {{ item.location }}
      </figcaption>
    </figure>
  </div>

  <AppImageLightbox
    v-if="gallery"
    v-model:open="lightboxOpen"
    v-model:index="lightboxIndex"
    :items="gallery.items"
  />
</template>
