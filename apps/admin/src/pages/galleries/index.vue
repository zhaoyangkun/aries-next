<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import {
  ArrowDownIcon,
  ArrowUpIcon,
  FolderIcon,
  ImageIcon,
  ImageOffIcon,
  ImagesIcon,
  LoaderCircleIcon,
  PencilIcon,
  PlusIcon,
  SearchIcon,
  StarIcon,
  Trash2Icon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardFooter, CardHeader } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Textarea } from '@/components/ui/textarea'
import { AppDataTablePagination } from '@/shared/components/data-table'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import AppMediaPicker from '@/shared/components/AppMediaPicker.vue'
import { BasicPage } from '@/components/global-layout'
import {
  galleriesApi,
  type Gallery,
  type GalleryCategory,
  type GalleryItem,
  type GalleryStatus,
} from '@/modules/galleries/api/galleries'
import { validateGalleryCategoryId, validateGallerySlug, validateGalleryTitle } from '@/modules/galleries/validation'
import type { MediaAsset } from '@/modules/media/api/media'
import { getApiError } from '@/shared/api/client'
import { toast } from 'vue-sonner'
import { useDebouncedWatch } from '@/composables/use-debounced-watch'
type StatusFilter = 'all' | GalleryStatus

const status = ref<StatusFilter>('all')
const categoryFilter = ref<number | null>(null)
const keyword = ref('')
const page = ref(1)
const pageSize = ref(20)
const galleries = ref<Gallery[]>([])
const total = ref(0)
const loading = ref(true)
const error = ref('')

const categories = ref<GalleryCategory[]>([])

// 编辑 Dialog：基本信息 + 条目管理（条目需图库已存在，新建保存后自动进入条目管理）。
const dialogOpen = ref(false)
const editing = ref<Gallery | null>(null)
const form = reactive({
  category_id: null as number | null,
  slug: '',
  title: '',
  description: '',
  status: 'draft' as GalleryStatus,
  sort_order: 0,
})
const formError = ref('')
const formNotice = ref('')
let noticeTimer: ReturnType<typeof setTimeout> | undefined

function showFormNotice(message: string) {
  formNotice.value = message
  formError.value = ''
  if (noticeTimer) clearTimeout(noticeTimer)
  noticeTimer = setTimeout(() => {
    formNotice.value = ''
  }, 5000)
}
const saving = ref(false)
const newCategoryName = ref('')
const creatingCategory = ref(false)

// 条目管理
const items = ref<GalleryItem[]>([])
const itemsLoading = ref(false)
const itemsError = ref('')
const pickerOpen = ref(false)
const itemBusyId = ref<number | null>(null)

// 删除确认
const deleteConfirmOpen = ref(false)
const pendingDelete = ref<Gallery | null>(null)
const deleting = ref(false)

const statusMeta: Record<GalleryStatus, { label: string, variant: 'default' | 'secondary' }> = {
  draft: { label: '草稿', variant: 'secondary' },
  published: { label: '已发布', variant: 'default' },
}

async function loadCategories() {
  try {
    categories.value = await galleriesApi.listCategories()
  }
  catch (e) {
    toast.error(getApiError(e, '分类加载失败'))
  }
}

async function loadGalleries() {
  loading.value = true
  error.value = ''
  try {
    const result = await galleriesApi.list({
      page: page.value,
      page_size: pageSize.value,
      status: status.value === 'all' ? undefined : status.value,
      category_id: categoryFilter.value ?? undefined,
      keyword: keyword.value.trim() || undefined,
    })
    galleries.value = result.items
    total.value = result.total
  }
  catch (e) {
    error.value = getApiError(e, '加载图库失败')
  }
  finally {
    loading.value = false
  }
}

function handleFilterChange(next: string | number) {
  status.value = next as StatusFilter
  const needPageReset = page.value !== 1
  page.value = 1
  if (!needPageReset)
    loadGalleries()
}

function handleCategoryFilter(next: string) {
  categoryFilter.value = next === '' ? null : Number(next)
  const needPageReset = page.value !== 1
  page.value = 1
  if (!needPageReset)
    loadGalleries()
}

useDebouncedWatch(keyword, () => {
  // page 非 1 时由下方 watch 触发加载，避免重复请求。
  if (page.value !== 1)
    page.value = 1
  else
    loadGalleries()
}, 300)

// 翻页与每页数量变化时重新请求；pageSize 变化且不在第一页时先回到第一页（由 page 变化触发加载）。
watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadGalleries()
})

onMounted(() => {
  loadGalleries()
  loadCategories()
})

function categoryName(id: number) {
  return categories.value.find(category => category.id === id)?.name ?? `分类 #${id}`
}

function openCreate() {
  editing.value = null
  form.category_id = categories.value[0]?.id ?? null
  form.slug = ''
  form.title = ''
  form.description = ''
  form.status = 'draft'
  form.sort_order = 0
  formError.value = ''
  formNotice.value = ''
  items.value = []
  itemsError.value = ''
  dialogOpen.value = true
}

function openEdit(gallery: Gallery) {
  editing.value = gallery
  form.category_id = gallery.category_id
  form.slug = gallery.slug
  form.title = gallery.title
  form.description = gallery.description
  form.status = gallery.status
  form.sort_order = gallery.sort_order
  formError.value = ''
  formNotice.value = ''
  dialogOpen.value = true
  void loadItems(gallery.id)
}

function handleDialogClose(open: boolean) {
  if (!saving.value) dialogOpen.value = open
}

async function handleSave() {
  formError.value = validateGalleryTitle(form.title)
    || validateGallerySlug(form.slug)
    || validateGalleryCategoryId(form.category_id)
  if (formError.value || saving.value) return
  saving.value = true
  try {
    const payload = {
      category_id: form.category_id!,
      slug: form.slug.trim(),
      title: form.title.trim(),
      description: form.description.trim(),
      cover_media_id: editing.value?.cover_media_id ?? null,
      status: form.status,
      sort_order: form.sort_order,
    }
    if (editing.value) {
      editing.value = await galleriesApi.update(editing.value.id, payload)
      await loadGalleries()
      // 编辑已有图库时保存即关闭；新建则留在 Dialog 内继续添加图片条目。
      dialogOpen.value = false
      toast.success('图库已保存')
    }
    else {
      // 新建成功后直接进入编辑态，用户可立即添加条目。
      editing.value = await galleriesApi.create(payload)
      items.value = []
      await loadGalleries()
      showFormNotice('图库已创建，可以继续添加图片条目。')
    }
  }
  catch (e) {
    formError.value = getApiError(e, '保存失败')
  }
  finally {
    saving.value = false
  }
}

async function createCategory() {
  const name = newCategoryName.value.trim()
  if (!name || creatingCategory.value) return
  creatingCategory.value = true
  formError.value = ''
  try {
    const category = await galleriesApi.createCategory({ name })
    categories.value = [...categories.value, category]
    form.category_id = category.id
    newCategoryName.value = ''
    toast.success('分类已创建')
  }
  catch (e) {
    formError.value = getApiError(e, '分类创建失败')
  }
  finally {
    creatingCategory.value = false
  }
}

async function loadItems(galleryId: number) {
  itemsLoading.value = true
  itemsError.value = ''
  try {
    // 条目响应已内联媒体摘要，无需再逐个拉取资产。
    items.value = await galleriesApi.listItems(galleryId)
  }
  catch (e) {
    itemsError.value = getApiError(e, '条目加载失败')
  }
  finally {
    itemsLoading.value = false
  }
}

async function handlePick(asset: MediaAsset) {
  const gallery = editing.value
  if (!gallery) return
  itemsError.value = ''
  try {
    // 直接使用接口返回的完整条目（含内联 media 摘要）。
    const item = await galleriesApi.addItem(gallery.id, {
      media_asset_id: asset.id,
      alt: asset.alt,
      sort_order: items.value.length,
    })
    items.value = [...items.value, item]
  }
  catch (e) {
    itemsError.value = getApiError(e, '添加条目失败')
  }
}

// 上移/下移：本地交换后立即调用原子排序接口，失败则回滚重新加载。
async function moveItem(index: number, direction: -1 | 1) {
  const gallery = editing.value
  const target = index + direction
  if (!gallery || target < 0 || target >= items.value.length) return
  const reordered = [...items.value]
  const [moved] = reordered.splice(index, 1)
  reordered.splice(target, 0, moved)
  items.value = reordered
  itemBusyId.value = moved.id
  try {
    await galleriesApi.reorderItems(gallery.id, reordered.map(item => item.id))
  }
  catch (e) {
    itemsError.value = getApiError(e, '排序保存失败')
    await loadItems(gallery.id)
  }
  finally {
    itemBusyId.value = null
  }
}

// alt/location 失焦保存：值未变化时跳过请求。
async function saveItemMeta(item: GalleryItem) {
  const original = items.value.find(entry => entry.id === item.id)
  if (!original) return
  itemBusyId.value = item.id
  itemsError.value = ''
  try {
    const updated = await galleriesApi.updateItem(item.gallery_id, item.id, {
      alt: item.alt,
      location: item.location,
    })
    items.value = items.value.map(entry => (entry.id === item.id ? updated : entry))
  }
  catch (e) {
    itemsError.value = getApiError(e, '条目保存失败')
  }
  finally {
    itemBusyId.value = null
  }
}

async function setCover(item: GalleryItem) {
  const gallery = editing.value
  if (!gallery) return
  itemsError.value = ''
  try {
    editing.value = await galleriesApi.update(gallery.id, {
      category_id: form.category_id ?? gallery.category_id,
      slug: form.slug.trim() || gallery.slug,
      title: form.title.trim() || gallery.title,
      description: form.description,
      cover_media_id: item.media_asset_id,
      status: form.status,
      sort_order: form.sort_order,
    })
    await loadGalleries()
    toast.success('封面已更新')
  }
  catch (e) {
    itemsError.value = getApiError(e, '封面设置失败')
  }
}

async function removeItem(item: GalleryItem) {
  const gallery = editing.value
  if (!gallery) return
  itemBusyId.value = item.id
  itemsError.value = ''
  try {
    await galleriesApi.removeItem(gallery.id, item.id)
    items.value = items.value.filter(entry => entry.id !== item.id)
    toast.success('条目已移除')
  }
  catch (e) {
    itemsError.value = getApiError(e, '条目删除失败')
  }
  finally {
    itemBusyId.value = null
  }
}

function confirmDelete(gallery: Gallery) {
  pendingDelete.value = gallery
  deleteConfirmOpen.value = true
}

async function handleDelete() {
  if (!pendingDelete.value) return
  deleting.value = true
  try {
    await galleriesApi.remove(pendingDelete.value.id)
    deleteConfirmOpen.value = false
    pendingDelete.value = null
    toast.success('图库已删除')
    await loadGalleries()
  }
  catch (e) {
    toast.error(getApiError(e, '删除失败'))
  }
  finally {
    deleting.value = false
  }
}

const hasFilter = computed(
  () => status.value !== 'all' || categoryFilter.value !== null || Boolean(keyword.value.trim()),
)
</script>

<template>
  <BasicPage title="图库管理" description="按分类组织相册，发布后可在公开站展示。">
    <template #actions>
      <Button @click="openCreate">
        <PlusIcon class="size-4" />
        新增图库
      </Button>
    </template>

    <Card>
      <CardHeader class="gap-4">
        <div class="relative max-w-xs flex-1">
          <SearchIcon class="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            v-model="keyword"
            placeholder="搜索图库标题或 Slug"
            class="h-8 pl-8 text-sm"
            @keyup.enter="page = 1; loadGalleries()"
          />
        </div>
        <select
          :value="categoryFilter ?? ''"
          aria-label="按分类筛选"
          class="flex h-8 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
          @change="handleCategoryFilter(($event.target as HTMLSelectElement).value)"
        >
          <option value="">全部分类</option>
          <option v-for="category in categories" :key="category.id" :value="category.id">
            {{ category.name }}
          </option>
        </select>
        <Tabs :model-value="status" class="ml-auto" @update:model-value="handleFilterChange">
          <TabsList class="w-full lg:w-auto">
            <TabsTrigger class="flex-1 lg:flex-none" value="all">全部</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="draft">草稿</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="published">已发布</TabsTrigger>
          </TabsList>
        </Tabs>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="error" class="border-b px-4 py-3 text-sm text-destructive">{{ error }}</p>

        <div v-if="loading" class="space-y-2 p-4">
          <Skeleton v-for="index in 5" :key="index" class="h-12 w-full" />
        </div>

        <AppEmptyState
          v-else-if="galleries.length === 0"
          :icon="ImagesIcon"
          :title="hasFilter ? '没有匹配的图库' : '还没有图库'"
          :description="hasFilter ? '尝试修改筛选条件。' : '点击「新增图库」创建第一个相册。'"
        />

        <ul v-else class="divide-y">
          <li
            v-for="gallery in galleries"
            :key="gallery.id"
            class="flex items-center gap-3 px-4 py-3"
          >
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-sm font-medium">{{ gallery.title }}</span>
                <Badge :variant="statusMeta[gallery.status].variant">
                  {{ statusMeta[gallery.status].label }}
                </Badge>
                <Badge variant="outline">
                  <FolderIcon class="mr-1 size-3" />
                  {{ categoryName(gallery.category_id) }}
                </Badge>
                <Badge variant="outline" class="font-mono">/{{ gallery.slug }}</Badge>
              </div>
              <p v-if="gallery.description" class="mt-1 line-clamp-1 text-xs text-muted-foreground">
                {{ gallery.description }}
              </p>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              <Button variant="ghost" size="icon-sm" :aria-label="`编辑图库：${gallery.title}`" @click="openEdit(gallery)">
                <PencilIcon />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                class="text-destructive hover:text-destructive"
                :aria-label="`删除图库：${gallery.title}`"
                @click="confirmDelete(gallery)"
              >
                <Trash2Icon />
              </Button>
            </div>
          </li>
        </ul>
      </CardContent>
      <CardFooter class="h-auto">
        <AppDataTablePagination
          v-model:page="page"
          v-model:page-size="pageSize"
          :total="total"
          :loading="loading"
          unit="个图库"
        />
      </CardFooter>
    </Card>

    <!-- 创建/编辑 Dialog：基本信息 + 条目管理 -->
    <Dialog :open="dialogOpen" @update:open="handleDialogClose">
      <DialogContent class="flex h-[min(780px,calc(100dvh-2rem))] w-[calc(100%-1rem)] max-w-3xl! flex-col gap-0 overflow-hidden p-0">
        <DialogHeader class="border-b px-4 py-4 text-left sm:px-5">
          <DialogTitle class="text-base">{{ editing ? '编辑图库' : '新增图库' }}</DialogTitle>
          <DialogDescription class="mt-0.5">
            {{ editing ? '管理基本信息与图片条目。' : '先保存基本信息，然后即可添加图片条目。' }}
          </DialogDescription>
        </DialogHeader>

        <div class="min-h-0 flex-1 space-y-5 overflow-y-auto p-4 sm:p-5">
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <Label for="gallery-title">标题</Label>
              <Input id="gallery-title" v-model="form.title" placeholder="图库标题" maxlength="120" :disabled="saving" />
            </div>
            <div class="grid gap-2">
              <Label for="gallery-slug">Slug</Label>
              <Input id="gallery-slug" v-model="form.slug" placeholder="travel-2026" maxlength="160" :disabled="saving" />
            </div>
          </div>
          <div class="grid gap-4 sm:grid-cols-3">
            <div class="grid gap-2">
              <Label for="gallery-category">分类</Label>
              <select
                id="gallery-category"
                v-model="form.category_id"
                class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                :disabled="saving"
              >
                <option :value="null">未选择分类</option>
                <option v-for="category in categories" :key="category.id" :value="category.id">
                  {{ category.name }}
                </option>
              </select>
              <div class="flex gap-2">
                <Input v-model="newCategoryName" class="h-8 text-xs" placeholder="快速新建分类" @keydown.enter.prevent="createCategory" />
                <Button type="button" size="icon-sm" variant="outline" :disabled="!newCategoryName.trim() || creatingCategory" aria-label="新建分类" @click="createCategory">
                  <LoaderCircleIcon v-if="creatingCategory" class="animate-spin" />
                  <PlusIcon v-else />
                </Button>
              </div>
            </div>
            <div class="grid gap-2">
              <Label for="gallery-status">状态</Label>
              <select
                id="gallery-status"
                v-model="form.status"
                class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                :disabled="saving"
              >
                <option value="draft">草稿</option>
                <option value="published">已发布</option>
              </select>
            </div>
            <div class="grid gap-2">
              <Label for="gallery-sort">排序</Label>
              <Input id="gallery-sort" v-model.number="form.sort_order" type="number" :disabled="saving" />
            </div>
          </div>
          <div class="grid gap-2">
            <Label for="gallery-description">描述</Label>
            <Textarea id="gallery-description" v-model="form.description" rows="2" placeholder="图库简介（可选）" :disabled="saving" />
          </div>
          <p v-if="formError" role="alert" class="text-sm text-destructive">{{ formError }}</p>
          <p v-else-if="formNotice" class="text-sm font-medium text-green-600 dark:text-green-500">{{ formNotice }}</p>

          <!-- 条目管理：仅图库保存后可用。 -->
          <section v-if="editing" class="space-y-3 border-t pt-4" aria-label="图片条目">
            <div class="flex items-center justify-between gap-2">
              <h3 class="text-sm font-medium">图片条目（{{ items.length }}）</h3>
              <Button type="button" variant="outline" size="sm" @click="pickerOpen = true">
                <ImageIcon />
                添加图片
              </Button>
            </div>
            <p v-if="itemsError" role="alert" class="text-xs font-medium text-destructive">{{ itemsError }}</p>
            <p v-if="itemsLoading" class="flex items-center gap-1.5 text-xs text-muted-foreground">
              <LoaderCircleIcon class="size-3.5 animate-spin" />
              正在加载条目
            </p>
            <AppEmptyState
              v-else-if="items.length === 0"
              :icon="ImagesIcon"
              title="还没有图片"
              description="点击「添加图片」从媒体库选择第一张图片。"
            />
            <ul v-else class="space-y-2">
              <li
                v-for="(item, index) in items"
                :key="item.id"
                class="flex items-start gap-3 rounded-md border p-2.5"
              >
                <div class="size-16 shrink-0 overflow-hidden rounded bg-muted">
                  <img
                    v-if="item.media"
                    :src="item.media.url"
                    :alt="item.alt || item.media.alt"
                    class="size-full object-cover"
                    loading="lazy"
                  />
                  <!-- 资产已被软删除（media 为 null）时显示破图占位。 -->
                  <div
                    v-else
                    class="flex size-full flex-col items-center justify-center gap-0.5 text-muted-foreground"
                    title="媒体文件已删除"
                  >
                    <ImageOffIcon class="size-5" />
                    <span class="text-[10px]">已删除</span>
                  </div>
                </div>
                <div class="grid min-w-0 flex-1 gap-1.5 sm:grid-cols-2">
                  <Input
                    v-model="item.alt"
                    class="h-8 text-xs"
                    placeholder="替代文本（alt）"
                    :disabled="itemBusyId === item.id"
                    @blur="saveItemMeta(item)"
                  />
                  <Input
                    v-model="item.location"
                    class="h-8 text-xs"
                    placeholder="拍摄地点（可选）"
                    :disabled="itemBusyId === item.id"
                    @blur="saveItemMeta(item)"
                  />
                </div>
                <div class="flex shrink-0 flex-col items-center gap-0.5">
                  <div class="flex items-center gap-0.5">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      :disabled="index === 0 || itemBusyId !== null"
                      :aria-label="`上移条目 ${item.alt || item.id}`"
                      @click="moveItem(index, -1)"
                    >
                      <ArrowUpIcon />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      :disabled="index === items.length - 1 || itemBusyId !== null"
                      :aria-label="`下移条目 ${item.alt || item.id}`"
                      @click="moveItem(index, 1)"
                    >
                      <ArrowDownIcon />
                    </Button>
                  </div>
                  <div class="flex items-center gap-0.5">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      :class="editing.cover_media_id === item.media_asset_id ? 'text-amber-500' : ''"
                      :disabled="itemBusyId !== null"
                      :aria-label="`设为封面：${item.alt || item.id}`"
                      :title="editing.cover_media_id === item.media_asset_id ? '当前封面' : '设为封面'"
                      @click="setCover(item)"
                    >
                      <StarIcon :fill="editing.cover_media_id === item.media_asset_id ? 'currentColor' : 'none'" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      class="text-destructive hover:text-destructive"
                      :disabled="itemBusyId !== null"
                      :aria-label="`移除条目 ${item.alt || item.id}`"
                      @click="removeItem(item)"
                    >
                      <Trash2Icon />
                    </Button>
                  </div>
                </div>
              </li>
            </ul>
          </section>
        </div>

        <DialogFooter class="border-t px-4 py-3 sm:px-5">
          <Button variant="outline" :disabled="saving" @click="dialogOpen = false">关闭</Button>
          <Button :disabled="saving" @click="handleSave">
            <LoaderCircleIcon v-if="saving" class="animate-spin" />
            {{ saving ? '保存中…' : '保存基本信息' }}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <AppMediaPicker v-model:open="pickerOpen" @select="handlePick" />

    <AppConfirmDialog
      v-model:open="deleteConfirmOpen"
      title="删除图库"
      :description="`确定要删除图库「${pendingDelete?.title}」吗？其中的条目会一并删除，媒体文件保留在媒体库。`"
      confirm-label="删除"
      destructive
      :busy="deleting"
      @confirm="handleDelete"
    />
  </BasicPage>
</template>

<route lang="yaml">
meta:
  permission: content:manage
</route>
