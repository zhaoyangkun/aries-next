<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import type { ColumnDef, RowSelectionState } from '@tanstack/vue-table'
import {
  ArrowDownIcon,
  ArrowUpIcon,
  ChevronsDownIcon,
  ChevronsUpIcon,
  EllipsisIcon,
  FileTextIcon,
  FileUpIcon,
  GripVerticalIcon,
  PencilIcon,
  PinIcon,
  PlusIcon,
  SearchIcon,
  SendIcon,
  Trash2Icon,
  Undo2Icon,
  XIcon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardFooter, CardHeader } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  articlesApi,
  type AdminArticle,
  type ArticleCategory,
  type ArticleSort,
  type ArticleSortOrder,
  type ArticleStatus,
  type ArticleStatusCommand,
  type ArticleTag,
} from '@/modules/articles/api/articles'
import { useSessionStore } from '@/modules/auth/stores/session'
import { getApiError } from '@/shared/api/client'
import { toast } from 'vue-sonner'
import { useDebouncedWatch } from '@/composables/use-debounced-watch'
import {
  AppDataTable,
  AppDataTableColumnToggle,
  AppDataTablePagination,
} from '@/shared/components/data-table'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import { BasicPage } from '@/components/global-layout'
import ArticleEditorDialog from './components/ArticleEditorDialog.vue'
import ArticleImportDialog from './components/ArticleImportDialog.vue'

type ArticleStatusFilter = 'all' | ArticleStatus

const keyword = ref('')
const session = useSessionStore()
const status = ref<ArticleStatusFilter>('all')
const page = ref(1)
const pageSize = ref(20)
const sort = ref<ArticleSort>('updated_at')
const order = ref<ArticleSortOrder>('desc')
const categoryId = ref<number | null>(null)
const tagId = ref<number | null>(null)
const categories = ref<ArticleCategory[]>([])
const tags = ref<ArticleTag[]>([])
const editorOpen = ref(false)
const selectedArticle = ref<AdminArticle | null>(null)
const importOpen = ref(false)
const articles = ref<AdminArticle[]>([])
const total = ref(0)
const loading = ref(false)
const error = ref('')
const operationError = ref('')
// 非阻断性提示（如跨置顶分组移动被定序规则拒绝），走全局 toast。
function showHint(message: string) {
  toast.warning(message)
}
const confirmOpen = ref(false)
const pendingArticle = ref<AdminArticle | null>(null)
const pendingCommand = ref<ArticleStatusCommand | null>(null)
const changingStatus = ref(false)
const deleteConfirmOpen = ref(false)
const pendingDelete = ref<AdminArticle | null>(null)
const deleting = ref(false)

// 「排序」模式（sort_order 升序）下展示拖拽手柄与上下箭头；切到该模式固定升序。
const reorderMode = computed(() => sort.value === 'sort_order')
const reordering = ref(false)
// 是否存在下一页：排序模式下据此判断页尾下移是否可用。
const hasNextPage = computed(() => page.value * pageSize.value < total.value)
const dragIndex = ref<number | null>(null)
const dropTargetIndex = ref<number | null>(null)
watch(sort, (field) => {
  if (field === 'sort_order') order.value = 'asc'
})

// 拖拽视觉反馈：被拖行半透明，悬停目标行高亮（含顶部插入线语义）。
const reorderRowClass = (row: AdminArticle) => {
  const index = articles.value.indexOf(row)
  if (index === dragIndex.value) return 'opacity-40'
  if (index === dropTargetIndex.value) return 'bg-primary/10 shadow-[inset_0_2px_0_0_var(--primary)]'
  return ''
}

const hasActiveFilter = computed(
  () => Boolean(keyword.value.trim()) || status.value !== 'all' || categoryId.value !== null || tagId.value !== null,
)

// 表格列定义；排序与分页走后端，AppDataTable 只负责呈现与交互状态。
// 「排序」模式下在最前面临时插入拖拽/箭头列，退出后隐藏。
const reorderColumn: ColumnDef<AdminArticle, unknown> = {
  id: 'reorder',
  header: '排序',
  enableHiding: false,
  meta: { headerClass: 'w-32', cellClass: 'w-32' },
}

const articleColumns = computed<ColumnDef<AdminArticle, unknown>[]>(() =>
  reorderMode.value
    ? [reorderColumn, ...staticArticleColumns]
    : staticArticleColumns,
)

const staticArticleColumns: ColumnDef<AdminArticle, unknown>[] = [
  {
    id: 'title',
    accessorKey: 'title',
    header: '标题',
    enableHiding: false,
    meta: { sortable: true, toggleLabel: '标题' },
  },
  {
    id: 'status',
    accessorKey: 'status',
    header: '状态',
    enableHiding: false,
    meta: { headerClass: 'w-28', toggleLabel: '状态' },
  },
  {
    id: 'author',
    accessorKey: 'author_id',
    header: '作者',
    meta: {
      headerClass: 'hidden w-28 md:table-cell',
      cellClass: 'hidden text-muted-foreground md:table-cell',
      toggleLabel: '作者',
    },
  },
  {
    id: 'updated_at',
    accessorKey: 'updated_at',
    header: '更新时间',
    meta: {
      sortable: true,
      headerClass: 'w-32 text-right sm:w-40',
      cellClass: 'text-right text-xs text-muted-foreground sm:text-sm',
      toggleLabel: '更新时间',
    },
  },
  { id: 'actions', header: '操作', enableHiding: false, meta: { headerClass: 'w-12' } },
]

// 行选择以文章 ID 为键，翻页不丢；批量操作复用单条接口逐个执行。
const tableRef = ref()
const selection = ref<RowSelectionState>({})
const selectedArticles = computed(() =>
  articles.value.filter((article) => selection.value[String(article.id)]),
)
const batchBusy = ref(false)

function handleSort(field: string) {
  if (sort.value === field) {
    order.value = order.value === 'desc' ? 'asc' : 'desc'
    return
  }
  sort.value = field as ArticleSort
  order.value = 'desc'
}

async function batchChangeStatus(command: ArticleStatusCommand) {
  if (batchBusy.value) return
  const eligible = selectedArticles.value.filter((article) =>
    command === 'publish'
      ? article.status === 'draft'
      : command === 'recycle'
        ? article.status !== 'recycled'
        : article.status === 'recycled',
  )
  if (!eligible.length) return
  batchBusy.value = true
  operationError.value = ''
  // 失败的项计数汇总，不中断整批。
  let failed = 0
  for (const article of eligible) {
    try {
      await articlesApi.changeStatus(article.id, command, article.version)
    } catch {
      failed += 1
    }
  }
  selection.value = {}
  batchBusy.value = false
  if (failed) operationError.value = `${failed} 篇文章操作失败，请逐条检查后重试`
  await loadArticles()
}
const confirmation = computed(() => {
  const title = pendingArticle.value?.title ?? '当前文章'
  if (pendingCommand.value === 'publish') {
    return {
      title: `发布《${title}》？`,
      description: '发布后文章将进入公开内容列表。',
      label: '发布',
      destructive: false,
    }
  }
  if (pendingCommand.value === 'recover') {
    return {
      title: `恢复《${title}》？`,
      description: '文章将恢复为草稿，可继续编辑后再次发布。',
      label: '恢复为草稿',
      destructive: false,
    }
  }
  return {
    title: `将《${title}》移入回收站？`,
    description: '文章不会被永久删除，可以从回收站恢复。',
    label: '移入回收站',
    destructive: true,
  }
})

onMounted(() => {
  loadArticles()
  loadTaxonomy()
  // 空闲时预取 Vditor chunk（体积较大），首次打开编辑器时已下载解析完成；
  // Safari 无 requestIdleCallback，用 setTimeout 兜底。
  const idle = window.requestIdleCallback ?? ((cb: () => void) => setTimeout(cb, 2000))
  idle(() => {
    void import('vditor')
  })
})

// 过滤/排序变化时回到第一页；翻页与每页数量变化时直接重新请求。
useDebouncedWatch([keyword, status, categoryId, tagId, sort, order], () => {
  if (page.value !== 1) page.value = 1
  else loadArticles()
}, 250)

watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadArticles()
})

async function loadArticles() {
  loading.value = true
  error.value = ''
  try {
    const pageData = await articlesApi.list({
      page: page.value,
      page_size: pageSize.value,
      keyword: keyword.value.trim() || undefined,
      status: status.value === 'all' ? undefined : status.value,
      category_id: categoryId.value ?? undefined,
      tag_id: tagId.value ?? undefined,
      sort: sort.value,
      order: order.value,
    })
    articles.value = pageData.items
    total.value = pageData.total
  } catch (requestError) {
    error.value = getApiError(requestError, '文章列表加载失败')
  } finally {
    loading.value = false
  }
}

// 按期望顺序提交重排：先乐观更新本地列表，成功后刷新对齐服务端状态，失败回滚。
async function saveReorder(ordered: AdminArticle[]) {
  if (reordering.value) return
  reordering.value = true
  operationError.value = ''
  const previous = articles.value
  articles.value = ordered
  try {
    await articlesApi.reorder(ordered.map((article) => article.id))
    await loadArticles()
  }
  catch (requestError) {
    articles.value = previous
    operationError.value = getApiError(requestError, '排序保存失败')
  }
  finally {
    reordering.value = false
    dragIndex.value = null
  }
}

// 上下箭头：页内与相邻条目交换；页首上移（非第一页）与页尾下移（存在下一页）走跨页移动接口。
function moveBy(index: number, delta: -1 | 1) {
  const target = index + delta
  if (target >= 0 && target < articles.value.length) {
    // 定序规则为置顶恒前：跨置顶分组的交换刷新后会被弹回，直接拒绝并提示。
    if (articles.value[index].is_pinned !== articles.value[target].is_pinned) {
      showHint('置顶文章始终排在最前，不能跨越置顶分组调整顺序')
      return
    }
    const next = [...articles.value]
    ;[next[index], next[target]] = [next[target], next[index]]
    void saveReorder(next)
    return
  }
  const article = articles.value[index]
  if (!article || reordering.value) return
  void moveAcrossPage(article, delta)
}

// 跨页移动：无法乐观交换，直接调 move 接口，成功后重新加载当前页对齐服务端状态；
// moved=false 表示文章已在置顶分组边界（后端 no-op），给出提示。
async function moveAcrossPage(article: AdminArticle, delta: -1 | 1) {
  reordering.value = true
  operationError.value = ''
  try {
    const moved = await articlesApi.move(article.id, delta === -1 ? 'up' : 'down')
    if (!moved) showHint('置顶文章始终排在最前，该文章已到分组边界，无法继续移动')
    await loadArticles()
  }
  catch (requestError) {
    operationError.value = getApiError(requestError, '文章移动失败')
  }
  finally {
    reordering.value = false
  }
}

// 移到所在置顶分组的最前/最后：后端单条 UPDATE 完成，moved=false 表示已在分组边缘。
async function moveToGroupEdge(article: AdminArticle, edge: 'top' | 'bottom') {
  if (reordering.value) return
  reordering.value = true
  operationError.value = ''
  try {
    const moved = await articlesApi.move(article.id, edge)
    if (!moved) showHint(edge === 'top' ? '该文章已在分组最前' : '该文章已在分组最后')
    await loadArticles()
  }
  catch (requestError) {
    operationError.value = getApiError(requestError, '文章移动失败')
  }
  finally {
    reordering.value = false
  }
}

// 整行拖拽（排序模式下由 AppDataTable 透传事件）：整行任意位置都可落下。
function handleRowDragStart(row: AdminArticle, event: DragEvent) {
  dragIndex.value = articles.value.indexOf(row)
  event.dataTransfer?.setData('text/plain', String(dragIndex.value))
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

function handleRowDragOver(row: AdminArticle, event: DragEvent) {
  if (dragIndex.value === null || reordering.value) return
  const target = articles.value.indexOf(row)
  if (target === -1 || target === dragIndex.value) return
  event.preventDefault()
  dropTargetIndex.value = target
  if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
}

function handleRowDrop(row: AdminArticle, event: DragEvent) {
  event.preventDefault()
  const from = dragIndex.value
  const target = articles.value.indexOf(row)
  dragIndex.value = null
  dropTargetIndex.value = null
  if (from === null || target === -1 || from === target) return
  // 定序规则为置顶恒前：把文章拖到另一置顶分组刷新后会被弹回，直接拒绝并提示。
  if (articles.value[from].is_pinned !== articles.value[target].is_pinned) {
    showHint('置顶文章始终排在最前，不能跨越置顶分组调整顺序')
    return
  }
  const next = [...articles.value]
  const [moved] = next.splice(from, 1)
  next.splice(target, 0, moved)
  void saveReorder(next)
}

function handleRowDragEnd() {
  dragIndex.value = null
  dropTargetIndex.value = null
}

async function loadTaxonomy() {  try {
    const [loadedCategories, loadedTags] = await Promise.all([
      articlesApi.listCategories(),
      articlesApi.listTags(),
    ])
    categories.value = loadedCategories
    tags.value = loadedTags
  } catch {
    // 过滤下拉加载失败不阻塞列表，仅不展示可选项。
    categories.value = []
    tags.value = []
  }
}

async function handleSaved() {
  editorOpen.value = false
  selectedArticle.value = null
  await loadArticles()
}

function openNewEditor() {
  selectedArticle.value = null
  editorOpen.value = true
}

// 导入成功后刷新列表，导入的文章均为草稿。
function handleImported() {
  void loadArticles()
}

function openEditEditor(article: AdminArticle) {
  selectedArticle.value = article
  editorOpen.value = true
}

function requestStatusChange(article: AdminArticle, command: ArticleStatusCommand) {
  operationError.value = ''
  pendingArticle.value = article
  pendingCommand.value = command
  confirmOpen.value = true
}

async function confirmStatusChange() {
  if (!pendingArticle.value || !pendingCommand.value || changingStatus.value) return
  changingStatus.value = true
  try {
    await articlesApi.changeStatus(
      pendingArticle.value.id,
      pendingCommand.value,
      pendingArticle.value.version,
    )
    confirmOpen.value = false
    pendingArticle.value = null
    pendingCommand.value = null
    await loadArticles()
  } catch (requestError) {
    operationError.value = getApiError(requestError, '文章状态更新失败')
  } finally {
    changingStatus.value = false
  }
}

function requestDelete(article: AdminArticle) {
  operationError.value = ''
  pendingDelete.value = article
  deleteConfirmOpen.value = true
}

async function confirmDelete() {
  if (!pendingDelete.value || deleting.value) return
  deleting.value = true
  try {
    await articlesApi.remove(pendingDelete.value.id)
    deleteConfirmOpen.value = false
    pendingDelete.value = null
    // 删除当前页最后一条时回退一页，避免停留在空页。
    if (articles.value.length === 1 && page.value > 1) page.value -= 1
    else await loadArticles()
  } catch (requestError) {
    operationError.value = getApiError(requestError, '文章删除失败')
  } finally {
    deleting.value = false
  }
}

function formatUpdatedAt(value: string) {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return '-'
  return new Intl.DateTimeFormat('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date)
}

function statusText(value: ArticleStatus) {
  if (value === 'published') return '已发布'
  if (value === 'recycled') return '回收站'
  return '草稿'
}

function authorText(article: AdminArticle) {
  if (article.author_id === session.user?.id) return session.user.display_name
  return `User #${article.author_id}`
}
</script>

<template>
  <BasicPage title="文章" description="创建、发布和维护站点文章。" sticky>
    <template #actions>
      <Button variant="outline" @click="importOpen = true">
        <FileUpIcon />
        导入
      </Button>
      <Button @click="openNewEditor">
        <PlusIcon />
        写文章
      </Button>
    </template>

    <section aria-label="文章列表">
      <Card class="gap-0 overflow-hidden py-0 shadow-none">
        <CardHeader class="gap-3 border-b p-3 sm:p-4">
          <div class="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div class="relative w-full lg:max-w-sm">
              <SearchIcon
                class="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input v-model="keyword" class="pl-8 pr-8" placeholder="搜索标题或 Slug" aria-label="搜索文章" />
              <Button v-if="keyword" variant="ghost" size="icon-sm"
                class="absolute right-0.5 top-1/2 -translate-y-1/2 text-muted-foreground" aria-label="清除搜索"
                @click="keyword = ''">
                <XIcon />
              </Button>
            </div>
            <Tabs v-model="status">
              <TabsList class="w-full lg:w-auto">
                <TabsTrigger class="flex-1 lg:flex-none" value="all">全部</TabsTrigger>
                <TabsTrigger class="flex-1 lg:flex-none" value="published">已发布</TabsTrigger>
                <TabsTrigger class="flex-1 lg:flex-none" value="draft">草稿</TabsTrigger>
                <TabsTrigger class="flex-1 lg:flex-none" value="recycled">回收站</TabsTrigger>
              </TabsList>
            </Tabs>
          </div>
          <div class="flex flex-wrap items-center gap-2">
            <select v-model="categoryId" aria-label="按分类筛选"
              class="flex h-8 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30">
              <option :value="null">全部分类</option>
              <option v-for="category in categories" :key="category.id" :value="category.id">{{ category.name }}
              </option>
            </select>
            <select v-model="tagId" aria-label="按标签筛选"
              class="flex h-8 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30">
              <option :value="null">全部标签</option>
              <option v-for="tag in tags" :key="tag.id" :value="tag.id">{{ tag.name }}</option>
            </select>
            <select v-model="sort" aria-label="排序字段"
              class="flex h-8 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30">
              <option value="updated_at">按更新时间</option>
              <option value="published_at">按发布时间</option>
              <option value="created_at">按创建时间</option>
              <option value="title">按标题</option>
              <option value="sort_order">按排序（可拖拽）</option>
            </select>
            <select v-model="order" aria-label="排序方向"
              class="flex h-8 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30">
              <option value="desc">降序</option>
              <option value="asc">升序</option>
            </select>
            <AppDataTableColumnToggle :table="tableRef?.table" class="ml-auto" />
          </div>
        </CardHeader>
        <CardContent class="p-0">
          <AppDataTable ref="tableRef" v-model:selection="selection" :data="articles" :columns="articleColumns"
            :loading="loading" :error="error" :sort-field="sort" :sort-order="order" selectable clickable
            :row-key="(article: AdminArticle) => article.id" :empty-icon="FileTextIcon"
            :row-draggable="reorderMode" :row-class="reorderMode ? reorderRowClass : undefined"
            :empty-title="hasActiveFilter ? '没有匹配的文章' : '还没有文章'"
            :empty-description="hasActiveFilter ? '尝试修改关键词、筛选条件或切换文章状态。' : '创建第一篇草稿，开始整理站点内容。'"
            @update:sort="handleSort" @row-click="openEditEditor" @retry="loadArticles"
            @row-dragstart="handleRowDragStart" @row-dragover="handleRowDragOver"
            @row-drop="handleRowDrop" @row-dragend="handleRowDragEnd">
            <template #cell-reorder="{ row }">
              <div class="flex h-full items-center gap-0.5">
                <button
                  type="button"
                  class="cursor-grab touch-none text-muted-foreground transition-colors hover:text-foreground active:cursor-grabbing"
                  :disabled="reordering"
                  aria-label="拖拽排序"
                  @click.stop
                >
                  <GripVerticalIcon class="size-4" />
                </button>
                <button
                  type="button"
                  class="text-muted-foreground transition-colors hover:text-foreground disabled:opacity-30"
                  :disabled="(articles.indexOf(row) === 0 && page === 1) || reordering"
                  :aria-label="`将《${row.title}》移到最前`"
                  @click.stop="moveToGroupEdge(row, 'top')"
                >
                  <ChevronsUpIcon class="size-3.5" />
                </button>
                <button
                  type="button"
                  class="text-muted-foreground transition-colors hover:text-foreground disabled:opacity-30"
                  :disabled="(articles.indexOf(row) === 0 && page === 1) || reordering"
                  :aria-label="`上移《${row.title}》`"
                  @click.stop="moveBy(articles.indexOf(row), -1)"
                >
                  <ArrowUpIcon class="size-3.5" />
                </button>
                <button
                  type="button"
                  class="text-muted-foreground transition-colors hover:text-foreground disabled:opacity-30"
                  :disabled="(articles.indexOf(row) === articles.length - 1 && !hasNextPage) || reordering"
                  :aria-label="`下移《${row.title}》`"
                  @click.stop="moveBy(articles.indexOf(row), 1)"
                >
                  <ArrowDownIcon class="size-3.5" />
                </button>
                <button
                  type="button"
                  class="text-muted-foreground transition-colors hover:text-foreground disabled:opacity-30"
                  :disabled="(articles.indexOf(row) === articles.length - 1 && !hasNextPage) || reordering"
                  :aria-label="`将《${row.title}》移到最后`"
                  @click.stop="moveToGroupEdge(row, 'bottom')"
                >
                  <ChevronsDownIcon class="size-3.5" />
                </button>
              </div>
            </template>
            <template #cell-title="{ row }">
              <div class="min-w-0 py-1">
                <div class="flex items-center gap-1.5">
                  <p class="truncate font-medium">{{ row.title }}</p>
                  <Badge v-if="row.is_pinned" variant="secondary" class="shrink-0 gap-0.5 px-1.5">
                    <PinIcon class="size-3" />
                    置顶
                  </Badge>
                </div>
                <p class="mt-0.5 truncate text-xs text-muted-foreground">/{{ row.slug }}</p>
              </div>
            </template>
            <template #cell-status="{ row }">
              <Badge :variant="row.status === 'published' ? 'default' : 'secondary'">
                {{ statusText(row.status as ArticleStatus) }}
              </Badge>
            </template>
            <template #cell-author="{ row }">{{ authorText(row) }}</template>
            <template #cell-updated_at="{ row }">{{ formatUpdatedAt(row.updated_at) }}</template>
            <template #cell-actions="{ row }">
              <div @click.stop>
                <DropdownMenu>
                  <DropdownMenuTrigger as-child>
                    <Button variant="ghost" size="icon-sm" :aria-label="`操作文章：${row.title}`">
                      <EllipsisIcon />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" class="w-40">
                    <DropdownMenuItem @click="openEditEditor(row)">
                      <PencilIcon />
                      编辑
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem v-if="row.status === 'draft'" @click="requestStatusChange(row, 'publish')">
                      <SendIcon />
                      发布
                    </DropdownMenuItem>
                    <template v-if="row.status !== 'recycled'">
                      <DropdownMenuItem class="text-destructive focus:text-destructive"
                        @click="requestStatusChange(row, 'recycle')">
                        <Trash2Icon />
                        移入回收站
                      </DropdownMenuItem>
                    </template>
                    <template v-else>
                      <DropdownMenuItem @click="requestStatusChange(row, 'recover')">
                        <Undo2Icon />
                        恢复为草稿
                      </DropdownMenuItem>
                      <DropdownMenuItem class="text-destructive focus:text-destructive" @click="requestDelete(row)">
                        <Trash2Icon />
                        永久删除
                      </DropdownMenuItem>
                    </template>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </template>
            <template v-if="!hasActiveFilter" #empty-actions>
              <Button variant="outline" @click="openNewEditor">
                <PlusIcon />
                创建草稿
              </Button>
            </template>
          </AppDataTable>
        </CardContent>
        <CardFooter class="flex-wrap items-center justify-end gap-x-4 gap-y-2 border-t px-4 py-3">
          <div v-if="selectedArticles.length" class="mr-auto flex flex-wrap items-center gap-2">
            <p class="text-xs font-medium">已选 {{ selectedArticles.length }} 篇</p>
            <Button variant="outline" size="sm" :disabled="batchBusy" @click="batchChangeStatus('publish')">
              批量发布
            </Button>
            <Button variant="outline" size="sm" :disabled="batchBusy" @click="batchChangeStatus('recycle')">
              移入回收站
            </Button>
            <Button variant="ghost" size="sm" :disabled="batchBusy" @click="selection = {}">
              清除选择
            </Button>
          </div>
          <p v-else-if="operationError" class="mr-auto text-xs font-medium text-destructive">{{ operationError }}</p>
          <AppDataTablePagination v-model:page="page" v-model:page-size="pageSize" :total="total" :loading="loading"
            unit="篇文章" />
        </CardFooter>
      </Card>
    </section>

    <ArticleEditorDialog v-model:open="editorOpen" :article="selectedArticle" @saved="handleSaved" />
    <ArticleImportDialog v-model:open="importOpen" @imported="handleImported" />
    <AppConfirmDialog v-model:open="confirmOpen" :title="confirmation.title" :description="confirmation.description"
      :confirm-label="confirmation.label" :destructive="confirmation.destructive" :busy="changingStatus"
      @confirm="confirmStatusChange" />
    <AppConfirmDialog v-model:open="deleteConfirmOpen" :title="`永久删除《${pendingDelete?.title ?? '当前文章'}》？`"
      description="文章及其全部历史版本将被彻底删除，无法恢复。如只是想下架，请使用移入回收站。" confirm-label="永久删除" destructive :busy="deleting"
      @confirm="confirmDelete" />
  </BasicPage>
</template>

<route lang="yaml">
meta:
  permission: content:manage
</route>
