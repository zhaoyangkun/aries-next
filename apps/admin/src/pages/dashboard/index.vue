<script setup lang="ts">
import axios from 'axios'
import dayjs from 'dayjs'
import {
  AlertTriangleIcon,
  FileTextIcon,
  ImagesIcon,
  MessageSquareIcon,
  TagsIcon,
  TrendingUpIcon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { BasicPage } from '@/components/global-layout'
import { articlesApi, type AdminArticle } from '@/modules/articles/api/articles'
import { mediaApi } from '@/modules/media/api/media'
import { dashboardApi } from '@/modules/dashboard/api/dashboard'
import { getApiError } from '@/shared/api/client'

type ServiceStatus = 'checking' | 'online' | 'offline'

const serviceStatus = ref<ServiceStatus>('checking')
const serviceLabel = computed(() => {
  if (serviceStatus.value === 'online')
    return 'Backend 正常'
  if (serviceStatus.value === 'offline')
    return 'Backend 离线'
  return '正在检查 Backend'
})

// 统计指标全部来自聚合端点 /api/admin/dashboard（一次请求，避免 N 个列表查询）。
const dashboard = ref<Awaited<ReturnType<typeof dashboardApi.get>> | null>(null)
const articleTotal = ref<number | null>(null)
const mediaTotal = ref<number | null>(null)
const tagTotal = ref<number | null>(null)
const recentArticles = ref<AdminArticle[]>([])
const loadingRecent = ref(true)
const dashboardError = ref('')

const metrics = computed(() => [
  { label: '文章', value: articleTotal.value, description: dashboard.value ? `已发布 ${dashboard.value.articles.published} · 草稿 ${dashboard.value.articles.draft}` : '全部状态文章总数', icon: FileTextIcon },
  { label: '待审核评论', value: dashboard.value?.comments.pending ?? null, description: dashboard.value ? `今日新增 ${dashboard.value.comments.today} · 累计 ${dashboard.value.comments.total}` : '等待人工审核的评论', icon: MessageSquareIcon },
  { label: '媒体文件', value: mediaTotal.value, description: '媒体库资产总数', icon: ImagesIcon },
  { label: '标签', value: tagTotal.value, description: '内容标签总数', icon: TagsIcon },
])

function formatTime(value: string) {
  const parsed = dayjs(value)
  return parsed.isValid() ? parsed.format('MM-DD HH:mm') : '-'
}

function statusText(article: AdminArticle) {
  if (article.status === 'published')
    return '已发布'
  if (article.status === 'recycled')
    return '回收站'
  return '草稿'
}

onMounted(async () => {
  try {
    await axios.get('/api/health/ready')
    serviceStatus.value = 'online'
  }
  catch {
    serviceStatus.value = 'offline'
  }

  try {
    const [articlesPage, mediaPage, tagList, dash] = await Promise.all([
      articlesApi.list({ page: 1, page_size: 5, sort: 'updated_at', order: 'desc' }),
      mediaApi.list({ page: 1, page_size: 1 }),
      articlesApi.listTags(),
      dashboardApi.get(),
    ])
    articleTotal.value = articlesPage.total
    mediaTotal.value = mediaPage.total
    tagTotal.value = tagList.length
    recentArticles.value = articlesPage.items
    dashboard.value = dash
  }
  catch (e) {
    // 指标加载失败不阻塞页面，保持占位并提示。
    dashboardError.value = getApiError(e, 'Dashboard 数据加载失败')
  }
  finally {
    loadingRecent.value = false
  }
})
</script>

<template>
  <BasicPage title="概览" description="查看内容状态、待处理评论和最近更新。" sticky>
    <template #actions>
      <Badge variant="secondary" class="h-7 gap-2 px-2.5 font-normal">
        <span
          class="size-1.5 rounded-full"
          :class="[
            serviceStatus === 'online'
              ? 'bg-green-500'
              : serviceStatus === 'offline'
                ? 'bg-destructive'
                : 'animate-pulse bg-muted-foreground',
          ]"
        />
        {{ serviceLabel }}
      </Badge>
    </template>

    <section class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4" aria-label="内容统计">
      <Card v-for="metric in metrics" :key="metric.label">
        <CardHeader class="flex flex-row items-start justify-between gap-3">
          <div class="space-y-1.5">
            <CardDescription>{{ metric.label }}</CardDescription>
            <CardTitle class="text-2xl tabular-nums">
              <Skeleton v-if="metric.value === null" class="h-8 w-12" />
              <template v-else>{{ metric.value }}</template>
            </CardTitle>
          </div>
          <span class="flex size-9 items-center justify-center rounded-md bg-primary/10 text-primary">
            <component :is="metric.icon" class="size-4" />
          </span>
        </CardHeader>
        <CardContent>
          <p class="text-xs text-muted-foreground">{{ metric.description }}</p>
        </CardContent>
      </Card>
    </section>

    <p v-if="dashboardError" class="mt-4 rounded-md border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
      {{ dashboardError }}
    </p>

    <div class="mt-4 grid gap-4 lg:grid-cols-2">
      <!-- 待审核评论：直接进入评论管理页处理 -->
      <Card class="gap-0 overflow-hidden py-0" aria-labelledby="pending-comments-heading">
        <CardHeader class="border-b py-5">
          <CardTitle id="pending-comments-heading" class="flex items-center gap-2 text-base">
            <MessageSquareIcon class="size-4 text-muted-foreground" />
            待审核评论
          </CardTitle>
          <CardDescription>最新等待处理的访客评论。</CardDescription>
        </CardHeader>
        <CardContent class="p-0">
          <div v-if="loadingRecent" class="space-y-2 p-4">
            <Skeleton v-for="index in 3" :key="index" class="h-10 w-full" />
          </div>
          <div v-else-if="(dashboard?.recent_pending_comments ?? []).length === 0" class="p-8 text-center text-sm text-muted-foreground">
            当前没有待审核的评论。
          </div>
          <ul v-else class="divide-y">
            <li
              v-for="comment in dashboard?.recent_pending_comments"
              :key="comment.id"
              class="flex items-center justify-between gap-3 px-4 py-3"
            >
              <div class="min-w-0">
                <p class="truncate text-sm font-medium">{{ comment.author_name }}</p>
                <p class="mt-0.5 truncate text-xs text-muted-foreground">{{ comment.content_markdown }}</p>
              </div>
              <div class="flex shrink-0 items-center gap-2">
                <span class="text-xs tabular-nums text-muted-foreground">{{ formatTime(comment.created_at) }}</span>
                <Button variant="ghost" size="sm" class="h-7 text-xs" as-child>
                  <RouterLink to="/comments?status=pending">去处理</RouterLink>
                </Button>
              </div>
            </li>
          </ul>
        </CardContent>
      </Card>

      <!-- 最近更新文章 -->
      <Card class="gap-0 overflow-hidden py-0" aria-labelledby="recent-heading">
        <CardHeader class="border-b py-5">
          <CardTitle id="recent-heading" class="flex items-center gap-2 text-base">
            <TrendingUpIcon class="size-4 text-muted-foreground" />
            最近更新
          </CardTitle>
          <CardDescription>最近编辑或发布的文章。</CardDescription>
        </CardHeader>
        <CardContent class="p-0">
          <div v-if="loadingRecent" class="space-y-2 p-4">
            <Skeleton v-for="index in 3" :key="index" class="h-10 w-full" />
          </div>
          <div v-else-if="recentArticles.length === 0" class="p-8 text-center text-sm text-muted-foreground">
            还没有文章，去
            <RouterLink to="/articles" class="text-primary hover:underline">写第一篇</RouterLink>
            吧。
          </div>
          <ul v-else class="divide-y">
            <li v-for="article in recentArticles" :key="article.id">
              <RouterLink
                to="/articles"
                class="flex items-center justify-between gap-3 px-4 py-3 transition-colors hover:bg-accent/50"
              >
                <div class="min-w-0">
                  <p class="truncate text-sm font-medium">{{ article.title }}</p>
                  <p class="mt-0.5 truncate text-xs text-muted-foreground">/{{ article.slug }}</p>
                </div>
                <div class="flex shrink-0 items-center gap-3">
                  <Badge :variant="article.status === 'published' ? 'default' : 'secondary'">
                    {{ statusText(article) }}
                  </Badge>
                  <span class="text-xs tabular-nums text-muted-foreground">{{ formatTime(article.updated_at) }}</span>
                </div>
              </RouterLink>
            </li>
          </ul>
        </CardContent>
      </Card>
    </div>

    <!-- 失败 Job：需要人工关注时才展示 -->
    <Card
      v-if="dashboard && dashboard.recent_failed_jobs.length > 0"
      class="mt-4 gap-0 overflow-hidden border-destructive/40 py-0"
      aria-labelledby="failed-jobs-heading"
    >
      <CardHeader class="border-b py-5">
        <CardTitle id="failed-jobs-heading" class="flex items-center gap-2 text-base text-destructive">
          <AlertTriangleIcon class="size-4" />
          失败的后台任务
        </CardTitle>
        <CardDescription>以下任务重试次数已达上限，需要检查。</CardDescription>
      </CardHeader>
      <CardContent class="p-0">
        <ul class="divide-y">
          <li v-for="job in dashboard.recent_failed_jobs" :key="job.id" class="flex items-start justify-between gap-3 px-4 py-3">
            <div class="min-w-0">
              <p class="text-sm font-medium font-mono">{{ job.kind }}</p>
              <p class="mt-0.5 truncate text-xs text-muted-foreground">{{ job.last_error ?? '未知错误' }}</p>
            </div>
            <div class="flex shrink-0 items-center gap-3 text-xs tabular-nums text-muted-foreground">
              <span>{{ job.attempts }}/{{ job.max_attempts }} 次</span>
              <span>{{ formatTime(job.updated_at) }}</span>
            </div>
          </li>
        </ul>
      </CardContent>
    </Card>
  </BasicPage>
</template>
