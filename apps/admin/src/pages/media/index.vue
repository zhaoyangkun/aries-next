<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { ColumnDef, RowSelectionState } from '@tanstack/vue-table'
import {
  CheckIcon,
  CircleAlertIcon,
  CopyIcon,
  ImagesIcon,
  LayoutGridIcon,
  LinkIcon,
  ListIcon,
  LoaderCircleIcon,
  RefreshCwIcon,
  SearchIcon,
  Trash2Icon,
  TriangleAlertIcon,
  UploadIcon,
  XIcon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Card, CardContent, CardFooter, CardHeader } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import AppImageLightbox from '@/shared/components/AppImageLightbox.vue'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import {
  MEDIA_UPLOAD_ACCEPT,
  getMediaInUseUsages,
  mediaApi,
  type MediaAsset,
  type MediaProvider,
  type MediaUsage,
} from '@/modules/media/api/media'
import { useMediaUpload } from '@/modules/media/composables/useMediaUpload'
import {
  formatFileSize,
  formatMediaTime,
  providerText,
  usageTargetText,
} from '@/modules/media/format'
import { getApiError } from '@/shared/api/client'
import { toast } from 'vue-sonner'
import { useDebouncedWatch } from '@/composables/use-debounced-watch'
import {
  AppDataTable,
  AppDataTableColumnToggle,
  AppDataTablePagination,
} from '@/shared/components/data-table'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import { BasicPage } from '@/components/global-layout'

type ViewMode = 'grid' | 'list'

const keyword = ref('')
const provider = ref<MediaProvider | ''>('')
const mime = ref('')
const viewMode = ref<ViewMode>('grid')
const page = ref(1)
const pageSize = ref(24)
const assets = ref<MediaAsset[]>([])
const total = ref(0)
const loading = ref(false)
const error = ref('')
const dragOver = ref(false)
const fileInput = ref<HTMLInputElement | null>(null)

const detailOpen = ref(false)
const detail = ref<MediaAsset | null>(null)
const detailAlt = ref('')
const detailName = ref('')
const detailSaving = ref(false)
const detailError = ref('')
const usages = ref<MediaUsage[]>([])
const usagesLoading = ref(false)
const usagesError = ref('')
const copied = ref(false)

const previewOpen = ref(false)
// 关闭预览后是否返回详情抽屉（Radix 弹层不能叠加，需先关抽屉再开 Lightbox）
const previewReturnSheet = ref(false)
let previewTimer: ReturnType<typeof setTimeout> | undefined

// 打开大图预览：先关闭详情抽屉，等滑出动画结束再打开 Lightbox。
// Radix Sheet 打开时会把 body 下其他兄弟节点标记为 inert，两个弹层不能同时存在。
function openPreview() {
  previewReturnSheet.value = Boolean(detail.value)
  detailOpen.value = false
  if (previewTimer) clearTimeout(previewTimer)
  previewTimer = setTimeout(() => {
    previewOpen.value = true
  }, 320)
}

function handlePreviewClose(open: boolean) {
  previewOpen.value = open
  if (!open && previewReturnSheet.value && detail.value) {
    previewReturnSheet.value = false
    detailOpen.value = true
  }
}

const deleteConfirmOpen = ref(false)
const deleting = ref(false)
const deleteBlockedUsages = ref<MediaUsage[]>([])
let dragDepth = 0
let copyTimer: ReturnType<typeof setTimeout> | undefined

// 批量选择：列表模式复用 AppDataTable 的 selection 列，网格模式在卡片角上挂 Checkbox。
const selection = ref<RowSelectionState>({})
const selectedIds = computed(() => Object.keys(selection.value).map(Number))
const batchDeleteConfirmOpen = ref(false)
const batchDeleting = ref(false)

const hasActiveFilter = computed(
  () => Boolean(keyword.value.trim()) || provider.value !== '' || mime.value !== '',
)
const usageCount = computed(() => usages.value.length)

// 列表模式列定义；媒体列表后端暂无排序参数，仅提供列显示控制。
const tableRef = ref()
const mediaColumns: ColumnDef<MediaAsset, unknown>[] = [
  { id: 'file', accessorKey: 'original_name', header: '文件', enableHiding: false, meta: { toggleLabel: '文件' } },
  {
    id: 'mime',
    accessorKey: 'mime',
    header: '类型',
    meta: {
      headerClass: 'hidden w-24 md:table-cell',
      cellClass: 'hidden text-muted-foreground md:table-cell',
      toggleLabel: '类型',
    },
  },
  {
    id: 'size',
    accessorKey: 'size_bytes',
    header: '大小',
    meta: { headerClass: 'w-24', cellClass: 'text-muted-foreground', toggleLabel: '大小' },
  },
  {
    id: 'provider',
    accessorKey: 'provider',
    header: '来源',
    meta: { headerClass: 'hidden w-28 sm:table-cell', cellClass: 'hidden sm:table-cell', toggleLabel: '来源' },
  },
  {
    id: 'created_at',
    accessorKey: 'created_at',
    header: '上传时间',
    meta: {
      headerClass: 'w-32 text-right sm:w-40',
      cellClass: 'text-right text-xs text-muted-foreground sm:text-sm',
      toggleLabel: '上传时间',
    },
  },
]

const { uploading, progress, error: uploadError, duplicates, upload } = useMediaUpload(() => {
  if (page.value !== 1) page.value = 1
  else void loadAssets()
})

onMounted(() => {
  void loadAssets()
})

// 过滤条件变化时回到第一页；翻页与每页数量变化时直接重新请求。
useDebouncedWatch([keyword, provider, mime], () => {
  if (page.value !== 1) page.value = 1
  else void loadAssets()
}, 250)

watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  void loadAssets()
})

onBeforeUnmount(() => {
  if (copyTimer) clearTimeout(copyTimer)
  if (previewTimer) clearTimeout(previewTimer)
})

async function loadAssets() {
  loading.value = true
  error.value = ''
  try {
    const pageData = await mediaApi.list({
      page: page.value,
      page_size: pageSize.value,
      keyword: keyword.value.trim() || undefined,
      provider: provider.value || undefined,
      mime: mime.value || undefined,
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
  input.value = ''
}

// 拖拽上传：用计数器跟踪 enter/leave，避免子元素间移动导致闪烁。
function handleDragEnter(event: DragEvent) {
  if (!hasFiles(event)) return
  dragDepth += 1
  dragOver.value = true
}

function handleDragLeave(event: DragEvent) {
  if (!hasFiles(event)) return
  dragDepth = Math.max(0, dragDepth - 1)
  if (dragDepth === 0) dragOver.value = false
}

function handleDrop(event: DragEvent) {
  dragDepth = 0
  dragOver.value = false
  const files = event.dataTransfer?.files
  if (files?.length) void upload(files)
}

function hasFiles(event: DragEvent) {
  return Array.from(event.dataTransfer?.types ?? []).includes('Files')
}

async function openDetail(asset: MediaAsset) {
  detail.value = asset
  detailAlt.value = asset.alt
  detailName.value = asset.original_name
  detailError.value = ''
  deleteBlockedUsages.value = []
  copied.value = false
  detailOpen.value = true
  await loadUsages(asset.id)
}

async function loadUsages(assetId: number) {
  usagesLoading.value = true
  usagesError.value = ''
  try {
    usages.value = await mediaApi.listUsages(assetId)
  } catch (requestError) {
    usagesError.value = getApiError(requestError, '引用列表加载失败')
  } finally {
    usagesLoading.value = false
  }
}

async function saveDetail() {
  const asset = detail.value
  if (!asset || detailSaving.value) return
  detailSaving.value = true
  detailError.value = ''
  try {
    const updated = await mediaApi.update(asset.id, {
      alt: detailAlt.value,
      original_name: detailName.value.trim() || undefined,
    })
    detail.value = updated
    assets.value = assets.value.map((item) => (item.id === updated.id ? updated : item))
    toast.success('媒体信息已保存')
  } catch (requestError) {
    detailError.value = getApiError(requestError, '保存失败')
  } finally {
    detailSaving.value = false
  }
}

async function copyUrl() {
  const asset = detail.value
  if (!asset) return
  try {
    await navigator.clipboard.writeText(asset.url)
  } catch {
    // 剪贴板 API 不可用（如非安全上下文）时退化为选中提示，不阻塞使用。
  }
  copied.value = true
  if (copyTimer) clearTimeout(copyTimer)
  copyTimer = setTimeout(() => {
    copied.value = false
  }, 2000)
}

function isAssetSelected(assetId: number) {
  return Boolean(selection.value[String(assetId)])
}

function toggleAssetSelected(assetId: number, selected: boolean) {
  const key = String(assetId)
  if (selected) selection.value[key] = true
  else delete selection.value[key]
}

function clearSelection() {
  selection.value = {}
}

function requestBatchDelete() {
  if (selectedIds.value.length === 0) return
  batchDeleteConfirmOpen.value = true
}

async function confirmBatchDelete() {
  const ids = selectedIds.value
  if (ids.length === 0 || batchDeleting.value) return
  batchDeleting.value = true
  try {
    const result = await mediaApi.batchDelete(ids)
    batchDeleteConfirmOpen.value = false
    clearSelection()
    detailOpen.value = false
    detail.value = null
    if (assets.value.length <= ids.length && page.value > 1) page.value -= 1
    else await loadAssets()
    if (result.deleted.length > 0) {
      toast.success(`已删除 ${result.deleted.length} 个文件`)
    }
    if (result.referenced.length > 0) {
      toast.warning(`${result.referenced.length} 个文件仍被引用，未删除：${result.referenced.map((id) => `#${id}`).join('、')}`)
    }
    if (result.not_found.length > 0) {
      toast.warning(`${result.not_found.length} 个文件已不存在，已忽略`)
    }
  } catch (requestError) {
    toast.error(getApiError(requestError, '批量删除失败'))
    batchDeleteConfirmOpen.value = false
  } finally {
    batchDeleting.value = false
  }
}

function requestDelete() {
  deleteBlockedUsages.value = []
  deleteConfirmOpen.value = true
}

async function confirmDelete() {
  const asset = detail.value
  if (!asset || deleting.value) return
  deleting.value = true
  try {
    await mediaApi.remove(asset.id)
    deleteConfirmOpen.value = false
    detailOpen.value = false
    detail.value = null
    if (assets.value.length === 1 && page.value > 1) page.value -= 1
    else await loadAssets()
    toast.success('文件已删除')
  } catch (requestError) {
    // 409 MEDIA_IN_USE：展示引用位置并禁止删除，由用户先解除引用。
    const inUse = getMediaInUseUsages(requestError)
    if (inUse) {
      deleteBlockedUsages.value = inUse
      usages.value = inUse
      deleteConfirmOpen.value = false
    } else {
      toast.error(getApiError(requestError, '删除失败'))
      deleteConfirmOpen.value = false
    }
  } finally {
    deleting.value = false
  }
}

function dimensionText(asset: MediaAsset) {
  if (asset.width === null || asset.height === null) return '尺寸探测中'
  return `${asset.width} × ${asset.height}`
}
</script>

<template>
  <BasicPage title="媒体库" description="管理站点图片资产：上传、编辑替代文本、查看引用与清理。" sticky>
      <template #actions>
        <Button :disabled="uploading" @click="triggerUpload">
          <LoaderCircleIcon v-if="uploading" class="animate-spin" />
          <UploadIcon v-else />
          {{ uploading ? `上传中 ${progress}%` : '上传文件' }}
        </Button>
      </template>

    <input
      ref="fileInput"
      type="file"
      class="hidden"
      multiple
      :accept="MEDIA_UPLOAD_ACCEPT"
      @change="handleFileChange"
    />

    <section
      aria-label="媒体资产列表"
      class="relative"
      @dragenter.prevent="handleDragEnter"
      @dragover.prevent
      @dragleave.prevent="handleDragLeave"
      @drop.prevent="handleDrop"
    >
      <div
        v-if="dragOver"
        class="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-lg border-2 border-dashed border-primary bg-primary/10"
      >
        <p class="flex items-center gap-2 text-sm font-medium text-primary">
          <UploadIcon class="size-4" />
          松开以上传文件（最多 5 个，单个不超过 5MB）
        </p>
      </div>

      <Card class="gap-0 overflow-hidden py-0 shadow-none">
        <CardHeader class="gap-3 border-b p-3 sm:p-4">
          <div class="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div class="relative w-full lg:max-w-sm">
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
            <div class="flex items-center gap-2">
              <Button
                variant="outline"
                size="icon-sm"
                aria-label="网格视图"
                :aria-pressed="viewMode === 'grid'"
                :class="viewMode === 'grid' ? 'bg-accent' : ''"
                @click="viewMode = 'grid'"
              >
                <LayoutGridIcon />
              </Button>
              <Button
                variant="outline"
                size="icon-sm"
                aria-label="列表视图"
                :aria-pressed="viewMode === 'list'"
                :class="viewMode === 'list' ? 'bg-accent' : ''"
                @click="viewMode = 'list'"
              >
                <ListIcon />
              </Button>
            </div>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <select
              v-model="provider"
              aria-label="按存储来源筛选"
              class="flex h-8 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
            >
              <option value="">全部来源</option>
              <option value="local">本地存储</option>
              <option value="s3">S3</option>
              <option value="legacy_url">旧站链接</option>
            </select>
            <select
              v-model="mime"
              aria-label="按文件类型筛选"
              class="flex h-8 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
            >
              <option value="">全部类型</option>
              <option value="image/jpeg">JPEG</option>
              <option value="image/png">PNG</option>
              <option value="image/gif">GIF</option>
              <option value="image/webp">WebP</option>
              <option value="image/bmp">BMP</option>
            </select>
            <AppDataTableColumnToggle v-if="viewMode === 'list'" :table="tableRef?.table" class="ml-auto" />
          </div>
          <div v-if="uploading" class="space-y-1">
            <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
              <div class="h-full bg-primary transition-all" :style="{ width: `${progress}%` }" />
            </div>
            <p class="text-xs text-muted-foreground">正在上传 {{ progress }}%</p>
          </div>
          <p v-if="uploadError" role="alert" class="flex items-center gap-1.5 text-xs font-medium text-destructive">
            <CircleAlertIcon class="size-3.5 shrink-0" />
            {{ uploadError }}
          </p>
          <p v-else-if="duplicates.length" class="text-xs text-muted-foreground">
            {{ duplicates.length }} 个文件与已有资产内容相同，已复用原文件（{{ duplicates.map((asset) => `#${asset.duplicate_of}`).join('、') }}）。
          </p>
        </CardHeader>

        <CardContent class="p-0" :aria-busy="loading">
          <div v-if="loading" class="flex h-48 items-center justify-center text-sm text-muted-foreground">
            <LoaderCircleIcon class="mr-2 size-5 animate-spin" />
            正在加载媒体库
          </div>
          <div v-else-if="error" class="flex h-48 flex-col items-center justify-center">
            <p class="text-sm font-medium">{{ error }}</p>
            <Button variant="outline" size="sm" class="mt-3" @click="loadAssets">
              <RefreshCwIcon />
              重试
            </Button>
          </div>
          <AppEmptyState
            v-else-if="assets.length === 0"
            :title="hasActiveFilter ? '没有匹配的媒体文件' : '媒体库为空'"
            :description="hasActiveFilter ? '尝试修改关键词或筛选条件。' : '点击右上角上传，或将图片拖拽到此页面。'"
            :icon="ImagesIcon"
            class="min-h-64"
          >
            <template v-if="!hasActiveFilter" #actions>
              <Button variant="outline" :disabled="uploading" @click="triggerUpload">
                <UploadIcon />
                上传文件
              </Button>
            </template>
          </AppEmptyState>

          <ul
            v-else-if="viewMode === 'grid'"
            class="grid grid-cols-2 gap-3 p-3 sm:grid-cols-3 sm:p-4 md:grid-cols-4 xl:grid-cols-6"
          >
            <li v-for="asset in assets" :key="asset.id" class="relative">
              <button
                type="button"
                class="group w-full overflow-hidden rounded-lg border bg-background text-left outline-none transition-colors hover:border-primary focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                :class="isAssetSelected(asset.id) ? 'border-primary ring-2 ring-primary/30' : ''"
                :aria-label="`查看 ${asset.alt || asset.original_name}`"
                @click="openDetail(asset)"
              >
                <div class="relative aspect-4/3 w-full overflow-hidden bg-muted">
                  <img
                    :src="asset.url"
                    :alt="asset.alt || asset.original_name"
                    loading="lazy"
                    class="size-full object-cover transition-transform group-hover:scale-105"
                  />
                  <!-- 选择框叠在卡片左上角：点击不触发打开详情 -->
                  <Checkbox
                    :model-value="isAssetSelected(asset.id)"
                    aria-label="选择该文件"
                    class="absolute left-1.5 top-1.5 z-10 size-4 border-background/60 bg-background/80 shadow-sm"
                    @click.stop
                    @update:model-value="(value: boolean | 'indeterminate') => toggleAssetSelected(asset.id, value === true)"
                  />
                </div>
                <div class="space-y-1 px-2 py-1.5">
                  <p class="truncate text-xs font-medium">{{ asset.alt || asset.original_name }}</p>
                  <p class="flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
                    <span>{{ formatFileSize(asset.size_bytes) }}</span>
                    <Badge variant="secondary" class="px-1 py-0 text-[10px]">{{ providerText(asset.provider) }}</Badge>
                  </p>
                </div>
              </button>
            </li>
          </ul>

          <AppDataTable
            v-else
            ref="tableRef"
            v-model:selection="selection"
            :data="assets"
            :columns="mediaColumns"
            selectable
            clickable
            :row-key="(asset: MediaAsset) => asset.id"
            @row-click="openDetail"
          >
            <template #cell-file="{ row }">
              <div class="flex min-w-0 items-center gap-3 py-1">
                <img
                  :src="row.url"
                  :alt="row.alt || row.original_name"
                  loading="lazy"
                  class="size-10 shrink-0 rounded-md border object-cover"
                />
                <div class="min-w-0">
                  <p class="truncate font-medium">{{ row.alt || row.original_name }}</p>
                  <p class="mt-0.5 truncate text-xs text-muted-foreground">{{ row.original_name }}</p>
                </div>
              </div>
            </template>
            <template #cell-mime="{ row }">{{ row.mime }}</template>
            <template #cell-size="{ row }">{{ formatFileSize(row.size_bytes) }}</template>
            <template #cell-provider="{ row }">
              <Badge variant="secondary">{{ providerText(row.provider) }}</Badge>
            </template>
            <template #cell-created_at="{ row }">{{ formatMediaTime(row.created_at) }}</template>
          </AppDataTable>
        </CardContent>

        <!-- 批量操作条：选中 ≥1 个文件时显示，跨页选择保留 -->
        <div
          v-if="selectedIds.length > 0"
          role="toolbar"
          aria-label="批量操作"
          class="flex flex-wrap items-center gap-2 border-t bg-muted/50 px-4 py-2"
        >
          <p class="text-sm font-medium">已选 {{ selectedIds.length }} 个文件</p>
          <Button variant="outline" size="sm" :disabled="batchDeleting" @click="clearSelection">
            <XIcon />
            取消选择
          </Button>
          <Button variant="destructive" size="sm" :disabled="batchDeleting" @click="requestBatchDelete">
            <LoaderCircleIcon v-if="batchDeleting" class="animate-spin" />
            <Trash2Icon v-else />
            {{ batchDeleting ? '删除中' : '删除所选' }}
          </Button>
        </div>

        <CardFooter class="flex-wrap items-center justify-end gap-x-4 gap-y-2 border-t px-4 py-3">
          <AppDataTablePagination
            v-model:page="page"
            v-model:page-size="pageSize"
            :total="total"
            :loading="loading"
            unit="个文件"
            :page-size-options="[12, 24, 48]"
          />
        </CardFooter>
      </Card>
    </section>

    <Sheet :open="detailOpen" @update:open="detailOpen = $event">
      <SheetContent class="w-full overflow-y-auto sm:max-w-md">
        <SheetHeader>
          <SheetTitle>媒体详情</SheetTitle>
          <SheetDescription>查看预览、编辑替代文本与文件名、检查引用位置。</SheetDescription>
        </SheetHeader>

        <div v-if="detail" class="space-y-5 px-4 pb-6">
          <button
            type="button"
            class="overflow-hidden rounded-lg border bg-muted transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/30"
            aria-label="点击预览大图"
            @click="openPreview"
          >
            <img :src="detail.url" :alt="detail.alt || detail.original_name" class="max-h-64 w-full object-contain" />
          </button>

          <div class="space-y-1.5">
            <label for="media-detail-alt" class="text-xs font-medium">替代文本（Alt）</label>
            <Input id="media-detail-alt" v-model="detailAlt" class="h-9 text-sm" placeholder="图片描述，用于 SEO 与无障碍" :disabled="detailSaving" />
          </div>
          <div class="space-y-1.5">
            <label for="media-detail-name" class="text-xs font-medium">原始文件名</label>
            <Input id="media-detail-name" v-model="detailName" class="h-9 text-sm" :disabled="detailSaving" />
          </div>
          <div class="flex items-center gap-2">
            <Button size="sm" :disabled="detailSaving" @click="saveDetail">
              <LoaderCircleIcon v-if="detailSaving" class="animate-spin" />
              {{ detailSaving ? '正在保存' : '保存修改' }}
            </Button>
            <p v-if="detailError" role="alert" class="text-xs font-medium text-destructive">{{ detailError }}</p>
          </div>

          <div class="space-y-1.5">
            <span class="flex items-center gap-1.5 text-xs font-medium">
              <LinkIcon class="size-3.5 text-muted-foreground" />
              文件 URL
            </span>
            <div class="flex gap-2">
              <Input :model-value="detail.url" readonly class="h-9 flex-1 font-mono text-xs" aria-label="文件 URL" />
              <Button variant="outline" size="sm" @click="copyUrl">
                <CheckIcon v-if="copied" />
                <CopyIcon v-else />
                {{ copied ? '已复制' : '复制' }}
              </Button>
            </div>
          </div>

          <dl class="divide-y overflow-hidden rounded-lg border text-xs">
            <div class="flex items-center justify-between gap-3 px-3 py-2.5">
              <dt class="text-muted-foreground">类型</dt>
              <dd class="font-medium">{{ detail.mime }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 px-3 py-2.5">
              <dt class="text-muted-foreground">大小</dt>
              <dd class="font-medium">{{ formatFileSize(detail.size_bytes) }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 px-3 py-2.5">
              <dt class="text-muted-foreground">尺寸</dt>
              <dd class="font-medium">{{ dimensionText(detail) }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 px-3 py-2.5">
              <dt class="text-muted-foreground">来源</dt>
              <dd class="font-medium">{{ providerText(detail.provider) }}</dd>
            </div>
            <div class="flex items-center justify-between gap-3 px-3 py-2.5">
              <dt class="text-muted-foreground">上传时间</dt>
              <dd class="font-medium">{{ formatMediaTime(detail.created_at) }}</dd>
            </div>
          </dl>

          <section class="space-y-2" aria-label="引用位置">
            <h3 class="text-xs font-medium">引用位置（{{ usageCount }}）</h3>
            <p v-if="usagesLoading" class="flex items-center gap-1.5 text-xs text-muted-foreground">
              <LoaderCircleIcon class="size-3.5 animate-spin" />
              正在加载引用
            </p>
            <p v-else-if="usagesError" role="alert" class="text-xs font-medium text-destructive">{{ usagesError }}</p>
            <p v-else-if="usages.length === 0" class="text-xs text-muted-foreground">暂无引用，可以安全删除。</p>
            <ul v-else class="divide-y overflow-hidden rounded-lg border text-xs">
              <li v-for="usage in usages" :key="usage.id" class="flex items-center justify-between gap-3 px-3 py-2">
                <span>{{ usageTargetText(usage.target_type) }} #{{ usage.target_id }}</span>
                <span class="text-muted-foreground">{{ formatMediaTime(usage.created_at) }}</span>
              </li>
            </ul>
          </section>

          <div
            v-if="deleteBlockedUsages.length"
            role="alert"
            class="space-y-1.5 rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-xs"
          >
            <p class="flex items-center gap-1.5 font-medium text-destructive">
              <TriangleAlertIcon class="size-3.5" />
              无法删除：文件仍被引用
            </p>
            <p class="leading-5 text-muted-foreground">
              请先在上述引用位置中移除该文件，然后再删除。
            </p>
          </div>

          <Button variant="destructive" size="sm" :disabled="deleting" @click="requestDelete">
            <Trash2Icon />
            删除文件
          </Button>
        </div>
      </SheetContent>
    </Sheet>

    <AppConfirmDialog
      v-model:open="deleteConfirmOpen"
      :title="`删除 ${detail?.alt || detail?.original_name || '该文件'}？`"
      description="文件将被标记为删除，引用它的内容会丢失图片。物理清理由后台任务在零引用后执行。"
      confirm-label="删除"
      destructive
      :busy="deleting"
      @confirm="confirmDelete"
    />

    <AppConfirmDialog
      v-model:open="batchDeleteConfirmOpen"
      :title="`删除所选 ${selectedIds.length} 个文件？`"
      description="文件将被标记为删除；仍被文章等引用的文件会自动跳过并在结果中列出。物理清理由后台任务在零引用后执行。"
      confirm-label="删除所选"
      destructive
      :busy="batchDeleting"
      @confirm="confirmBatchDelete"
    />

    <!-- 大图预览 Lightbox（缩放 + 拖拽 + 键盘快捷键） -->
    <AppImageLightbox
      :open="previewOpen"
      :src="detail?.url || ''"
      :alt="detail?.alt || detail?.original_name"
      @update:open="handlePreviewClose"
    />
  </BasicPage>
</template>

<route lang="yaml">
meta:
  permission: content:manage
</route>
