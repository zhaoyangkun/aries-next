<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import dayjs from 'dayjs'
import {
  BanIcon,
  CheckIcon,
  EllipsisIcon,
  EyeIcon,
  MessageSquareIcon,
  ReplyIcon,
  SearchIcon,
  SparklesIcon,
  Trash2Icon,
  Undo2Icon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardFooter, CardHeader } from '@/components/ui/card'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Textarea } from '@/components/ui/textarea'
import {
  AppDataTablePagination,
} from '@/shared/components/data-table'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import { BasicPage } from '@/components/global-layout'
import {
  commentsApi,
  type Comment,
  type CommentAiRisk,
  type CommentStatus,
} from '@/modules/comments/api/comments'
import { getApiError } from '@/shared/api/client'
import { useDebouncedWatch } from '@/composables/use-debounced-watch'

type StatusFilter = 'all' | CommentStatus

// 支持查询参数直达：Dashboard「去处理」按钮 → /comments?status=pending
const route = useRoute()
function statusFromQuery(): StatusFilter {
  const value = route.query.status
  if (value === 'pending' || value === 'approved' || value === 'rejected' || value === 'spam' || value === 'recycled')
    return value
  return 'all'
}

const status = ref<StatusFilter>(statusFromQuery())
const keyword = ref('')
const page = ref(1)
const pageSize = ref(20)
const comments = ref<Comment[]>([])
const total = ref(0)
const loading = ref(true)
const error = ref('')
const operationError = ref('')

// 详情 Dialog
const detailOpen = ref(false)
const detailComment = ref<Comment | null>(null)
// 回复 Dialog
const replyOpen = ref(false)
const replyComment = ref<Comment | null>(null)
const replyContent = ref('')
const replyError = ref('')
const replying = ref(false)
// 删除确认
const deleteConfirmOpen = ref(false)
const pendingDelete = ref<Comment | null>(null)
const deleting = ref(false)

const statusMeta: Record<CommentStatus, { label: string, variant: 'default' | 'secondary' | 'destructive' | 'outline' }> = {
  pending: { label: '待审核', variant: 'secondary' },
  approved: { label: '已通过', variant: 'default' },
  rejected: { label: '已拒绝', variant: 'outline' },
  spam: { label: '垃圾评论', variant: 'destructive' },
  recycled: { label: '回收站', variant: 'outline' },
}

// AI 审核风险等级展示：safe 绿 / suspicious 黄 / spam 红。
const aiRiskMeta: Record<CommentAiRisk, { label: string, badgeClass: string }> = {
  safe: { label: '安全', badgeClass: 'border-transparent bg-green-500/15 text-green-700 dark:text-green-400' },
  suspicious: { label: '可疑', badgeClass: 'border-transparent bg-amber-500/15 text-amber-700 dark:text-amber-400' },
  spam: { label: '疑似垃圾', badgeClass: 'border-transparent bg-destructive/15 text-destructive' },
}

async function loadComments() {
  loading.value = true
  error.value = ''
  try {
    const result = await commentsApi.list({
      page: page.value,
      page_size: pageSize.value,
      status: status.value === 'all' ? undefined : status.value,
      keyword: keyword.value.trim() || undefined,
    })
    comments.value = result.items
    total.value = result.total
  }
  catch (e) {
    error.value = getApiError(e, '加载评论失败')
  }
  finally {
    loading.value = false
  }
}

// Tabs 切换筛选：page 已为 1 时 watch 不会触发，需手动加载。
function handleFilterChange(next: string | number) {
  status.value = next as StatusFilter
  const needPageReset = page.value !== 1
  page.value = 1
  if (!needPageReset)
    loadComments()
}

useDebouncedWatch(keyword, () => {
  // page 非 1 时由下方 watch 触发加载，避免重复请求。
  if (page.value !== 1)
    page.value = 1
  else
    loadComments()
}, 300)

// 翻页与每页数量变化时重新请求；pageSize 变化且不在第一页时先回到第一页（由 page 变化触发加载）。
watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadComments()
})

onMounted(loadComments)

function formatTime(value: string) {
  const parsed = dayjs(value)
  return parsed.isValid() ? parsed.format('YYYY-MM-DD HH:mm') : '-'
}

function openDetail(comment: Comment) {
  detailComment.value = comment
  detailOpen.value = true
}

function openReply(comment: Comment) {
  replyComment.value = comment
  replyContent.value = ''
  replyError.value = ''
  replyOpen.value = true
}

function handleReplyOpenChange(open: boolean) {
  if (!replying.value)
    replyOpen.value = open
}

async function handleReply() {
  if (!replyComment.value)
    return
  const content = replyContent.value.trim()
  if (content.length < 1 || content.length > 2000) {
    replyError.value = '回复内容需在 1–2000 字之间'
    return
  }
  replying.value = true
  replyError.value = ''
  try {
    await commentsApi.reply(replyComment.value.id, content)
    replyOpen.value = false
    detailOpen.value = false
    await loadComments()
  }
  catch (e) {
    replyError.value = getApiError(e, '回复失败')
  }
  finally {
    replying.value = false
  }
}

async function handleStatusChange(comment: Comment, next: CommentStatus, reason?: string) {
  operationError.value = ''
  try {
    await commentsApi.changeStatus(comment.id, next, reason)
    await loadComments()
  }
  catch (e) {
    operationError.value = getApiError(e, '审核操作失败')
  }
}

function confirmDelete(comment: Comment) {
  pendingDelete.value = comment
  deleteConfirmOpen.value = true
}

async function handleDelete() {
  if (!pendingDelete.value)
    return
  deleting.value = true
  try {
    await commentsApi.remove(pendingDelete.value.id)
    deleteConfirmOpen.value = false
    pendingDelete.value = null
    await loadComments()
  }
  catch (e) {
    operationError.value = getApiError(e, '删除失败')
  }
  finally {
    deleting.value = false
  }
}

// 状态机（与 core 一致）决定每行可执行的审核操作。
function actionsFor(comment: Comment) {
  if (comment.status === 'pending')
    return ['approve', 'reject', 'spam'] as const
  if (comment.status === 'recycled')
    return ['recover', 'delete'] as const
  return ['recycle'] as const
}

const hasFilter = computed(
  () => status.value !== 'all' || Boolean(keyword.value.trim()),
)
</script>

<template>
  <BasicPage title="评论管理" description="审核访客评论、回复并管理回收站。">
    <Card>
      <CardHeader class="gap-4">
        <div class="relative max-w-xs flex-1">
          <SearchIcon class="absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            v-model="keyword"
            placeholder="搜索评论内容或评论者"
            class="h-8 pl-8 text-sm"
            @keyup.enter="page = 1; loadComments()"
          />
        </div>
        <Tabs :model-value="status" class="ml-auto" @update:model-value="handleFilterChange">
          <TabsList class="w-full lg:w-auto">
            <TabsTrigger class="flex-1 lg:flex-none" value="all">全部</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="pending">待审核</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="approved">已通过</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="rejected">已拒绝</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="spam">垃圾</TabsTrigger>
            <TabsTrigger class="flex-1 lg:flex-none" value="recycled">回收站</TabsTrigger>
          </TabsList>
        </Tabs>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="error" class="border-b px-4 py-3 text-sm text-destructive">{{ error }}</p>
        <div v-if="operationError" class="border-b px-4 py-2.5 text-xs font-medium text-destructive">{{ operationError }}</div>

        <div v-if="loading" class="space-y-2 p-4">
          <Skeleton v-for="index in 5" :key="index" class="h-12 w-full" />
        </div>

        <AppEmptyState
          v-else-if="comments.length === 0"
          :icon="MessageSquareIcon"
          :title="hasFilter ? '没有匹配的评论' : '还没有评论'"
          :description="hasFilter ? '尝试修改关键词或切换状态筛选。' : '访客在文章下提交评论后会出现在这里。'"
        />

        <ul v-else class="divide-y">
          <li
            v-for="comment in comments"
            :key="comment.id"
            class="flex items-start gap-3 px-4 py-3"
          >
            <div class="min-w-0 flex-1">
              <div class="flex flex-wrap items-center gap-2">
                <span class="text-sm font-medium">{{ comment.author_name }}</span>
                <Badge :variant="statusMeta[comment.status as CommentStatus].variant">
                  {{ statusMeta[comment.status as CommentStatus].label }}
                </Badge>
                <Badge v-if="comment.is_admin_reply" variant="outline">管理员回复</Badge>
                <span class="text-xs tabular-nums text-muted-foreground">{{ formatTime(comment.created_at) }}</span>
              </div>
              <p class="mt-1 line-clamp-2 text-sm text-muted-foreground">
                {{ comment.content_markdown }}
              </p>
              <div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <span class="font-mono">{{ comment.author_email }}</span>
                <span>文章 #{{ comment.target_id }}</span>
                <span v-if="comment.moderation_reason">审核备注：{{ comment.moderation_reason }}</span>
              </div>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              <Button
                variant="ghost"
                size="icon-sm"
                :aria-label="`查看评论详情：${comment.author_name}`"
                @click="openDetail(comment)"
              >
                <EyeIcon />
              </Button>
              <DropdownMenu>
                <DropdownMenuTrigger as-child>
                  <Button variant="ghost" size="icon-sm" :aria-label="`操作评论：${comment.author_name}`">
                    <EllipsisIcon />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" class="w-36">
                  <DropdownMenuItem @click="openReply(comment)">
                    <ReplyIcon />
                    回复
                  </DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <template v-for="action in actionsFor(comment)" :key="action">
                    <template v-if="action === 'approve'">
                      <DropdownMenuItem @click="handleStatusChange(comment, 'approved')">
                        <CheckIcon />
                        通过
                      </DropdownMenuItem>
                    </template>
                    <template v-else-if="action === 'reject'">
                      <DropdownMenuItem @click="handleStatusChange(comment, 'rejected')">
                        <BanIcon />
                        拒绝
                      </DropdownMenuItem>
                    </template>
                    <template v-else-if="action === 'spam'">
                      <DropdownMenuItem @click="handleStatusChange(comment, 'spam')">
                        <BanIcon />
                        标记垃圾
                      </DropdownMenuItem>
                    </template>
                    <template v-else-if="action === 'recycle'">
                      <DropdownMenuItem
                        class="text-destructive focus:text-destructive"
                        @click="handleStatusChange(comment, 'recycled')"
                      >
                        <Trash2Icon />
                        移入回收站
                      </DropdownMenuItem>
                    </template>
                    <template v-else-if="action === 'recover'">
                      <DropdownMenuItem @click="handleStatusChange(comment, 'approved')">
                        <Undo2Icon />
                        恢复并公开
                      </DropdownMenuItem>
                    </template>
                    <DropdownMenuItem
                      v-else
                      class="text-destructive focus:text-destructive"
                      @click="confirmDelete(comment)"
                    >
                      <Trash2Icon />
                      永久删除
                    </DropdownMenuItem>
                  </template>
                </DropdownMenuContent>
              </DropdownMenu>
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
          unit="条评论"
        />
      </CardFooter>
    </Card>

    <!-- 评论详情：content_html 已由服务端 Sanitize，可安全渲染。 -->
    <Dialog :open="detailOpen" @update:open="detailOpen = $event">
      <DialogContent class="max-w-lg">
        <DialogHeader>
          <DialogTitle>评论详情</DialogTitle>
          <DialogDescription>
            {{ detailComment?.author_name }} · {{ formatTime(detailComment?.created_at ?? '') }}
          </DialogDescription>
        </DialogHeader>
        <div v-if="detailComment" class="grid gap-4 py-2 text-sm">
          <div class="flex flex-wrap items-center gap-2">
            <Badge :variant="statusMeta[detailComment.status as CommentStatus].variant">
              {{ statusMeta[detailComment.status as CommentStatus].label }}
            </Badge>
            <span class="font-mono text-xs text-muted-foreground">{{ detailComment.author_email }}</span>
            <span class="text-xs text-muted-foreground">文章 #{{ detailComment.target_id }}</span>
          </div>
          <div
            class="prose prose-sm max-w-none rounded-md border bg-muted/30 p-4 text-sm dark:prose-invert"
            v-html="detailComment.content_html || detailComment.content_markdown"
          />
          <!-- AI 审核结论（开启「评论自动审核」后由 Backend 写入）；未评估的评论不显示该区块。 -->
          <div v-if="detailComment.ai_risk" class="space-y-1.5 rounded-md border bg-muted/30 p-3 text-xs">
            <div class="flex flex-wrap items-center gap-2">
              <span class="flex items-center gap-1 font-medium">
                <SparklesIcon class="size-3.5 text-primary" />
                AI 审核
              </span>
              <Badge :class="aiRiskMeta[detailComment.ai_risk].badgeClass">
                {{ aiRiskMeta[detailComment.ai_risk].label }}
              </Badge>
              <span v-if="detailComment.ai_confidence != null" class="tabular-nums text-muted-foreground">
                置信度 {{ Math.round(detailComment.ai_confidence * 100) }}%
              </span>
            </div>
            <p v-if="detailComment.ai_reason" class="leading-5 text-muted-foreground">
              {{ detailComment.ai_reason }}
            </p>
          </div>
          <p v-if="detailComment.moderation_reason" class="text-xs text-muted-foreground">
            审核备注：{{ detailComment.moderation_reason }}
          </p>
          <p v-if="detailComment.is_admin_reply" class="text-xs text-muted-foreground">
            该评论为管理员回复。
          </p>
        </div>
        <DialogFooter>
          <Button variant="outline" :disabled="replying" @click="detailComment && openReply(detailComment)">
            <ReplyIcon />
            回复
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <!-- 回复 Dialog -->
    <Dialog :open="replyOpen" @update:open="handleReplyOpenChange">
      <DialogContent class="max-w-lg">
        <DialogHeader>
          <DialogTitle>回复评论</DialogTitle>
          <DialogDescription>
            回复将立即公开，署名使用当前账号名。支持 Markdown。
          </DialogDescription>
        </DialogHeader>
        <div class="grid gap-4 py-4">
          <div class="grid gap-2">
            <Label for="reply-content">回复内容</Label>
            <Textarea
              id="reply-content"
              v-model="replyContent"
              rows="5"
              maxlength="2000"
              placeholder="输入回复内容…"
            />
            <p class="text-xs text-muted-foreground">{{ replyContent.length }} / 2000</p>
          </div>
          <p v-if="replyError" class="text-sm text-destructive">{{ replyError }}</p>
        </div>
        <DialogFooter>
          <Button variant="outline" :disabled="replying" @click="replyOpen = false">取消</Button>
          <Button :disabled="replying" @click="handleReply">
            {{ replying ? '发送中…' : '发送回复' }}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <!-- 永久删除确认 -->
    <AppConfirmDialog
      v-model:open="deleteConfirmOpen"
      title="永久删除评论"
      :description="`确定要永久删除「${pendingDelete?.author_name}」的评论吗？该操作不可恢复。`"
      confirm-label="永久删除"
      destructive
      :busy="deleting"
      @confirm="handleDelete"
    />
  </BasicPage>
</template>
