<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import dayjs from 'dayjs'
import {
  GlobeIcon,
  LoaderCircleIcon,
  LockIcon,
  NotebookPenIcon,
  PencilIcon,
  PlusIcon,
  Trash2Icon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardFooter, CardHeader } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { AppDataTablePagination } from '@/shared/components/data-table'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import MarkdownEditor from '@/shared/components/MarkdownEditor.vue'
import { uploadEditorImages } from '@/shared/components/markdown-editor-upload'
import { BasicPage } from '@/components/global-layout'
import { journalsApi, type Journal, type JournalVisibility } from '@/modules/journals/api/journals'
import { getApiError } from '@/shared/api/client'

type VisibilityFilter = 'all' | JournalVisibility

const visibility = ref<VisibilityFilter>('all')
const page = ref(1)
const pageSize = ref(20)
const journals = ref<Journal[]>([])
const total = ref(0)
const loading = ref(true)
const error = ref('')
const operationError = ref('')

// 创建/编辑 Dialog
const dialogOpen = ref(false)
const editing = ref<Journal | null>(null)
const formContent = ref('')
const formVisibility = ref<JournalVisibility>('public')
const formError = ref('')
const saving = ref(false)

// 删除确认
const deleteConfirmOpen = ref(false)
const pendingDelete = ref<Journal | null>(null)
const deleting = ref(false)

async function loadJournals() {
  loading.value = true
  error.value = ''
  try {
    const result = await journalsApi.list({
      page: page.value,
      page_size: pageSize.value,
      visibility: visibility.value === 'all' ? undefined : visibility.value,
    })
    journals.value = result.items
    total.value = result.total
  }
  catch (e) {
    error.value = getApiError(e, '加载日志失败')
  }
  finally {
    loading.value = false
  }
}

function handleFilterChange(next: string | number) {
  visibility.value = next as VisibilityFilter
  const needPageReset = page.value !== 1
  page.value = 1
  if (!needPageReset)
    loadJournals()
}

// 翻页与每页数量变化时重新请求；pageSize 变化且不在第一页时先回到第一页（由 page 变化触发加载）。
watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadJournals()
})

onMounted(loadJournals)

function formatTime(value: string) {
  const parsed = dayjs(value)
  return parsed.isValid() ? parsed.format('YYYY-MM-DD HH:mm') : '-'
}

function openCreate() {
  editing.value = null
  formContent.value = ''
  formVisibility.value = 'public'
  formError.value = ''
  dialogOpen.value = true
}

function openEdit(journal: Journal) {
  editing.value = journal
  formContent.value = journal.content_markdown
  formVisibility.value = journal.visibility
  formError.value = ''
  dialogOpen.value = true
}

function handleDialogClose(open: boolean) {
  if (!saving.value) dialogOpen.value = open
}

async function handleSave() {
  if (!formContent.value.trim()) {
    formError.value = '请输入日志内容'
    return
  }
  if (saving.value) return
  saving.value = true
  formError.value = ''
  try {
    const payload = { content_markdown: formContent.value, visibility: formVisibility.value }
    if (editing.value)
      await journalsApi.update(editing.value.id, payload)
    else
      await journalsApi.create(payload)
    dialogOpen.value = false
    await loadJournals()
  }
  catch (e) {
    formError.value = getApiError(e, '保存失败')
  }
  finally {
    saving.value = false
  }
}

function confirmDelete(journal: Journal) {
  pendingDelete.value = journal
  deleteConfirmOpen.value = true
}

async function handleDelete() {
  if (!pendingDelete.value) return
  deleting.value = true
  try {
    await journalsApi.remove(pendingDelete.value.id)
    deleteConfirmOpen.value = false
    pendingDelete.value = null
    await loadJournals()
  }
  catch (e) {
    operationError.value = getApiError(e, '删除失败')
  }
  finally {
    deleting.value = false
  }
}
</script>

<template>
  <BasicPage title="日志" description="记录短内容时间线，私密日志仅管理端可见。">
    <template #actions>
      <Button @click="openCreate">
        <PlusIcon class="size-4" />
        写日志
      </Button>
    </template>

    <Card>
      <CardHeader class="gap-4">
        <Tabs :model-value="visibility" class="ml-auto" @update:model-value="handleFilterChange">
          <TabsList class="w-full lg:w-auto">
            <TabsTrigger class="flex-1 lg:flex-none" value="all">全部</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="public">公开</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="private">私密</TabsTrigger>
          </TabsList>
        </Tabs>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="error" class="border-b px-4 py-3 text-sm text-destructive">{{ error }}</p>
        <div v-if="operationError" class="border-b px-4 py-2.5 text-xs font-medium text-destructive">{{ operationError }}</div>

        <div v-if="loading" class="space-y-2 p-4">
          <Skeleton v-for="index in 5" :key="index" class="h-16 w-full" />
        </div>

        <AppEmptyState
          v-else-if="journals.length === 0"
          :icon="NotebookPenIcon"
          :title="visibility === 'all' ? '还没有日志' : '没有匹配的日志'"
          :description="visibility === 'all' ? '点击「写日志」记录第一条想法。' : '切换可见性筛选试试。'"
        />

        <!-- 时间线式列表：左侧节点线 + 右侧内容卡片 -->
        <ol v-else class="relative space-y-0 px-4 py-2">
          <li
            v-for="journal in journals"
            :key="journal.id"
            class="relative border-l pb-6 pl-6 last:pb-2"
          >
            <span class="absolute -left-[5px] top-2 size-2.5 rounded-full border-2 border-background bg-primary" />
            <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <time class="tabular-nums">{{ formatTime(journal.created_at) }}</time>
              <Badge v-if="journal.visibility === 'private'" variant="secondary">
                <LockIcon class="mr-1 size-3" />
                私密
              </Badge>
              <Badge v-else variant="outline">
                <GlobeIcon class="mr-1 size-3" />
                公开
              </Badge>
              <span class="ml-auto flex items-center gap-1">
                <Button variant="ghost" size="icon-sm" :aria-label="`编辑日志 #${journal.id}`" @click="openEdit(journal)">
                  <PencilIcon />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  class="text-destructive hover:text-destructive"
                  :aria-label="`删除日志 #${journal.id}`"
                  @click="confirmDelete(journal)"
                >
                  <Trash2Icon />
                </Button>
              </span>
            </div>
            <div
              class="prose prose-sm mt-2 max-w-none rounded-md border bg-muted/30 p-3 text-sm dark:prose-invert"
              v-html="journal.content_html || journal.content_markdown"
            />
          </li>
        </ol>
      </CardContent>
      <CardFooter class="h-auto">
        <AppDataTablePagination
          v-model:page="page"
          v-model:page-size="pageSize"
          :total="total"
          :loading="loading"
          unit="条日志"
        />
      </CardFooter>
    </Card>

    <!-- 创建/编辑 Dialog -->
    <Dialog :open="dialogOpen" @update:open="handleDialogClose">
      <DialogContent class="flex h-[min(640px,calc(100dvh-2rem))] w-[calc(100%-1rem)] max-w-2xl! flex-col gap-0 overflow-hidden p-0">
        <DialogHeader class="border-b px-4 py-4 text-left sm:px-5">
          <DialogTitle class="text-base">{{ editing ? '编辑日志' : '写日志' }}</DialogTitle>
          <DialogDescription class="mt-0.5">支持 Markdown，私密日志不会出现在公开站。</DialogDescription>
        </DialogHeader>

        <div class="min-h-0 flex-1 space-y-4 overflow-y-auto p-4 sm:p-5">
          <!-- 可见性切换：原生 Checkbox 开关，与文章编辑器同款样式。 -->
          <label class="flex cursor-pointer items-center justify-between gap-3 text-sm font-medium">
            <span class="flex items-center gap-1.5">
              <LockIcon class="size-4 text-muted-foreground" />
              私密日志（仅管理端可见）
            </span>
            <input
              type="checkbox"
              class="peer sr-only"
              :checked="formVisibility === 'private'"
              :disabled="saving"
              @change="formVisibility = ($event.target as HTMLInputElement).checked ? 'private' : 'public'"
            />
            <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
          </label>
          <MarkdownEditor
            v-if="dialogOpen"
            v-model="formContent"
            :height="360"
            :upload="uploadEditorImages"
            placeholder="记录此刻的想法..."
          />
          <p v-if="formError" role="alert" class="text-sm text-destructive">{{ formError }}</p>
        </div>

        <DialogFooter class="border-t px-4 py-3 sm:px-5">
          <Button variant="outline" :disabled="saving" @click="dialogOpen = false">取消</Button>
          <Button :disabled="saving" @click="handleSave">
            <LoaderCircleIcon v-if="saving" class="animate-spin" />
            {{ saving ? '保存中…' : editing ? '保存' : '发布' }}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <AppConfirmDialog
      v-model:open="deleteConfirmOpen"
      title="删除日志"
      :description="`确定要删除 ${formatTime(pendingDelete?.created_at ?? '')} 的这条日志吗？该操作不可恢复。`"
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
