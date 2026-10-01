<script setup lang="ts">
import { onMounted, ref } from 'vue'
import {
  EllipsisIcon,
  PencilIcon,
  PlusIcon,
  TagIcon,
  Trash2Icon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Table, TableBody, TableCell, TableHead, TableHeader, TableRow,
} from '@/components/ui/table'
import { articlesApi, type ArticleTag } from '@/modules/articles/api/articles'
import { getApiError } from '@/shared/api/client'
import { BasicPage } from '@/components/global-layout'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'

const tags = ref<ArticleTag[]>([])
const loading = ref(true)
const error = ref('')

// 创建/编辑 Dialog 状态
const dialogOpen = ref(false)
const editing = ref<ArticleTag | null>(null)
const formName = ref('')
const formSlug = ref('')
const saving = ref(false)
const formError = ref('')

// 删除确认状态
const deleteConfirmOpen = ref(false)
const pendingDelete = ref<ArticleTag | null>(null)
const deleting = ref(false)

async function loadTags() {
  loading.value = true
  error.value = ''
  try {
    tags.value = await articlesApi.listTags()
  }
  catch (e) {
    error.value = getApiError(e, '加载标签失败')
  }
  finally {
    loading.value = false
  }
}

function openCreate() {
  editing.value = null
  formName.value = ''
  formSlug.value = ''
  formError.value = ''
  dialogOpen.value = true
}

function openEdit(tag: ArticleTag) {
  editing.value = tag
  formName.value = tag.name
  formSlug.value = tag.slug
  formError.value = ''
  dialogOpen.value = true
}

function handleDialogClose(open: boolean) {
  if (!saving.value) dialogOpen.value = open
}

async function handleSave() {
  const name = formName.value.trim()
  if (!name) {
    formError.value = '标签名称不能为空'
    return
  }
  saving.value = true
  formError.value = ''
  try {
    if (editing.value) {
      await articlesApi.updateTag(editing.value.id, {
        name,
        slug: formSlug.value.trim() || undefined,
      })
    }
    else {
      await articlesApi.createTag(name)
    }
    dialogOpen.value = false
    await loadTags()
  }
  catch (e) {
    formError.value = getApiError(e, '保存失败')
  }
  finally {
    saving.value = false
  }
}

function confirmDelete(tag: ArticleTag) {
  pendingDelete.value = tag
  deleteConfirmOpen.value = true
}

async function handleDelete() {
  if (!pendingDelete.value) return
  deleting.value = true
  try {
    await articlesApi.deleteTag(pendingDelete.value.id)
    deleteConfirmOpen.value = false
    pendingDelete.value = null
    await loadTags()
  }
  catch (e) {
    error.value = getApiError(e, '删除失败')
  }
  finally {
    deleting.value = false
  }
}

onMounted(loadTags)
</script>

<template>
  <BasicPage title="标签管理" description="管理文章标签，支持创建、编辑和删除。">
    <template #actions>
      <Button @click="openCreate">
        <PlusIcon class="size-4" />
        新增标签
      </Button>
    </template>

    <Card>
      <CardHeader class="px-6 py-4">
        <div v-if="error" class="rounded-md bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {{ error }}
        </div>
      </CardHeader>
      <CardContent class="px-6 pb-6">
        <div v-if="loading" class="flex items-center justify-center py-12 text-sm text-muted-foreground">
          加载中…
        </div>

        <AppEmptyState
          v-else-if="tags.length === 0"
          :icon="TagIcon"
          title="暂无标签"
          description="点击「新增标签」创建第一个标签。"
        />

        <Table v-else>
          <TableHeader>
            <TableRow>
              <TableHead>名称</TableHead>
              <TableHead>Slug</TableHead>
              <TableHead class="w-16 text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="tag in tags" :key="tag.id">
              <TableCell class="font-medium">
                <div class="flex items-center gap-2">
                  <TagIcon class="size-4 text-muted-foreground" />
                  {{ tag.name }}
                </div>
              </TableCell>
              <TableCell>
                <Badge variant="outline">{{ tag.slug }}</Badge>
              </TableCell>
              <TableCell class="text-right">
                <DropdownMenu>
                  <DropdownMenuTrigger as-child>
                    <Button variant="ghost" size="icon" class="size-8">
                      <EllipsisIcon class="size-4" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem @click="openEdit(tag)">
                      <PencilIcon class="size-4" />
                      编辑
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      class="text-destructive focus:text-destructive"
                      @click="confirmDelete(tag)"
                    >
                      <Trash2Icon class="size-4" />
                      删除
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </CardContent>
    </Card>

    <!-- 创建/编辑 Dialog -->
    <Dialog :open="dialogOpen" @update:open="handleDialogClose">
      <DialogContent class="max-w-md">
        <DialogHeader>
          <DialogTitle>{{ editing ? '编辑标签' : '新增标签' }}</DialogTitle>
          <DialogDescription>
            {{ editing ? '修改标签名称和 Slug。' : '创建一个新的文章标签。' }}
          </DialogDescription>
        </DialogHeader>

        <div class="grid gap-4 py-4">
          <div class="grid gap-2">
            <Label for="tag-name">名称</Label>
            <Input id="tag-name" v-model="formName" placeholder="标签名称" maxlength="100" />
          </div>
          <div class="grid gap-2">
            <Label for="tag-slug">
              Slug
              <span class="text-xs text-muted-foreground">（留空则自动生成）</span>
            </Label>
            <Input id="tag-slug" v-model="formSlug" placeholder="tag-slug" maxlength="160" />
          </div>
          <p v-if="formError" class="text-sm text-destructive">{{ formError }}</p>
        </div>

        <DialogFooter>
          <Button variant="outline" :disabled="saving" @click="dialogOpen = false">取消</Button>
          <Button :disabled="saving" @click="handleSave">
            {{ saving ? '保存中…' : '保存' }}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <!-- 删除确认 Dialog -->
    <AppConfirmDialog
      v-model:open="deleteConfirmOpen"
      title="删除标签"
      :description="`确定要删除标签「${pendingDelete?.name}」吗？已关联的文章不会被删除。`"
      confirm-label="删除"
      destructive
      :busy="deleting"
      @confirm="handleDelete"
    />
  </BasicPage>
</template>
