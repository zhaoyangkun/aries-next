<script setup lang="ts">
import { CheckCircle2Icon, LoaderCircleIcon, SparklesIcon } from '@lucide/vue'
import dayjs from 'dayjs'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table, TableBody, TableCell, TableHead, TableHeader, TableRow,
} from '@/components/ui/table'
import { AppDataTablePagination } from '@/shared/components/data-table'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import { aiApi, type AiFeature, type AiRequestStatus, type AiUsageItem } from '@/modules/ai/api/ai'
import { settingsGroupApi, type AiProtocol } from '@/modules/settings/api/settings'
import { getApiError } from '@/shared/api/client'

import SettingsLayout from './components/settings-layout.vue'

// api_key 为 write-only secret：GET 只回 api_key_set，
// 提交时留空表示保持不变，点击「清除」则显式提交空串删除已保存密钥。
const form = reactive({
  enabled: false,
  protocol: 'openai' as AiProtocol,
  base_url: '',
  model: '',
  editor_assist: false,
  comment_moderation: false,
  smart_search: false,
  embedding_base_url: '',
  embedding_model: '',
})
const apiKeySet = ref(false)
const newApiKey = ref('')
const clearApiKey = ref(false)
const version = ref(0)
const loading = ref(true)
const saving = ref(false)
const message = ref('')
const error = ref('')

// 用量记录
const usageItems = ref<AiUsageItem[]>([])
const usageTotal = ref(0)
const usagePage = ref(1)
const usagePageSize = ref(10)
const usageFeature = ref<'' | AiFeature>('')
const usageLoading = ref(true)
const usageError = ref('')

const featureMeta: Record<AiFeature, string> = {
  editor_rewrite: '编辑器 · 改写',
  editor_summary: '编辑器 · 摘要',
  editor_metadata: '编辑器 · SEO 建议',
  editor_tags: '编辑器 · 标签推荐',
  editor_brief: '编辑器 · AI 导读',
  comment_moderation: '评论自动审核',
}

const statusMeta: Record<AiRequestStatus, { label: string, variant: 'default' | 'secondary' | 'destructive' | 'outline' }> = {
  success: { label: '成功', variant: 'default' },
  failed: { label: '失败', variant: 'destructive' },
  cancelled: { label: '已取消', variant: 'outline' },
}

// 协议相关展示适配：anthropic 走官方端点时 base_url 可留空。
const baseUrlPlaceholder = computed(() =>
  form.protocol === 'anthropic' ? 'https://api.anthropic.com' : 'https://api.example.com/v1',
)
const baseUrlHint = computed(() =>
  form.protocol === 'anthropic' ? '可留空，默认 Anthropic 官方端点。' : 'OpenAI 兼容服务的 API 地址。',
)
const modelPlaceholder = computed(() =>
  form.protocol === 'anthropic' ? 'claude-sonnet-4-5' : 'gpt-4o-mini',
)

onMounted(async () => {
  try {
    const record = await settingsGroupApi.getAi()
    version.value = record.version
    apiKeySet.value = record.settings.api_key_set
    form.enabled = record.settings.enabled
    form.protocol = record.settings.protocol
    form.base_url = record.settings.base_url ?? ''
    form.model = record.settings.model ?? ''
    form.editor_assist = record.settings.features.editor_assist
    form.comment_moderation = record.settings.features.comment_moderation
    form.smart_search = record.settings.features.smart_search
    form.embedding_base_url = record.settings.embedding_base_url ?? ''
    form.embedding_model = record.settings.embedding_model ?? ''
  }
  catch (requestError) {
    error.value = getApiError(requestError, 'AI 设置加载失败')
  }
  finally {
    loading.value = false
  }
  void loadUsage()
})

async function save() {
  saving.value = true
  message.value = ''
  error.value = ''
  try {
    const record = await settingsGroupApi.updateAi(version.value, {
      enabled: form.enabled,
      protocol: form.protocol,
      base_url: form.base_url.trim() || null,
      model: form.model.trim() || null,
      // 三态语义：输入了新密钥优先更新，其次显式清除，否则缺省保持不变。
      ...(newApiKey.value
        ? { api_key: newApiKey.value }
        : clearApiKey.value
          ? { api_key: '' }
          : {}),
      features: {
        editor_assist: form.editor_assist,
        comment_moderation: form.comment_moderation,
        smart_search: form.smart_search,
      },
      // 省略/null 保持不变：留空提交 null，不清除服务端已保存的值。
      embedding_base_url: form.embedding_base_url.trim() || null,
      embedding_model: form.embedding_model.trim() || null,
    })
    version.value = record.version
    apiKeySet.value = record.settings.api_key_set
    newApiKey.value = ''
    clearApiKey.value = false
    message.value = 'AI 设置已保存'
  }
  catch (requestError) {
    error.value = getApiError(requestError, '保存失败')
  }
  finally {
    saving.value = false
  }
}

async function loadUsage() {
  usageLoading.value = true
  usageError.value = ''
  try {
    const result = await aiApi.listUsage({
      page: usagePage.value,
      page_size: usagePageSize.value,
      feature: usageFeature.value || undefined,
    })
    usageItems.value = result.items
    usageTotal.value = result.total
  }
  catch (requestError) {
    usageError.value = getApiError(requestError, '用量记录加载失败')
  }
  finally {
    usageLoading.value = false
  }
}

watch(usagePage, loadUsage)

function handleFeatureFilter(next: string) {
  usageFeature.value = next as '' | AiFeature
  const needPageReset = usagePage.value !== 1
  usagePage.value = 1
  if (!needPageReset) void loadUsage()
}

function formatTime(value: string) {
  const parsed = dayjs(value)
  return parsed.isValid() ? parsed.format('MM-DD HH:mm:ss') : '-'
}

function tokenText(item: AiUsageItem) {
  if (item.prompt_tokens === null && item.completion_tokens === null) return '-'
  return `${item.prompt_tokens ?? 0} + ${item.completion_tokens ?? 0}`
}
</script>

<template>
  <SettingsLayout title="AI 设置" description="配置 AI Provider 与功能开关；AI 只提供建议，不会自动发布或删除内容。">
    <div class="space-y-6">
      <Card>
        <form class="contents" novalidate @submit.prevent="save">
          <CardHeader>
            <CardTitle class="text-base">AI 服务</CardTitle>
            <CardDescription>API Key 为 write-only：服务端只返回是否已设置，永不回读明文。</CardDescription>
          </CardHeader>
          <CardContent class="grid max-w-2xl gap-5">
            <label class="flex cursor-pointer items-center justify-between gap-3 text-sm font-medium">
              启用 AI 功能（总开关）
              <input v-model="form.enabled" type="checkbox" class="peer sr-only" :disabled="loading || saving" />
              <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
            </label>

            <fieldset class="grid gap-4 border-t pt-5">
              <legend class="text-sm font-medium">Provider 配置</legend>
              <div class="grid max-w-sm gap-2">
                <label for="ai-protocol" class="text-sm font-medium leading-none">协议</label>
                <select
                  id="ai-protocol"
                  v-model="form.protocol"
                  class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                  :disabled="loading || saving"
                >
                  <option value="openai">OpenAI 兼容</option>
                  <option value="anthropic">Anthropic Claude</option>
                </select>
              </div>
              <div class="grid gap-4 sm:grid-cols-2">
                <div class="grid gap-2">
                  <label for="ai-base-url" class="text-sm font-medium leading-none">Base URL</label>
                  <Input id="ai-base-url" v-model="form.base_url" type="url" :placeholder="baseUrlPlaceholder" :disabled="loading || saving" />
                  <p class="text-xs text-muted-foreground">{{ baseUrlHint }}</p>
                </div>
                <div class="grid gap-2">
                  <label for="ai-model" class="text-sm font-medium leading-none">Model</label>
                  <Input id="ai-model" v-model="form.model" :placeholder="modelPlaceholder" :disabled="loading || saving" />
                </div>
              </div>
              <div class="grid max-w-sm gap-2">
                <label for="ai-api-key" class="text-sm font-medium leading-none">
                  API Key
                  <span class="text-xs text-muted-foreground">（{{ apiKeySet ? '已设置' : '未设置' }}）</span>
                </label>
                <Input
                  id="ai-api-key"
                  v-model="newApiKey"
                  type="password"
                  autocomplete="new-password"
                  :placeholder="apiKeySet ? '留空保持当前密钥' : '输入 API Key'"
                  :disabled="loading || saving"
                  @update:model-value="clearApiKey = false"
                />
                <div v-if="apiKeySet && !newApiKey" class="space-y-1.5">
                  <Button
                    v-if="!clearApiKey"
                    type="button"
                    variant="outline"
                    size="sm"
                    :disabled="loading || saving"
                    @click="clearApiKey = true"
                  >
                    清除已保存的密钥
                  </Button>
                  <p v-else class="flex items-center justify-between gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-2.5 py-1.5 text-xs text-destructive">
                    保存后将清除 API Key
                    <button type="button" class="font-medium underline" :disabled="saving" @click="clearApiKey = false">撤销</button>
                  </p>
                </div>
              </div>
            </fieldset>

            <fieldset class="grid gap-4 border-t pt-5">
              <legend class="text-sm font-medium">Embedding 配置（可选）</legend>
              <p class="text-xs text-muted-foreground">
                用于相关文章推荐与站内 AI 问答；推荐本地 Ollama（免费）。
              </p>
              <div class="grid gap-4 sm:grid-cols-2">
                <div class="grid gap-2">
                  <label for="ai-embedding-base-url" class="text-sm font-medium leading-none">Embedding Base URL</label>
                  <Input id="ai-embedding-base-url" v-model="form.embedding_base_url" type="url" placeholder="http://localhost:11434/v1" :disabled="loading || saving" />
                  <p class="text-xs text-muted-foreground">留空保持不变；OpenAI 兼容的 Embedding 服务地址。</p>
                </div>
                <div class="grid gap-2">
                  <label for="ai-embedding-model" class="text-sm font-medium leading-none">Embedding Model</label>
                  <Input id="ai-embedding-model" v-model="form.embedding_model" placeholder="bge-m3" :disabled="loading || saving" />
                  <p class="text-xs text-muted-foreground">留空保持不变。</p>
                </div>
              </div>
            </fieldset>

            <fieldset class="grid gap-3 border-t pt-5">
              <legend class="text-sm font-medium">功能开关</legend>
              <label class="flex cursor-pointer items-center justify-between gap-3 text-sm">
                <span>
                  编辑器助手
                  <span class="block text-xs font-normal text-muted-foreground">文章编辑器内的改写 / 摘要 / SEO 建议 / 标签推荐 / AI 导读</span>
                </span>
                <input v-model="form.editor_assist" type="checkbox" class="peer sr-only" :disabled="loading || saving" />
                <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
              </label>
              <label class="flex cursor-pointer items-center justify-between gap-3 text-sm">
                <span>
                  评论自动审核
                  <span class="block text-xs font-normal text-muted-foreground">访客评论提交后由 AI 评估风险等级，仅高置信垃圾评论自动标记</span>
                </span>
                <input v-model="form.comment_moderation" type="checkbox" class="peer sr-only" :disabled="loading || saving" />
                <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
              </label>
              <label class="flex cursor-pointer items-center justify-between gap-3 text-sm">
                <span>
                  智能搜索
                  <span class="block text-xs font-normal text-muted-foreground">开启后启用相关文章推荐与站内 AI 问答；需配置 Embedding 端点，推荐本地 Ollama（免费）</span>
                </span>
                <input v-model="form.smart_search" type="checkbox" class="peer sr-only" :disabled="loading || saving" />
                <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
              </label>
            </fieldset>

            <p v-if="error" role="alert" class="text-xs font-medium text-destructive">{{ error }}</p>
          </CardContent>
          <CardFooter class="justify-between gap-4 border-t pt-6">
            <p v-if="message" role="status" class="flex items-center gap-1.5 text-xs text-muted-foreground">
              <CheckCircle2Icon class="size-4 text-primary" />
              {{ message }}
            </p>
            <span v-else />
            <Button type="submit" :disabled="loading || saving">
              <LoaderCircleIcon v-if="saving" class="animate-spin" />
              {{ saving ? '保存中' : '保存设置' }}
            </Button>
          </CardFooter>
        </form>
      </Card>

      <Card>
        <CardHeader class="flex-row items-center justify-between gap-4 space-y-0">
          <div>
            <CardTitle class="text-base">用量记录</CardTitle>
            <CardDescription>每次 AI 请求的审计记录，按时间倒序。</CardDescription>
          </div>
          <select
            :value="usageFeature"
            aria-label="按功能筛选"
            class="flex h-8 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
            @change="handleFeatureFilter(($event.target as HTMLSelectElement).value)"
          >
            <option value="">全部功能</option>
            <option value="editor_rewrite">编辑器 · 改写</option>
            <option value="editor_summary">编辑器 · 摘要</option>
            <option value="editor_metadata">编辑器 · SEO 建议</option>
            <option value="editor_tags">编辑器 · 标签推荐</option>
            <option value="editor_brief">编辑器 · AI 导读</option>
            <option value="comment_moderation">评论自动审核</option>
          </select>
        </CardHeader>
        <CardContent class="p-0">
          <p v-if="usageError" class="border-b px-4 py-3 text-sm text-destructive">{{ usageError }}</p>
          <div v-if="usageLoading" class="space-y-2 p-4">
            <Skeleton v-for="index in 5" :key="index" class="h-10 w-full" />
          </div>
          <AppEmptyState
            v-else-if="usageItems.length === 0"
            :icon="SparklesIcon"
            title="还没有 AI 请求记录"
            description="使用编辑器助手或开启评论自动审核后，请求会记录在这里。"
          />
          <div v-else class="overflow-x-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>时间</TableHead>
                  <TableHead>功能</TableHead>
                  <TableHead>模型</TableHead>
                  <TableHead>状态</TableHead>
                  <TableHead class="text-right">Token（输入 + 输出）</TableHead>
                  <TableHead class="text-right">耗时</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow v-for="item in usageItems" :key="item.id">
                  <TableCell class="whitespace-nowrap tabular-nums text-xs text-muted-foreground">
                    {{ formatTime(item.created_at) }}
                  </TableCell>
                  <TableCell class="text-xs">{{ featureMeta[item.feature] ?? item.feature }}</TableCell>
                  <TableCell class="max-w-36 truncate font-mono text-xs">{{ item.model || '-' }}</TableCell>
                  <TableCell>
                    <Badge :variant="statusMeta[item.status].variant" class="text-xs">
                      {{ statusMeta[item.status].label }}
                    </Badge>
                    <span v-if="item.error_category" class="ml-1 text-xs text-muted-foreground">{{ item.error_category }}</span>
                  </TableCell>
                  <TableCell class="text-right text-xs tabular-nums">{{ tokenText(item) }}</TableCell>
                  <TableCell class="text-right text-xs tabular-nums text-muted-foreground">{{ item.latency_ms }} ms</TableCell>
                </TableRow>
              </TableBody>
            </Table>
          </div>
        </CardContent>
        <CardFooter class="h-auto">
          <AppDataTablePagination
            v-model:page="usagePage"
            v-model:page-size="usagePageSize"
            :total="usageTotal"
            :loading="usageLoading"
            unit="条记录"
          />
        </CardFooter>
      </Card>
    </div>
  </SettingsLayout>
</template>

<route lang="yaml">
meta:
  permission: settings:manage
</route>
