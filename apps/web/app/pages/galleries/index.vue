<script setup lang="ts">
import type {
  PublicGalleryItem,
  PublicGalleryPage,
  PublicPhoto,
} from '~/composables/usePublicApi'

const route = useRoute()
const site = await useSite()
const siteUrl = useSiteUrl()

// 视图切换走 ?view=albums query：可分享、可直达；默认（无参）为照片墙，canonical 保持干净
const view = computed(() => (route.query.view === 'albums' ? 'albums' : 'photos'))
const page = computed(() => Math.max(1, Number(route.query.page) || 1))

// enabled 语义：视图未激活时跳过请求（data 为 null），把 enabled 放进 watch 让切入该视图时自动拉取
const photosEnabled = computed(() => view.value === 'photos')
const albumsEnabled = computed(() => view.value === 'albums')

// 照片墙数据不分页一次拉完，分类筛选纯前端进行
const { data: photos, status: photosStatus } = await usePublicApi<PublicPhoto[]>(
  'photo-wall',
  '/photos',
  { watch: [photosEnabled], enabled: photosEnabled },
)

// 相册数据沿用原有分页接口，仅切到相册视图时请求
const { data, status } = await usePublicApi<PublicGalleryPage>('gallery-list', '/galleries', {
  query: { page },
  watch: [page, albumsEnabled],
  enabled: albumsEnabled,
})

// 顶部分类筛选 chips：从照片数据提取去重分类，无分类的归「未分类」固定排最后
const UNCATEGORIZED = '未分类'
const activeCategory = ref('')

const categories = computed(() => {
  const names = new Set<string>()
  for (const photo of photos.value ?? []) names.add(photo.category_name || UNCATEGORIZED)
  const list = Array.from(names).filter((name) => name !== UNCATEGORIZED)
  if (names.has(UNCATEGORIZED)) list.push(UNCATEGORIZED)
  return list
})

const filteredPhotos = computed(() => {
  const list = photos.value ?? []
  if (!activeCategory.value) return list
  return list.filter((photo) => (photo.category_name || UNCATEGORIZED) === activeCategory.value)
})

// 照片数据刷新后若当前筛选分类已不存在，退回「全部」避免停在无结果态
watch(categories, (list) => {
  if (activeCategory.value && !list.includes(activeCategory.value)) activeCategory.value = ''
})

// 灯箱在「当前过滤结果」内原地预览、左右切换；items 需映射成灯箱约定的 PublicGalleryItem 形状
const lightboxOpen = ref(false)
const lightboxIndex = ref(0)

const lightboxItems = computed<PublicGalleryItem[]>(() =>
  filteredPhotos.value.map((photo) => ({
    url: photo.url,
    alt: photo.alt,
    location: photo.location,
    width: photo.width,
    height: photo.height,
  })),
)

function openLightbox(index: number) {
  lightboxIndex.value = index
  lightboxOpen.value = true
}

// 分页页标题带上页码，保证各页 Title 唯一；两个视图共用「图库」前缀
useSeoMeta({
  title: () => {
    if (view.value === 'albums') {
      return page.value > 1 ? `图库 - 相册 - 第 ${page.value} 页` : '图库 - 相册'
    }
    return '图库'
  },
  description: '摄影与图片集',
  ogTitle: '图库',
  ogType: 'website',
  ogImage: () => toAbsoluteUrl(site.value.default_cover_url, siteUrl.value),
})
useCanonical(
  computed(() => {
    if (view.value === 'photos') return '/galleries'
    return page.value > 1
      ? `/galleries?view=albums&page=${page.value}`
      : '/galleries?view=albums'
  }),
)

const tabBaseClass = 'rounded-md px-4 py-2 text-sm no-underline transition-colors'
const chipBaseClass = 'rounded-full border px-3.5 py-1.5 text-sm transition-colors'
</script>

<template>
  <PageHero title="图库" label="光影记录" />

  <!-- 视图切换：query 驱动，激活态沿用导航激活样式（主色红） -->
  <nav class="mt-8 flex items-center gap-1" aria-label="图库视图切换">
    <NuxtLink
      :to="{ path: '/galleries' }"
      :class="[
        tabBaseClass,
        view === 'photos'
          ? 'bg-muted font-medium text-primary'
          : 'text-muted-foreground hover:bg-muted hover:text-foreground',
      ]"
      :aria-current="view === 'photos' ? 'page' : undefined"
    >照片墙</NuxtLink>
    <NuxtLink
      :to="{ path: '/galleries', query: { view: 'albums' } }"
      :class="[
        tabBaseClass,
        view === 'albums'
          ? 'bg-muted font-medium text-primary'
          : 'text-muted-foreground hover:bg-muted hover:text-foreground',
      ]"
      :aria-current="view === 'albums' ? 'page' : undefined"
    >相册</NuxtLink>
  </nav>

  <template v-if="view === 'photos'">
    <!-- 分类筛选：数据一次拉完，纯前端过滤 -->
    <div v-if="categories.length > 0" class="mt-6 flex flex-wrap items-center gap-2">
      <button
        type="button"
        :class="[
          chipBaseClass,
          activeCategory === ''
            ? 'border-primary bg-primary text-primary-foreground'
            : 'text-muted-foreground hover:border-primary/60 hover:text-foreground',
        ]"
        @click="activeCategory = ''"
      >全部</button>
      <button
        v-for="category in categories"
        :key="category"
        type="button"
        :class="[
          chipBaseClass,
          activeCategory === category
            ? 'border-primary bg-primary text-primary-foreground'
            : 'text-muted-foreground hover:border-primary/60 hover:text-foreground',
        ]"
        @click="activeCategory = category"
      >{{ category }}</button>
    </div>

    <div v-if="photosStatus === 'pending'" class="empty-articles">加载中…</div>
    <div v-else-if="!photos || photos.length === 0" class="empty-articles">暂无照片</div>
    <div v-else-if="filteredPhotos.length === 0" class="empty-articles">该分类下暂无照片</div>

    <!-- CSS columns 瀑布流：按自然高度排列，width/height 防 CLS；点击开灯箱原地预览 -->
    <div v-else class="mt-8 columns-1 gap-4 sm:columns-2 lg:columns-3">
      <figure
        v-for="(photo, index) in filteredPhotos"
        :key="`${photo.url}-${index}`"
        class="mb-4 break-inside-avoid"
      >
        <button
          type="button"
          class="block w-full cursor-zoom-in"
          :aria-label="`预览图片：${photo.alt || photo.gallery_title}`"
          @click="openLightbox(index)"
        >
          <img
            :src="photo.url"
            :alt="photo.alt || photo.gallery_title"
            :width="photo.width ?? undefined"
            :height="photo.height ?? undefined"
            loading="lazy"
            class="w-full rounded-lg border transition-opacity hover:opacity-90"
          >
        </button>
        <figcaption
          v-if="photo.alt || photo.gallery_title"
          class="mt-1.5 text-xs text-muted-foreground"
        >{{ photo.alt || photo.gallery_title }}</figcaption>
      </figure>
    </div>

    <AppImageLightbox
      v-model:open="lightboxOpen"
      v-model:index="lightboxIndex"
      :items="lightboxItems"
    />
  </template>

  <template v-else>
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
            :src="gallery.cover_url"
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
</template>
