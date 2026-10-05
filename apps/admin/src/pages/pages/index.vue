<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import dayjs from 'dayjs'
import {
  FileIcon,
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
import { AppDataTablePagination } from '@/shared/components/data-table'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import MarkdownEditor from '@/shared/components/MarkdownEditor.vue'
import { uploadEditorImages } from '@/shared/components/markdown-editor-upload'
import { BasicPage } from '@/components/global-layout'
import { pagesApi, type CustomPage, type PageStatus } from '@/modules/pages/api/pages'
import { validatePageSlug, validatePageTitle } from '@/modules/pages/validation'
import { getApiError } from '@/shared/api/client'
import { toast } from 'vue-sonner'
import { useDebouncedWatch } from '@/composables/use-debounced-watch'

type StatusFilter = 'all' | PageStatus

const status = ref<StatusFilter>('all')
const keyword = ref('')
const page = ref(1)
const pageSize = ref(20)
const pages = ref<CustomPage[]>([])
const total = ref(0)
const loading = ref(true)
const error = ref('')

// 编辑 Dialog
const dialogOpen = ref(false)
const editing = ref<CustomPage | null>(null)
const form = reactive({
  slug: '',
  title: '',
  content_markdown: '',
  status: 'draft' as PageStatus,
  sort_order: 0,
})
const formError = ref('')
const saving = ref(false)

// 删除确认
const deleteConfirmOpen = ref(false)
const pendingDelete = ref<CustomPage | null>(null)
const deleting = ref(false)

const statusMeta: Record<PageStatus, { label: string, variant: 'default' | 'secondary' }> = {
  draft: { label: '草稿', variant: 'secondary' },
  published: { label: '已发布', variant: 'default' },
}

async function loadPages() {
  loading.value = true
  error.value = ''
  try {
    const result = await pagesApi.list({
      page: page.value,
      page_size: pageSize.value,
      status: status.value === 'all' ? undefined : status.value,
      keyword: keyword.value.trim() || undefined,
    })
    pages.value = result.items
    total.value = result.total
  }
  catch (e) {
    error.value = getApiError(e, '加载页面失败')
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
    loadPages()
}

useDebouncedWatch(keyword, () => {
  // page 非 1 时由下方 watch 触发加载，避免重复请求。
  if (page.value !== 1)
    page.value = 1
  else
    loadPages()
}, 300)

// 翻页与每页数量变化时重新请求；pageSize 变化且不在第一页时先回到第一页（由 page 变化触发加载）。
watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadPages()
})

onMounted(loadPages)

function formatTime(value: string) {
  const parsed = dayjs(value)
  return parsed.isValid() ? parsed.format('YYYY-MM-DD HH:mm') : '-'
}

function openCreate() {
  editing.value = null
  form.slug = ''
  form.title = ''
  form.content_markdown = ''
  form.status = 'draft'
  form.sort_order = 0
  formError.value = ''
  dialogOpen.value = true
}

function openEdit(item: CustomPage) {
  editing.value = item
  form.slug = item.slug
  form.title = item.title
  form.content_markdown = item.content_markdown
  form.status = item.status
  form.sort_order = item.sort_order
  formError.value = ''
  dialogOpen.value = true
}

function handleDialogClose(open: boolean) {
  if (!saving.value) dialogOpen.value = open
}

async function handleSave() {
  formError.value = validatePageTitle(form.title) || validatePageSlug(form.slug)
  if (formError.value || saving.value) return
  saving.value = true
  try {
    const payload = {
      slug: form.slug.trim(),
      title: form.title.trim(),
      content_markdown: form.content_markdown,
      status: form.status,
      sort_order: form.sort_order,
    }
    if (editing.value)
      await pagesApi.update(editing.value.id, payload)
    else
      await pagesApi.create(payload)
    dialogOpen.value = false
    toast.success(editing.value ? '页面已更新' : '页面已创建')
    await loadPages()
  }
  catch (e) {
    formError.value = getApiError(e, '保存失败')
  }
  finally {
    saving.value = false
  }
}

function confirmDelete(item: CustomPage) {
  pendingDelete.value = item
  deleteConfirmOpen.value = true
}

async function handleDelete() {
  if (!pendingDelete.value) return
  deleting.value = true
  try {
    await pagesApi.remove(pendingDelete.value.id)
    deleteConfirmOpen.value = false
    pendingDelete.value = null
    toast.success('页面已删除')
    await loadPages()
  }
  catch (e) {
    toast.error(getApiError(e, '删除失败'))
  }
  finally {
    deleting.value = false
  }
}

const hasFilter = computed(() => status.value !== 'all' || Boolean(keyword.value.trim()))
</script>

<template>
  <BasicPage title="页面管理" description="管理自定义页面（关于、友链说明等），发布后可在公开站访问。">
    <template #actions>
      <Button @click="openCreate">
        <PlusIcon class="size-4" />
        新增页面
      </Button>
    </template>

    <Card>
      <CardHeader class="gap-4">
        <div class="relative max-w-xs flex-1">
          <SearchIcon class="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            v-model="keyword"
            placeholder="搜索标题或 Slug"
            class="h-8 pl-8 text-sm"
            @keyup.enter="page = 1; loadPages()"
          />
        </div>
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
          v-else-if="pages.length === 0"
          :icon="FileIcon"
          :title="hasFilter ? '没有匹配的页面' : '还没有自定义页面'"
          :description="hasFilter ? '尝试修改关键词或切换状态筛选。' : '点击「新增页面」创建第一个页面。'"
        />

        <ul v-else class="divide-y">
          <li
            v-for="item in pages"
            :key="item.id"
            class="flex items-center gap-3 px-4 py-3"
          >
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-sm font-medium">{{ item.title }}</span>
                <Badge :variant="statusMeta[item.status].variant">
                  {{ statusMeta[item.status].label }}
                </Badge>
                <Badge variant="outline" class="font-mono">/{{ item.slug }}</Badge>
              </div>
              <p class="mt-1 text-xs text-muted-foreground">
                排序 {{ item.sort_order }} · 更新于 {{ formatTime(item.updated_at) }}
              </p>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              <Button variant="ghost" size="icon-sm" :aria-label="`编辑页面：${item.title}`" @click="openEdit(item)">
                <PencilIcon />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                class="text-destructive hover:text-destructive"
                :aria-label="`删除页面：${item.title}`"
                @click="confirmDelete(item)"
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
          unit="个页面"
        />
      </CardFooter>
    </Card>

    <!-- 创建/编辑 Dialog -->
    <Dialog :open="dialogOpen" @update:open="handleDialogClose">
      <DialogContent class="flex h-[min(720px,calc(100dvh-2rem))] w-[calc(100%-1rem)] max-w-3xl! flex-col gap-0 overflow-hidden p-0">
        <DialogHeader class="border-b px-4 py-4 text-left sm:px-5">
          <DialogTitle class="text-base">{{ editing ? '编辑页面' : '新增页面' }}</DialogTitle>
          <DialogDescription class="mt-0.5">正文支持 Markdown，保存后由服务端渲染。</DialogDescription>
        </DialogHeader>

        <div class="min-h-0 flex-1 space-y-4 overflow-y-auto p-4 sm:p-5">
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <Label for="page-title">标题</Label>
              <Input id="page-title" v-model="form.title" placeholder="页面标题" maxlength="120" :disabled="saving" />
            </div>
            <div class="grid gap-2">
              <Label for="page-slug">Slug</Label>
              <Input id="page-slug" v-model="form.slug" placeholder="about" maxlength="160" :disabled="saving" />
            </div>
          </div>
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <Label for="page-status">状态</Label>
              <select
                id="page-status"
                v-model="form.status"
                class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                :disabled="saving"
              >
                <option value="draft">草稿</option>
                <option value="published">已发布</option>
              </select>
            </div>
            <div class="grid gap-2">
              <Label for="page-sort">排序</Label>
              <Input id="page-sort" v-model.number="form.sort_order" type="number" :disabled="saving" />
            </div>
          </div>
          <div class="grid gap-2">
            <Label>正文</Label>
            <MarkdownEditor
              v-if="dialogOpen"
              v-model="form.content_markdown"
              :height="380"
              :upload="uploadEditorImages"
              placeholder="使用 Markdown 编写页面内容..."
            />
          </div>
          <p v-if="formError" role="alert" class="text-sm text-destructive">{{ formError }}</p>
        </div>

        <DialogFooter class="border-t px-4 py-3 sm:px-5">
          <Button variant="outline" :disabled="saving" @click="dialogOpen = false">取消</Button>
          <Button :disabled="saving" @click="handleSave">
            {{ saving ? '保存中…' : '保存' }}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <AppConfirmDialog
      v-model:open="deleteConfirmOpen"
      title="删除页面"
      :description="`确定要删除页面「${pendingDelete?.title}」吗？该操作不可恢复。`"
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
