<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  CheckIcon,
  ClipboardCopyIcon,
  LoaderCircleIcon,
  RefreshCwIcon,
  SparklesIcon,
  TriangleAlertIcon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  getAiErrorMessage,
  streamAiEditor,
  type AiStreamHandle,
  type AiStreamStart,
  type AiStreamUsage,
} from '@/modules/ai/api/ai'

export type AiAssistFeature = 'rewrite' | 'summary' | 'metadata' | 'tags' | 'brief'

// AI 建议结果：只有用户点击「填入/替换」才由父组件写回表单（Phase 08 原则）。
export interface AiAssistApplyPayload {
  text?: string
  slug?: string
  keywords?: string[]
  description?: string
  tags?: string[]
}

const props = defineProps<{
  open: boolean
  feature: AiAssistFeature
  // 打开瞬间捕获的请求入参：rewrite 为 { text }，summary/metadata 为 { title, content }。
  request: Record<string, string>
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  apply: [payload: AiAssistApplyPayload]
}>()

type Phase = 'streaming' | 'done' | 'error' | 'cancelled'

const phase = ref<Phase>('streaming')
const outputText = ref('')
const startInfo = ref<AiStreamStart | null>(null)
const usage = ref<AiStreamUsage | null>(null)
const errorMessage = ref('')
const copied = ref(false)
let activeStream: AiStreamHandle | null = null
// 用户主动取消与组件关闭都会 abort，用标记区分于真实错误。
let userCancelled = false

const featureMeta: Record<AiAssistFeature, { title: string, description: string, applyLabel: string }> = {
  rewrite: {
    title: '改写选中文本',
    description: 'AI 将改写打开面板时选中的片段；确认「替换选中文本」后才会写回正文。',
    applyLabel: '替换选中文本',
  },
  summary: {
    title: '生成摘要',
    description: '基于当前标题与正文生成摘要草稿；确认后填入摘要字段。',
    applyLabel: '填入摘要字段',
  },
  metadata: {
    title: 'SEO 建议',
    description: '基于当前标题与正文建议 Slug、关键词与描述；确认「填入表单」后映射到对应字段（描述填入摘要）。',
    applyLabel: '填入表单',
  },
  tags: {
    title: '标签推荐',
    description: '基于当前标题与正文推荐最多 5 个标签；确认后按名称匹配并选中，不存在的标签会自动创建。',
    applyLabel: '选中这些标签',
  },
  brief: {
    title: 'AI 导读',
    description: '基于当前标题与正文生成 150 字以内的导读；确认后填入导读字段，可继续手动编辑。',
    applyLabel: '填入导读字段',
  },
}

// metadata 输出是 JSON 文本：服务端 done 前已校验合法性，此处解析做结构化展示。
const parsedMetadata = computed<AiAssistApplyPayload | null>(() => {
  if (props.feature !== 'metadata' || phase.value !== 'done') return null
  try {
    const parsed = JSON.parse(outputText.value.trim()) as Record<string, unknown>
    return {
      slug: typeof parsed.slug === 'string' ? parsed.slug : undefined,
      keywords: Array.isArray(parsed.keywords)
        ? parsed.keywords.filter((item): item is string => typeof item === 'string')
        : undefined,
      description: typeof parsed.description === 'string' ? parsed.description : undefined,
    }
  }
  catch {
    return null
  }
})

// tags 输出为严格 JSON：{"tags": ["标签1", ...]}；解析失败返回 null，由界面给出错误提示。
const parsedTags = computed<string[] | null>(() => {
  if (props.feature !== 'tags' || phase.value !== 'done') return null
  try {
    const parsed = JSON.parse(outputText.value.trim()) as Record<string, unknown>
    if (!Array.isArray(parsed.tags)) return null
    return parsed.tags.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
  }
  catch {
    return null
  }
})

const canApply = computed(() => {
  if (phase.value !== 'done' || !outputText.value.trim()) return false
  if (props.feature === 'metadata') {
    const parsed = parsedMetadata.value
    return Boolean(parsed && (parsed.slug || parsed.description || parsed.keywords?.length))
  }
  if (props.feature === 'tags') {
    return Boolean(parsedTags.value?.length)
  }
  return true
})

watch(
  () => props.open,
  (open) => {
    if (open) startStream()
    else stopStream()
  },
)

function stopStream() {
  if (activeStream) {
    userCancelled = true
    activeStream.cancel()
    activeStream = null
  }
}

function startStream() {
  stopStream()
  phase.value = 'streaming'
  outputText.value = ''
  startInfo.value = null
  usage.value = null
  errorMessage.value = ''
  copied.value = false
  userCancelled = false

  const handle = streamAiEditor(props.feature, props.request, {
    onStart: info => (startInfo.value = info),
    onDelta: text => (outputText.value += text),
    onUsage: value => (usage.value = value),
    onDone: () => {
      phase.value = 'done'
      activeStream = null
    },
  })
  activeStream = handle
  handle.finished.catch((streamError: unknown) => {
    activeStream = null
    if (userCancelled) {
      phase.value = 'cancelled'
      return
    }
    const code = typeof (streamError as { code?: unknown })?.code === 'string'
      ? (streamError as { code: string }).code
      : ''
    errorMessage.value = getAiErrorMessage(code)
    phase.value = 'error'
  })
}

function cancelStream() {
  stopStream()
  phase.value = 'cancelled'
}

function regenerate() {
  startStream()
}

async function copyOutput() {
  try {
    await navigator.clipboard.writeText(outputText.value)
    copied.value = true
    setTimeout(() => (copied.value = false), 2000)
  }
  catch {
    copied.value = false
  }
}

function applyResult() {
  if (!canApply.value) return
  if (props.feature === 'metadata') {
    emit('apply', parsedMetadata.value ?? {})
  }
  else if (props.feature === 'tags') {
    emit('apply', { tags: parsedTags.value ?? [] })
  }
  else {
    emit('apply', { text: outputText.value.trim() })
  }
  emit('update:open', false)
}

function handleOpenChange(open: boolean) {
  // 打开状态由父组件控制；关闭时中止流，未确认的内容绝不写回正文。
  emit('update:open', open)
}
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="flex h-[min(560px,calc(100dvh-2rem))] w-[calc(100%-1rem)] max-w-2xl! flex-col gap-0 overflow-hidden p-0">
      <DialogHeader class="border-b px-4 py-4 text-left sm:px-5">
        <DialogTitle class="flex items-center gap-1.5 text-base">
          <SparklesIcon class="size-4 text-primary" />
          AI 助手 · {{ featureMeta[feature].title }}
        </DialogTitle>
        <DialogDescription class="mt-0.5">{{ featureMeta[feature].description }}</DialogDescription>
      </DialogHeader>

      <div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-4 sm:p-5">
        <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <Badge v-if="phase === 'streaming'" variant="secondary">
            <LoaderCircleIcon class="mr-1 size-3 animate-spin" />
            正在生成
          </Badge>
          <Badge v-else-if="phase === 'done'" variant="default">已完成</Badge>
          <Badge v-else-if="phase === 'cancelled'" variant="outline">已取消</Badge>
          <Badge v-else variant="destructive">生成失败</Badge>
          <span v-if="startInfo?.model" class="font-mono">{{ startInfo.model }}</span>
          <span v-if="usage" class="tabular-nums">
            Token：{{ usage.prompt_tokens ?? 0 }} 输入 + {{ usage.completion_tokens ?? 0 }} 输出
          </span>
        </div>

        <p v-if="phase === 'error'" role="alert" class="flex items-start gap-1.5 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2.5 text-xs font-medium leading-5 text-destructive">
          <TriangleAlertIcon class="mt-0.5 size-3.5 shrink-0" />
          {{ errorMessage }}
        </p>

        <!-- metadata 完成后结构化展示；流式期间展示原始文本。 -->
        <dl v-if="feature === 'metadata' && parsedMetadata" class="space-y-3 rounded-md border bg-muted/30 p-4 text-sm">
          <div v-if="parsedMetadata.slug">
            <dt class="text-xs font-medium text-muted-foreground">Slug</dt>
            <dd class="mt-0.5 font-mono text-sm">{{ parsedMetadata.slug }}</dd>
          </div>
          <div v-if="parsedMetadata.keywords?.length">
            <dt class="text-xs font-medium text-muted-foreground">关键词</dt>
            <dd class="mt-1 flex flex-wrap gap-1.5">
              <Badge v-for="keyword in parsedMetadata.keywords" :key="keyword" variant="secondary">{{ keyword }}</Badge>
            </dd>
          </div>
          <div v-if="parsedMetadata.description">
            <dt class="text-xs font-medium text-muted-foreground">描述</dt>
            <dd class="mt-0.5 leading-6">{{ parsedMetadata.description }}</dd>
          </div>
        </dl>
        <div v-else-if="feature === 'tags' && parsedTags" class="rounded-md border bg-muted/30 p-4">
          <p class="text-xs font-medium text-muted-foreground">推荐标签</p>
          <div class="mt-1.5 flex flex-wrap gap-1.5">
            <Badge v-for="tag in parsedTags" :key="tag" variant="secondary">{{ tag }}</Badge>
          </div>
        </div>
        <div
          v-else
          class="min-h-32 whitespace-pre-wrap rounded-md border bg-muted/30 p-4 text-sm leading-6"
          aria-live="polite"
        >{{ outputText }}<span v-if="phase === 'streaming'" class="ml-0.5 inline-block h-4 w-2 animate-pulse bg-primary/70 align-text-bottom" /></div>

        <p v-if="(feature === 'metadata' || feature === 'tags') && phase === 'done' && (feature === 'metadata' ? !parsedMetadata : !parsedTags)" class="text-xs text-muted-foreground">
          未能解析 AI 输出的 JSON，可直接复制原始文本，或点击「重新生成」。
        </p>
        <p v-if="phase === 'cancelled'" class="text-xs text-muted-foreground">
          已取消生成，正文未被修改。可点击「重新生成」重试。
        </p>
      </div>

      <DialogFooter class="flex-row flex-wrap items-center justify-end gap-2 border-t px-4 py-3 sm:px-5">
        <Button v-if="phase === 'streaming'" variant="outline" @click="cancelStream">取消生成</Button>
        <Button
          variant="outline"
          :disabled="phase === 'streaming' || !outputText"
          @click="copyOutput"
        >
          <CheckIcon v-if="copied" />
          <ClipboardCopyIcon v-else />
          {{ copied ? '已复制' : '复制' }}
        </Button>
        <Button variant="outline" :disabled="phase === 'streaming'" @click="regenerate">
          <RefreshCwIcon />
          重新生成
        </Button>
        <Button :disabled="!canApply" @click="applyResult">
          {{ featureMeta[feature].applyLabel }}
        </Button>
        <Button variant="ghost" :disabled="phase === 'streaming'" @click="emit('update:open', false)">关闭</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
