<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import {
  FolderIcon,
  LinkIcon,
  LoaderCircleIcon,
  PencilIcon,
  PlusIcon,
  SearchIcon,
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
import { BasicPage } from '@/components/global-layout'
import {
  linksApi,
  type Link,
  type LinkCategory,
  type LinkStatus,
} from '@/modules/links/api/links'
import { validateLinkIconUrl, validateLinkTitle, validateLinkUrl } from '@/modules/links/validation'
import { getApiError } from '@/shared/api/client'
import { toast } from 'vue-sonner'
import { useDebouncedWatch } from '@/composables/use-debounced-watch'

type StatusFilter = 'all' | LinkStatus

const status = ref<StatusFilter>('all')
const categoryFilter = ref<number | null>(null)
const keyword = ref('')
const page = ref(1)
const pageSize = ref(20)
const links = ref<Link[]>([])
const total = ref(0)
const loading = ref(true)
const error = ref('')

const categories = ref<LinkCategory[]>([])

// 编辑 Dialog
const dialogOpen = ref(false)
const editing = ref<Link | null>(null)
const form = reactive({
  category_id: null as number | null,
  title: '',
  url: '',
  icon_url: '',
  description: '',
  status: 'active' as LinkStatus,
  sort_order: 0,
})
const formError = ref('')
const saving = ref(false)
const newCategoryName = ref('')
const creatingCategory = ref(false)

// 删除确认
const deleteConfirmOpen = ref(false)
const pendingDelete = ref<Link | null>(null)
const deleting = ref(false)

const statusMeta: Record<LinkStatus, { label: string, variant: 'default' | 'secondary' }> = {
  active: { label: '启用', variant: 'default' },
  inactive: { label: '停用', variant: 'secondary' },
}

async function loadCategories() {
  try {
    categories.value = await linksApi.listCategories()
  }
  catch (e) {
    toast.error(getApiError(e, '分类加载失败'))
  }
}

async function loadLinks() {
  loading.value = true
  error.value = ''
  try {
    const result = await linksApi.list({
      page: page.value,
      page_size: pageSize.value,
      status: status.value === 'all' ? undefined : status.value,
      category_id: categoryFilter.value ?? undefined,
      keyword: keyword.value.trim() || undefined,
    })
    links.value = result.items
    total.value = result.total
  }
  catch (e) {
    error.value = getApiError(e, '加载友情链接失败')
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
    loadLinks()
}

function handleCategoryFilter(next: string) {
  categoryFilter.value = next === '' ? null : Number(next)
  const needPageReset = page.value !== 1
  page.value = 1
  if (!needPageReset)
    loadLinks()
}

useDebouncedWatch(keyword, () => {
  // page 非 1 时由下方 watch 触发加载，避免重复请求。
  if (page.value !== 1)
    page.value = 1
  else
    loadLinks()
}, 300)

// 翻页与每页数量变化时重新请求；pageSize 变化且不在第一页时先回到第一页（由 page 变化触发加载）。
watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadLinks()
})

onMounted(() => {
  loadLinks()
  loadCategories()
})

function categoryName(id: number | null) {
  if (id === null) return '未分类'
  return categories.value.find(category => category.id === id)?.name ?? `分类 #${id}`
}

function openCreate() {
  editing.value = null
  form.category_id = null
  form.title = ''
  form.url = ''
  form.icon_url = ''
  form.description = ''
  form.status = 'active'
  form.sort_order = 0
  formError.value = ''
  dialogOpen.value = true
}

function openEdit(link: Link) {
  editing.value = link
  form.category_id = link.category_id
  form.title = link.title
  form.url = link.url
  form.icon_url = link.icon_url ?? ''
  form.description = link.description
  form.status = link.status
  form.sort_order = link.sort_order
  formError.value = ''
  dialogOpen.value = true
}

function handleDialogClose(open: boolean) {
  if (!saving.value) dialogOpen.value = open
}

async function handleSave() {
  formError.value = validateLinkTitle(form.title)
    || validateLinkUrl(form.url)
    || validateLinkIconUrl(form.icon_url)
  if (formError.value || saving.value) return
  saving.value = true
  try {
    const payload = {
      category_id: form.category_id,
      title: form.title.trim(),
      url: form.url.trim(),
      icon_url: form.icon_url.trim() || null,
      description: form.description.trim(),
      status: form.status,
      sort_order: form.sort_order,
    }
    if (editing.value)
      await linksApi.update(editing.value.id, payload)
    else
      await linksApi.create(payload)
    dialogOpen.value = false
    toast.success(editing.value ? '友链已更新' : '友链已创建')
    await loadLinks()
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
    const category = await linksApi.createCategory({ name })
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

function confirmDelete(link: Link) {
  pendingDelete.value = link
  deleteConfirmOpen.value = true
}

async function handleDelete() {
  if (!pendingDelete.value) return
  deleting.value = true
  try {
    await linksApi.remove(pendingDelete.value.id)
    deleteConfirmOpen.value = false
    pendingDelete.value = null
    toast.success('友链已删除')
    await loadLinks()
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
  <BasicPage title="友情链接" description="管理公开站展示的友情链接，按分类分组。">
    <template #actions>
      <Button @click="openCreate">
        <PlusIcon class="size-4" />
        新增友链
      </Button>
    </template>

    <Card>
      <CardHeader class="gap-4">
        <div class="relative max-w-xs flex-1">
          <SearchIcon class="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            v-model="keyword"
            placeholder="搜索友链名称或 URL"
            class="h-8 pl-8 text-sm"
            @keyup.enter="page = 1; loadLinks()"
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
            <TabsTrigger class="flex-1 lg:flex-none" value="active">启用</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="inactive">停用</TabsTrigger>
          </TabsList>
        </Tabs>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="error" class="border-b px-4 py-3 text-sm text-destructive">{{ error }}</p>

        <div v-if="loading" class="space-y-2 p-4">
          <Skeleton v-for="index in 5" :key="index" class="h-12 w-full" />
        </div>

        <AppEmptyState
          v-else-if="links.length === 0"
          :icon="LinkIcon"
          :title="hasFilter ? '没有匹配的友链' : '还没有友情链接'"
          :description="hasFilter ? '尝试修改筛选条件。' : '点击「新增友链」添加第一个链接。'"
        />

        <ul v-else class="divide-y">
          <li
            v-for="link in links"
            :key="link.id"
            class="flex items-center gap-3 px-4 py-3"
          >
            <img
              v-if="link.icon_url"
              :src="link.icon_url"
              :alt="`${link.title} 图标`"
              class="size-8 shrink-0 rounded border object-cover"
              loading="lazy"
            />
            <div v-else class="flex size-8 shrink-0 items-center justify-center rounded border bg-muted text-muted-foreground">
              <LinkIcon class="size-4" />
            </div>
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-sm font-medium">{{ link.title }}</span>
                <Badge :variant="statusMeta[link.status].variant">
                  {{ statusMeta[link.status].label }}
                </Badge>
                <Badge variant="outline">
                  <FolderIcon class="mr-1 size-3" />
                  {{ categoryName(link.category_id) }}
                </Badge>
              </div>
              <p class="mt-0.5 truncate text-xs text-muted-foreground">
                <a :href="link.url" target="_blank" rel="noopener noreferrer" class="hover:underline">{{ link.url }}</a>
                <template v-if="link.description"> · {{ link.description }}</template>
              </p>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              <Button variant="ghost" size="icon-sm" :aria-label="`编辑友链：${link.title}`" @click="openEdit(link)">
                <PencilIcon />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                class="text-destructive hover:text-destructive"
                :aria-label="`删除友链：${link.title}`"
                @click="confirmDelete(link)"
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
          unit="个友链"
        />
      </CardFooter>
    </Card>

    <!-- 创建/编辑 Dialog -->
    <Dialog :open="dialogOpen" @update:open="handleDialogClose">
      <DialogContent class="max-w-lg">
        <DialogHeader>
          <DialogTitle>{{ editing ? '编辑友链' : '新增友链' }}</DialogTitle>
          <DialogDescription>URL 仅支持 http/https。</DialogDescription>
        </DialogHeader>

        <div class="grid gap-4 py-2">
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <Label for="link-title">名称</Label>
              <Input id="link-title" v-model="form.title" placeholder="友链名称" maxlength="120" :disabled="saving" />
            </div>
            <div class="grid gap-2">
              <Label for="link-category">分类</Label>
              <select
                id="link-category"
                v-model="form.category_id"
                class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                :disabled="saving"
              >
                <option :value="null">未分类</option>
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
          </div>
          <div class="grid gap-2">
            <Label for="link-url">URL</Label>
            <Input id="link-url" v-model="form.url" type="url" placeholder="https://example.com" :disabled="saving" />
          </div>
          <div class="grid gap-2">
            <Label for="link-icon">
              图标 URL
              <span class="text-xs text-muted-foreground">（可选）</span>
            </Label>
            <Input id="link-icon" v-model="form.icon_url" type="url" placeholder="https://example.com/icon.png" :disabled="saving" />
          </div>
          <div class="grid gap-2">
            <Label for="link-description">
              描述
              <span class="text-xs text-muted-foreground">（可选）</span>
            </Label>
            <Textarea id="link-description" v-model="form.description" rows="2" :disabled="saving" />
          </div>
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <Label for="link-status">状态</Label>
              <select
                id="link-status"
                v-model="form.status"
                class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                :disabled="saving"
              >
                <option value="active">启用</option>
                <option value="inactive">停用</option>
              </select>
            </div>
            <div class="grid gap-2">
              <Label for="link-sort">排序</Label>
              <Input id="link-sort" v-model.number="form.sort_order" type="number" :disabled="saving" />
            </div>
          </div>
          <p v-if="formError" role="alert" class="text-sm text-destructive">{{ formError }}</p>
        </div>

        <DialogFooter>
          <Button variant="outline" :disabled="saving" @click="dialogOpen = false">取消</Button>
          <Button :disabled="saving" @click="handleSave">
            {{ saving ? '保存中…' : '保存' }}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <AppConfirmDialog
      v-model:open="deleteConfirmOpen"
      title="删除友链"
      :description="`确定要删除友链「${pendingDelete?.title}」吗？该操作不可恢复。`"
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
