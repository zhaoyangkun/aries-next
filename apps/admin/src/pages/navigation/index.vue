<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import {
  ArrowDownIcon,
  ArrowUpIcon,
  ExternalLinkIcon,
  EyeOffIcon,
  LoaderCircleIcon,
  NavigationIcon,
  PencilIcon,
  PlusIcon,
  SaveIcon,
  Trash2Icon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import { BasicPage } from '@/components/global-layout'
import {
  navigationApi,
  type NavigationItem,
  type NavigationTargetType,
} from '@/modules/navigation/api/navigation'
import { articlesApi, type AdminArticle, type ArticleCategory } from '@/modules/articles/api/articles'
import { pagesApi, type CustomPage } from '@/modules/pages/api/pages'
import { getApiError } from '@/shared/api/client'

// 两级树的本地视图：children 顺序即展示顺序。
interface NavNode {
  item: NavigationItem
  children: NavigationItem[]
}

const nodes = ref<NavNode[]>([])
const loading = ref(true)
const error = ref('')
const operationError = ref('')
// 本地排序偏离服务器状态时为 true，提示用户「保存排序」一次性提交。
const orderDirty = ref(false)
const savingOrder = ref(false)
let savedOrderSignature = ''

// 编辑 Dialog
const dialogOpen = ref(false)
const editing = ref<NavigationItem | null>(null)
const form = reactive({
  parent_id: null as number | null,
  label: '',
  target_type: 'url' as NavigationTargetType,
  target_id: null as number | null,
  url: '',
  open_in_new_tab: false,
  visible: true,
})
const formError = ref('')
const saving = ref(false)

// target_id 联动选项：按 target_type 分别加载候选列表。
const targetArticles = ref<AdminArticle[]>([])
const targetPages = ref<CustomPage[]>([])
const targetCategories = ref<ArticleCategory[]>([])
const targetsLoading = ref(false)

// 删除确认
const deleteConfirmOpen = ref(false)
const pendingDelete = ref<NavigationItem | null>(null)
const deleting = ref(false)

const targetTypeMeta: Record<NavigationTargetType, string> = {
  article: '文章',
  page: '页面',
  category: '分类',
  url: '自定义链接',
}

function orderSignature(tree: NavNode[]) {
  return tree.flatMap(node => [node.item.id, ...node.children.map(child => child.id)]).join(',')
}

function buildTree(items: NavigationItem[]): NavNode[] {
  const parents = items
    .filter(item => item.parent_id === null)
    .sort((a, b) => a.sort_order - b.sort_order)
  return parents.map(parent => ({
    item: parent,
    children: items
      .filter(item => item.parent_id === parent.id)
      .sort((a, b) => a.sort_order - b.sort_order),
  }))
}

async function loadNavigation() {
  loading.value = true
  error.value = ''
  try {
    const items = await navigationApi.list()
    nodes.value = buildTree(items)
    savedOrderSignature = orderSignature(nodes.value)
    orderDirty.value = false
  }
  catch (e) {
    error.value = getApiError(e, '加载导航菜单失败')
  }
  finally {
    loading.value = false
  }
}

onMounted(loadNavigation)

// 父节点候选：所有一级节点（编辑自身时排除，避免自引用）。
const parentOptions = computed(() =>
  nodes.value
    .map(node => node.item)
    .filter(item => item.id !== editing.value?.id),
)

async function ensureTargetsLoaded() {
  if (targetsLoading.value || targetArticles.value.length > 0) return
  targetsLoading.value = true
  try {
    const [articles, pageResult, categories] = await Promise.all([
      articlesApi.list({ page: 1, page_size: 100 }),
      pagesApi.list({ page: 1, page_size: 100 }),
      articlesApi.listCategories(),
    ])
    targetArticles.value = articles.items
    targetPages.value = pageResult.items
    targetCategories.value = categories
  }
  catch (e) {
    formError.value = getApiError(e, '目标列表加载失败')
  }
  finally {
    targetsLoading.value = false
  }
}

function openCreate(parentId: number | null) {
  editing.value = null
  form.parent_id = parentId
  form.label = ''
  form.target_type = 'url'
  form.target_id = null
  form.url = ''
  form.open_in_new_tab = false
  form.visible = true
  formError.value = ''
  dialogOpen.value = true
  void ensureTargetsLoaded()
}

function openEdit(item: NavigationItem) {
  editing.value = item
  form.parent_id = item.parent_id
  form.label = item.label
  form.target_type = item.target_type
  form.target_id = item.target_id
  form.url = item.url ?? ''
  form.open_in_new_tab = item.open_in_new_tab
  form.visible = item.visible
  formError.value = ''
  dialogOpen.value = true
  void ensureTargetsLoaded()
}

function handleDialogClose(open: boolean) {
  if (!saving.value) dialogOpen.value = open
}

async function handleSave() {
  const label = form.label.trim()
  if (!label) {
    formError.value = '请输入菜单名称'
    return
  }
  if (form.target_type === 'url' && !form.url.trim()) {
    formError.value = '请输入链接 URL'
    return
  }
  if (form.target_type !== 'url' && form.target_id === null) {
    formError.value = `请选择目标${targetTypeMeta[form.target_type]}`
    return
  }
  if (saving.value) return
  saving.value = true
  formError.value = ''
  try {
    const payload = {
      parent_id: form.parent_id,
      label,
      target_type: form.target_type,
      // url 类型使用 url 字段，其余类型使用 target_id，互斥提交。
      target_id: form.target_type === 'url' ? null : form.target_id,
      url: form.target_type === 'url' ? form.url.trim() : null,
      open_in_new_tab: form.open_in_new_tab,
      visible: form.visible,
      sort_order: editing.value?.sort_order ?? 0,
    }
    if (editing.value)
      await navigationApi.update(editing.value.id, payload)
    else
      await navigationApi.create(payload)
    dialogOpen.value = false
    await loadNavigation()
  }
  catch (e) {
    formError.value = getApiError(e, '保存失败')
  }
  finally {
    saving.value = false
  }
}

// 上移/下移只改本地顺序并置脏；点击「保存排序」才调用原子排序接口。
function moveNode(nodeIndex: number, direction: -1 | 1) {
  const target = nodeIndex + direction
  if (target < 0 || target >= nodes.value.length) return
  const reordered = [...nodes.value]
  const [moved] = reordered.splice(nodeIndex, 1)
  reordered.splice(target, 0, moved)
  nodes.value = reordered
  orderDirty.value = orderSignature(nodes.value) !== savedOrderSignature
}

function moveChild(nodeIndex: number, childIndex: number, direction: -1 | 1) {
  const node = nodes.value[nodeIndex]
  const target = childIndex + direction
  if (target < 0 || target >= node.children.length) return
  const children = [...node.children]
  const [moved] = children.splice(childIndex, 1)
  children.splice(target, 0, moved)
  nodes.value = nodes.value.map((entry, index) =>
    index === nodeIndex ? { ...entry, children } : entry,
  )
  orderDirty.value = orderSignature(nodes.value) !== savedOrderSignature
}

async function saveOrder() {
  if (!orderDirty.value || savingOrder.value) return
  savingOrder.value = true
  operationError.value = ''
  try {
    const ids = nodes.value.flatMap(node => [node.item.id, ...node.children.map(child => child.id)])
    await navigationApi.reorder(ids)
    savedOrderSignature = orderSignature(nodes.value)
    orderDirty.value = false
  }
  catch (e) {
    operationError.value = getApiError(e, '排序保存失败')
  }
  finally {
    savingOrder.value = false
  }
}

function confirmDelete(item: NavigationItem) {
  pendingDelete.value = item
  deleteConfirmOpen.value = true
}

async function handleDelete() {
  if (!pendingDelete.value) return
  deleting.value = true
  try {
    await navigationApi.remove(pendingDelete.value.id)
    deleteConfirmOpen.value = false
    pendingDelete.value = null
    await loadNavigation()
  }
  catch (e) {
    operationError.value = getApiError(e, '删除失败')
  }
  finally {
    deleting.value = false
  }
}

function targetText(item: NavigationItem) {
  if (item.target_type === 'url') return item.url ?? ''
  return `${targetTypeMeta[item.target_type]} #${item.target_id}`
}
</script>

<template>
  <BasicPage title="导航菜单" description="维护公开站的导航结构，最多支持两级。">
    <template #actions>
      <Button variant="outline" :disabled="!orderDirty || savingOrder" @click="saveOrder">
        <LoaderCircleIcon v-if="savingOrder" class="animate-spin" />
        <SaveIcon v-else />
        {{ savingOrder ? '保存中…' : '保存排序' }}
      </Button>
      <Button @click="openCreate(null)">
        <PlusIcon class="size-4" />
        新增菜单
      </Button>
    </template>

    <Card>
      <CardHeader v-if="orderDirty" class="border-b px-4 py-3">
        <p class="text-xs font-medium text-amber-600 dark:text-amber-500">
          顺序已修改但尚未保存，点击右上角「保存排序」生效。
        </p>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="error" class="border-b px-4 py-3 text-sm text-destructive">{{ error }}</p>
        <div v-if="operationError" class="border-b px-4 py-2.5 text-xs font-medium text-destructive">{{ operationError }}</div>

        <div v-if="loading" class="space-y-2 p-4">
          <Skeleton v-for="index in 4" :key="index" class="h-12 w-full" />
        </div>

        <AppEmptyState
          v-else-if="nodes.length === 0"
          :icon="NavigationIcon"
          title="还没有导航菜单"
          description="点击「新增菜单」创建第一个导航项。"
        />

        <ul v-else class="divide-y">
          <li v-for="(node, nodeIndex) in nodes" :key="node.item.id" class="px-4 py-3">
            <div class="flex items-center gap-3">
              <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="text-sm font-medium">{{ node.item.label }}</span>
                  <Badge variant="outline">{{ targetTypeMeta[node.item.target_type] }}</Badge>
                  <Badge v-if="!node.item.visible" variant="secondary">
                    <EyeOffIcon class="mr-1 size-3" />
                    隐藏
                  </Badge>
                  <Badge v-if="node.item.open_in_new_tab" variant="secondary">
                    <ExternalLinkIcon class="mr-1 size-3" />
                    新窗口
                  </Badge>
                </div>
                <p class="mt-0.5 truncate text-xs text-muted-foreground">{{ targetText(node.item) }}</p>
              </div>
              <div class="flex shrink-0 items-center gap-1">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  :disabled="nodeIndex === 0"
                  :aria-label="`上移菜单 ${node.item.label}`"
                  @click="moveNode(nodeIndex, -1)"
                >
                  <ArrowUpIcon />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  :disabled="nodeIndex === nodes.length - 1"
                  :aria-label="`下移菜单 ${node.item.label}`"
                  @click="moveNode(nodeIndex, 1)"
                >
                  <ArrowDownIcon />
                </Button>
                <Button variant="ghost" size="icon-sm" :aria-label="`为 ${node.item.label} 添加子菜单`" @click="openCreate(node.item.id)">
                  <PlusIcon />
                </Button>
                <Button variant="ghost" size="icon-sm" :aria-label="`编辑菜单 ${node.item.label}`" @click="openEdit(node.item)">
                  <PencilIcon />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  class="text-destructive hover:text-destructive"
                  :aria-label="`删除菜单 ${node.item.label}`"
                  @click="confirmDelete(node.item)"
                >
                  <Trash2Icon />
                </Button>
              </div>
            </div>

            <ul v-if="node.children.length" class="mt-2 space-y-1.5 border-l pl-4">
              <li
                v-for="(child, childIndex) in node.children"
                :key="child.id"
                class="flex items-center gap-3 rounded-md border bg-muted/30 px-3 py-2"
              >
                <div class="min-w-0 flex-1">
                  <div class="flex flex-wrap items-center gap-2">
                    <span class="text-sm">{{ child.label }}</span>
                    <Badge variant="outline">{{ targetTypeMeta[child.target_type] }}</Badge>
                    <Badge v-if="!child.visible" variant="secondary">
                      <EyeOffIcon class="mr-1 size-3" />
                      隐藏
                    </Badge>
                    <Badge v-if="child.open_in_new_tab" variant="secondary">
                      <ExternalLinkIcon class="mr-1 size-3" />
                      新窗口
                    </Badge>
                  </div>
                  <p class="mt-0.5 truncate text-xs text-muted-foreground">{{ targetText(child) }}</p>
                </div>
                <div class="flex shrink-0 items-center gap-1">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    :disabled="childIndex === 0"
                    :aria-label="`上移子菜单 ${child.label}`"
                    @click="moveChild(nodeIndex, childIndex, -1)"
                  >
                    <ArrowUpIcon />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    :disabled="childIndex === node.children.length - 1"
                    :aria-label="`下移子菜单 ${child.label}`"
                    @click="moveChild(nodeIndex, childIndex, 1)"
                  >
                    <ArrowDownIcon />
                  </Button>
                  <Button variant="ghost" size="icon-sm" :aria-label="`编辑子菜单 ${child.label}`" @click="openEdit(child)">
                    <PencilIcon />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    class="text-destructive hover:text-destructive"
                    :aria-label="`删除子菜单 ${child.label}`"
                    @click="confirmDelete(child)"
                  >
                    <Trash2Icon />
                  </Button>
                </div>
              </li>
            </ul>
          </li>
        </ul>
      </CardContent>
    </Card>

    <!-- 创建/编辑 Dialog -->
    <Dialog :open="dialogOpen" @update:open="handleDialogClose">
      <DialogContent class="max-w-lg">
        <DialogHeader>
          <DialogTitle>{{ editing ? '编辑菜单' : '新增菜单' }}</DialogTitle>
          <DialogDescription>目标类型决定链接指向：文章/页面/分类选择具体目标，自定义链接直接填写 URL。</DialogDescription>
        </DialogHeader>

        <div class="grid gap-4 py-2">
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <Label for="nav-label">名称</Label>
              <Input id="nav-label" v-model="form.label" placeholder="菜单名称" maxlength="60" :disabled="saving" />
            </div>
            <div class="grid gap-2">
              <Label for="nav-parent">父级菜单</Label>
              <select
                id="nav-parent"
                v-model="form.parent_id"
                class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                :disabled="saving"
              >
                <option :value="null">（作为一级菜单）</option>
                <option v-for="parent in parentOptions" :key="parent.id" :value="parent.id">
                  {{ parent.label }}
                </option>
              </select>
            </div>
          </div>
          <div class="grid gap-2">
            <Label for="nav-target-type">目标类型</Label>
            <select
              id="nav-target-type"
              v-model="form.target_type"
              class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
              :disabled="saving"
              @change="form.target_id = null; form.url = ''"
            >
              <option value="url">自定义链接</option>
              <option value="article">文章</option>
              <option value="page">页面</option>
              <option value="category">分类</option>
            </select>
          </div>
          <div v-if="form.target_type === 'url'" class="grid gap-2">
            <Label for="nav-url">URL</Label>
            <Input id="nav-url" v-model="form.url" placeholder="/about 或 https://example.com" :disabled="saving" />
          </div>
          <div v-else class="grid gap-2">
            <Label for="nav-target">目标{{ targetTypeMeta[form.target_type] }}</Label>
            <select
              id="nav-target"
              v-model="form.target_id"
              class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
              :disabled="saving || targetsLoading"
            >
              <option :value="null">{{ targetsLoading ? '加载中…' : '请选择' }}</option>
              <template v-if="form.target_type === 'article'">
                <option v-for="article in targetArticles" :key="article.id" :value="article.id">
                  {{ article.title }}
                </option>
              </template>
              <template v-else-if="form.target_type === 'page'">
                <option v-for="pageItem in targetPages" :key="pageItem.id" :value="pageItem.id">
                  {{ pageItem.title }}（/custom/{{ pageItem.slug }}）
                </option>
              </template>
              <template v-else>
                <option v-for="category in targetCategories" :key="category.id" :value="category.id">
                  {{ category.name }}
                </option>
              </template>
            </select>
          </div>
          <!-- 开关复用文章编辑器的原生 Checkbox 样式，行为与原生控件一致。 -->
          <div class="space-y-2">
            <label class="flex cursor-pointer items-center justify-between gap-3 text-sm font-medium">
              在导航中显示
              <input v-model="form.visible" type="checkbox" class="peer sr-only" :disabled="saving" />
              <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
            </label>
            <label class="flex cursor-pointer items-center justify-between gap-3 text-sm font-medium">
              在新窗口打开
              <input v-model="form.open_in_new_tab" type="checkbox" class="peer sr-only" :disabled="saving" />
              <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
            </label>
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
      title="删除菜单"
      :description="`确定要删除菜单「${pendingDelete?.label}」吗？包含子菜单时需先删除全部子菜单。`"
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
