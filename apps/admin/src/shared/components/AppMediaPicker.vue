<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  ChevronLeftIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  ImagesIcon,
  LoaderCircleIcon,
  RefreshCwIcon,
  SearchIcon,
  UploadIcon,
  XIcon,
} from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import {
  MEDIA_UPLOAD_ACCEPT,
  mediaApi,
  type MediaAsset,
} from '@/modules/media/api/media'
import { useMediaUpload } from '@/modules/media/composables/useMediaUpload'
import { formatFileSize } from '@/modules/media/format'
import { getApiError } from '@/shared/api/client'
import { useDebouncedWatch } from '@/composables/use-debounced-watch'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'

const props = defineProps<{
  open: boolean
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  select: [asset: MediaAsset]
}>()

const PAGE_SIZE = 12

const keyword = ref('')
const page = ref(1)
const assets = ref<MediaAsset[]>([])
const total = ref(0)
const loading = ref(false)
const error = ref('')
const fileInput = ref<HTMLInputElement | null>(null)

const { uploading, progress, error: uploadError, duplicates, upload } = useMediaUpload(() => {
  // 上传完成后刷新第一页，让新资产出现在最前（服务端按创建时间倒序）。
  if (page.value !== 1) page.value = 1
  else void loadAssets()
})

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))

watch(
  () => props.open,
  (open) => {
    if (!open) return
    keyword.value = ''
    page.value = 1
    void loadAssets()
  },
)

useDebouncedWatch(keyword, () => {
  if (page.value !== 1) page.value = 1
  else void loadAssets()
}, 250)

watch(page, () => {
  void loadAssets()
})

async function loadAssets() {
  loading.value = true
  error.value = ''
  try {
    const pageData = await mediaApi.list({
      page: page.value,
      page_size: PAGE_SIZE,
      keyword: keyword.value.trim() || undefined,
    })
    assets.value = pageData.items
    total.value = pageData.total
  } catch (requestError) {
    error.value = getApiError(requestError, '媒体库加载失败')
  } finally {
    loading.value = false
  }
}

function triggerUpload() {
  fileInput.value?.click()
}

async function handleFileChange(event: Event) {
  const input = event.target as HTMLInputElement
  const files = input.files
  if (files?.length) await upload(files)
  // 重置 input，允许连续选择同一文件。
  input.value = ''
}

function handleSelect(asset: MediaAsset) {
  emit('select', asset)
  emit('update:open', false)
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <DialogContent class="flex h-[min(640px,calc(100dvh-2rem))] w-[calc(100%-1rem)] max-w-3xl! flex-col gap-0 overflow-hidden p-0">
      <DialogHeader class="border-b px-4 py-4 text-left sm:px-5">
        <DialogTitle class="text-base">选择媒体</DialogTitle>
        <DialogDescription class="mt-0.5">从媒体库选择一张图片，或直接上传新图片。</DialogDescription>
      </DialogHeader>

      <div class="flex items-center gap-2 border-b px-4 py-3 sm:px-5">
        <div class="relative min-w-0 flex-1">
          <SearchIcon class="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input v-model="keyword" class="pl-8 pr-8" placeholder="搜索文件名" aria-label="搜索媒体" />
          <Button
            v-if="keyword"
            variant="ghost"
            size="icon-sm"
            class="absolute right-0.5 top-1/2 -translate-y-1/2 text-muted-foreground"
            aria-label="清除搜索"
            @click="keyword = ''"
          >
            <XIcon />
          </Button>
        </div>
        <Button variant="outline" :disabled="uploading" @click="triggerUpload">
          <LoaderCircleIcon v-if="uploading" class="animate-spin" />
          <UploadIcon v-else />
          {{ uploading ? `上传中 ${progress}%` : '上传图片' }}
        </Button>
        <input
          ref="fileInput"
          type="file"
          class="hidden"
          multiple
          :accept="MEDIA_UPLOAD_ACCEPT"
          @change="handleFileChange"
        />
      </div>

      <div v-if="uploading" class="h-1 w-full bg-muted">
        <div class="h-full bg-primary transition-all" :style="{ width: `${progress}%` }" />
      </div>
      <p v-if="uploadError" role="alert" class="flex items-center gap-1.5 border-b px-4 py-2 text-xs font-medium text-destructive sm:px-5">
        <CircleAlertIcon class="size-3.5 shrink-0" />
        {{ uploadError }}
      </p>
      <p v-else-if="duplicates.length" class="border-b bg-muted/40 px-4 py-2 text-xs text-muted-foreground sm:px-5">
        {{ duplicates.length }} 个文件与已有资产内容相同，已复用原文件。
      </p>

      <div class="min-h-0 flex-1 overflow-y-auto p-4 sm:p-5" :aria-busy="loading">
        <div v-if="loading" class="flex h-full items-center justify-center text-sm text-muted-foreground">
          <LoaderCircleIcon class="mr-2 size-4 animate-spin" />
          正在加载媒体库
        </div>
        <div v-else-if="error" class="flex h-full flex-col items-center justify-center">
          <p class="text-sm font-medium">{{ error }}</p>
          <Button variant="outline" size="sm" class="mt-3" @click="loadAssets">
            <RefreshCwIcon />
            重试
          </Button>
        </div>
        <AppEmptyState
          v-else-if="assets.length === 0"
          :title="keyword.trim() ? '没有匹配的媒体文件' : '媒体库为空'"
          :description="keyword.trim() ? '尝试修改搜索关键词。' : '上传第一张图片后，即可在这里选择。'"
          :icon="ImagesIcon"
          class="h-full min-h-0"
        >
          <template v-if="!keyword.trim()" #actions>
            <Button variant="outline" :disabled="uploading" @click="triggerUpload">
              <UploadIcon />
              上传图片
            </Button>
          </template>
        </AppEmptyState>
        <ul v-else class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
          <li v-for="asset in assets" :key="asset.id">
            <button
              type="button"
              class="group w-full overflow-hidden rounded-lg border bg-background text-left outline-none transition-colors hover:border-primary focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
              :aria-label="`选择 ${asset.alt || asset.original_name}`"
              @click="handleSelect(asset)"
            >
              <div class="aspect-4/3 w-full overflow-hidden bg-muted">
                <img
                  :src="asset.url"
                  :alt="asset.alt || asset.original_name"
                  loading="lazy"
                  class="size-full object-cover transition-transform group-hover:scale-105"
                />
              </div>
              <div class="px-2 py-1.5">
                <p class="truncate text-xs font-medium">{{ asset.alt || asset.original_name }}</p>
                <p class="text-[11px] text-muted-foreground">{{ formatFileSize(asset.size_bytes) }}</p>
              </div>
            </button>
          </li>
        </ul>
      </div>

      <DialogFooter class="flex-row items-center justify-between border-t px-4 py-3 sm:px-5">
        <p class="text-xs text-muted-foreground">共 {{ total }} 个文件</p>
        <div class="flex items-center gap-1">
          <Button
            variant="outline"
            size="icon-sm"
            aria-label="上一页"
            :disabled="page <= 1 || loading"
            @click="page -= 1"
          >
            <ChevronLeftIcon />
          </Button>
          <span class="min-w-14 text-center text-xs tabular-nums text-muted-foreground">{{ page }} / {{ pageCount }}</span>
          <Button
            variant="outline"
            size="icon-sm"
            aria-label="下一页"
            :disabled="page >= pageCount || loading"
            @click="page += 1"
          >
            <ChevronRightIcon />
          </Button>
        </div>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
