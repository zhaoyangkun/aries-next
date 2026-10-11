<script setup lang="ts">
import type { PublicGalleryPage } from '~/composables/usePublicApi'

const route = useRoute()
const site = await useSite()
const siteUrl = useSiteUrl()

const page = computed(() => Math.max(1, Number(route.query.page) || 1))

const { data, status } = await usePublicApi<PublicGalleryPage>('gallery-list', '/galleries', {
  query: { page },
  watch: [page],
})

// 分页页标题带上页码，保证各页 Title 唯一
useSeoMeta({
  title: () => (page.value > 1 ? `图库 - 第 ${page.value} 页` : '图库'),
  description: '摄影与图片集',
  ogTitle: '图库',
  ogType: 'website',
  ogImage: () => toAbsoluteUrl(site.value.default_cover_url, siteUrl.value),
})
useCanonical(computed(() => (page.value > 1 ? `/galleries?page=${page.value}` : '/galleries')))
</script>

<template>
  <PageHero title="图库" label="光影记录" />

  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>
  <div v-else-if="!data || data.items.length === 0" class="empty-articles">暂无图库</div>

  <template v-else>
    <div class="mt-10 grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
      <NuxtLink
        v-for="gallery in data.items"
        :key="gallery.slug"
        :to="`/galleries/${gallery.slug}`"
        class="group overflow-hidden rounded-lg border no-underline transition-colors hover:bg-muted"
      >
        <img
          v-if="gallery.cover_url"
          :src="thumbUrl(gallery.cover_url, 768)"
          :srcset="thumbSrcset(gallery.cover_url, [480, 768, 1200])"
          sizes="(max-width: 639px) 100vw, (max-width: 1023px) 50vw, 330px"
          :alt="gallery.title"
          loading="lazy"
          class="aspect-[3/2] w-full object-cover transition-transform group-hover:scale-[1.02]"
        >
        <!-- 未设置封面时用渐变底 + 标题首字占位，保持网格高度一致 -->
        <div
          v-else
          class="grid aspect-[3/2] w-full place-items-center text-3xl font-semibold text-white/70"
          style="background: linear-gradient(150deg, #3a3d42 0%, #26282b 48%, #141517 100%)"
          aria-hidden="true"
        >{{ gallery.title.charAt(0) }}</div>
        <div class="p-4">
          <h2 class="m-0 truncate text-base font-semibold text-foreground">{{ gallery.title }}</h2>
          <p v-if="gallery.description" class="mt-1 text-sm text-muted-foreground line-clamp-2">
            {{ gallery.description }}
          </p>
        </div>
      </NuxtLink>
    </div>
    <AppPagination :page="data.page" :page-size="data.page_size" :total="data.total" />
  </template>
</template>
