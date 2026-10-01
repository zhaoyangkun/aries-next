<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import {
  CircleAlertIcon,
  FileTextIcon,
  FileUpIcon,
  LoaderCircleIcon,
  TriangleAlertIcon,
  UploadIcon,
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
  IMPORT_ACCEPT,
  buildCommitItems,
  importsApi,
  validateImportFiles,
  type CommitImportResult,
  type ImportJob,
  type ImportStrategy,
} from '@/modules/media/api/media'
import { getApiError } from '@/shared/api/client'

type ImportStep = 'select' | 'preview' | 'done'

const props = defineProps<{
  open: boolean
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  imported: [result: CommitImportResult]
}>()

// Backend 解析为 Background Job，轮询等待预览就绪。
const POLL_INTERVAL_MS = 800
const POLL_MAX_ATTEMPTS = 20

const step = ref<ImportStep>('select')
const fileInput = ref<HTMLInputElement | null>(null)
const selectedFiles = ref<File[]>([])
const selectError = ref('')
const uploading = ref(false)
const progress = ref(0)
const job = ref<ImportJob | null>(null)
const strategies = ref<Record<number, ImportStrategy>>({})
const committing = ref(false)
const commitError = ref('')
const result = ref<CommitImportResult | null>(null)
let pollTimer: ReturnType<typeof setTimeout> | undefined

const conflictCount = computed(
  () => job.value?.items.filter((item) => item.slug_conflict).length ?? 0,
)

watch(
  () => props.open,
  (open) => {
    if (!open) return
    resetWizard()
  },
)

onBeforeUnmount(() => {
  cancelPoll()
})

function cancelPoll() {
  if (pollTimer) clearTimeout(pollTimer)
  pollTimer = undefined
}

function resetWizard() {
  cancelPoll()
  step.value = 'select'
  selectedFiles.value = []
  selectError.value = ''
  uploading.value = false
  progress.value = 0
  job.value = null
  strategies.value = {}
  committing.value = false
  commitError.value = ''
  result.value = null
}

function triggerSelect() {
  fileInput.value?.click()
}

function handleFileChange(event: Event) {
  const input = event.target as HTMLInputElement
  const files = Array.from(input.files ?? [])
  input.value = ''
  selectError.value = ''
  const validationError = validateImportFiles(files)
  if (validationError) {
    selectError.value = validationError
    selectedFiles.value = []
    return
  }
  selectedFiles.value = files
}

async function startImport() {
  if (selectedFiles.value.length === 0 || uploading.value) return
  uploading.value = true
  progress.value = 0
  selectError.value = ''
  try {
    const created = await importsApi.createImport(selectedFiles.value, (percent) => {
      progress.value = percent
    })
    job.value = created
    if (created.status === 'done') {
      enterPreview(created)
    } else if (created.status === 'failed') {
      selectError.value = '导入解析失败，请检查 Markdown 文件内容'
    } else {
      pollJob(created.job_id, 0)
      return
    }
  } catch (requestError) {
    selectError.value = getApiError(requestError, '导入失败')
  } finally {
    uploading.value = false
  }
}

// 预览未就绪时按固定间隔轮询，超过次数上限后提示手动重试。
function pollJob(jobId: number, attempt: number) {
  cancelPoll()
  pollTimer = setTimeout(async () => {
    try {
      const latest = await importsApi.getImport(jobId)
      job.value = latest
      if (latest.status === 'done') {
        uploading.value = false
        enterPreview(latest)
        return
      }
      if (latest.status === 'failed') {
        uploading.value = false
        selectError.value = '导入解析失败，请检查 Markdown 文件内容'
        return
      }
      if (attempt + 1 >= POLL_MAX_ATTEMPTS) {
        uploading.value = false
        selectError.value = '导入解析超时，请稍后重试'
        return
      }
      pollJob(jobId, attempt + 1)
    } catch (requestError) {
      uploading.value = false
      selectError.value = getApiError(requestError, '导入状态查询失败')
    }
  }, POLL_INTERVAL_MS)
}

function enterPreview(readyJob: ImportJob) {
  // 冲突项默认 skip，由用户逐条改为 rename。
  const defaults: Record<number, ImportStrategy> = {}
  for (const item of readyJob.items) {
    if (item.slug_conflict) defaults[item.index] = 'skip'
  }
  strategies.value = defaults
  step.value = 'preview'
}

async function commitImport() {
  const currentJob = job.value
  if (!currentJob || committing.value) return
  committing.value = true
  commitError.value = ''
  try {
    const committed = await importsApi.commitImport(
      currentJob.job_id,
      buildCommitItems(currentJob.items, strategies.value),
    )
    result.value = committed
    step.value = 'done'
    emit('imported', committed)
  } catch (requestError) {
    commitError.value = getApiError(requestError, '导入提交失败')
  } finally {
    committing.value = false
  }
}

function handleOpenChange(open: boolean) {
  if (!open && (uploading.value || committing.value)) return
  emit('update:open', open)
}
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="flex h-[min(600px,calc(100dvh-2rem))] w-[calc(100%-1rem)] max-w-2xl! flex-col gap-0 overflow-hidden p-0">
      <DialogHeader class="border-b px-4 py-4 text-left sm:px-5">
        <DialogTitle class="text-base">导入 Markdown</DialogTitle>
        <DialogDescription class="mt-0.5">
          批量导入 .md 文件（最多 10 个，单个不超过 2MB），所有导入文章一律保存为草稿。
        </DialogDescription>
      </DialogHeader>

      <div class="min-h-0 flex-1 overflow-y-auto p-4 sm:p-5">
        <!-- 第一步：选择文件 -->
        <div v-if="step === 'select'" class="space-y-4">
          <button
            type="button"
            class="flex w-full flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed px-6 py-10 text-center outline-none transition-colors hover:border-primary focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
            :disabled="uploading"
            @click="triggerSelect"
          >
            <FileUpIcon class="size-6 text-muted-foreground" />
            <span class="text-sm font-medium">点击选择 Markdown 文件</span>
            <span class="text-xs text-muted-foreground">支持 YAML Front Matter（title / slug / tags / category / summary）</span>
          </button>
          <input
            ref="fileInput"
            type="file"
            class="hidden"
            multiple
            :accept="IMPORT_ACCEPT"
            @change="handleFileChange"
          />
          <ul v-if="selectedFiles.length" class="divide-y overflow-hidden rounded-lg border text-xs">
            <li v-for="file in selectedFiles" :key="file.name" class="flex items-center gap-2 px-3 py-2">
              <FileTextIcon class="size-3.5 shrink-0 text-muted-foreground" />
              <span class="min-w-0 flex-1 truncate">{{ file.name }}</span>
              <span class="text-muted-foreground">{{ (file.size / 1024).toFixed(1) }} KB</span>
            </li>
          </ul>
          <div v-if="uploading" class="space-y-1">
            <div class="h-1.5 w-full overflow-hidden rounded-full bg-muted">
              <div class="h-full bg-primary transition-all" :style="{ width: `${progress}%` }" />
            </div>
            <p class="flex items-center gap-1.5 text-xs text-muted-foreground">
              <LoaderCircleIcon class="size-3.5 animate-spin" />
              正在上传并解析（{{ progress }}%）…
            </p>
          </div>
          <p v-if="selectError" role="alert" class="flex items-center gap-1.5 text-xs font-medium text-destructive">
            <CircleAlertIcon class="size-3.5 shrink-0" />
            {{ selectError }}
          </p>
        </div>

        <!-- 第二步：预览与冲突策略 -->
        <div v-else-if="step === 'preview' && job" class="space-y-3">
          <p class="text-xs text-muted-foreground">
            共解析 {{ job.items.length }} 篇，其中 {{ conflictCount }} 篇 Slug 与现有文章冲突。
          </p>
          <ul class="space-y-2">
            <li v-for="item in job.items" :key="item.index" class="space-y-2 rounded-lg border p-3">
              <div class="flex min-w-0 items-start justify-between gap-3">
                <div class="min-w-0">
                  <p class="truncate text-sm font-medium">{{ item.title || '（无标题）' }}</p>
                  <p class="mt-0.5 truncate text-xs text-muted-foreground">
                    {{ item.file_name }} · /{{ item.slug || '（无 Slug）' }}
                  </p>
                </div>
                <Badge v-if="item.slug_conflict" variant="destructive" class="shrink-0">Slug 冲突</Badge>
                <Badge v-else variant="secondary" class="shrink-0">可导入</Badge>
              </div>
              <ul v-if="item.warnings.length" class="space-y-1">
                <li
                  v-for="warning in item.warnings"
                  :key="warning"
                  class="flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-500"
                >
                  <TriangleAlertIcon class="size-3 shrink-0" />
                  {{ warning }}
                </li>
              </ul>
              <div v-if="item.slug_conflict" class="flex items-center gap-4 text-xs" :aria-label="`冲突处理：${item.title}`">
                <label class="flex cursor-pointer items-center gap-1.5">
                  <input
                    v-model="strategies[item.index]"
                    type="radio"
                    value="skip"
                    class="accent-primary"
                    :disabled="committing"
                  />
                  跳过该篇
                </label>
                <label class="flex cursor-pointer items-center gap-1.5">
                  <input
                    v-model="strategies[item.index]"
                    type="radio"
                    value="rename"
                    class="accent-primary"
                    :disabled="committing"
                  />
                  自动重命名 Slug 后导入
                </label>
              </div>
            </li>
          </ul>
          <p v-if="commitError" role="alert" class="flex items-center gap-1.5 text-xs font-medium text-destructive">
            <CircleAlertIcon class="size-3.5 shrink-0" />
            {{ commitError }}
          </p>
        </div>

        <!-- 第三步：结果 -->
        <div v-else-if="step === 'done' && result" class="space-y-4">
          <div class="rounded-lg border border-emerald-500/40 bg-emerald-500/10 p-4 text-center">
            <p class="text-sm font-medium text-emerald-600 dark:text-emerald-500">
              已导入 {{ result.created.length }} 篇草稿
            </p>
            <p class="mt-1 text-xs text-muted-foreground">
              跳过 {{ result.skipped.length }} 篇，自动重命名 {{ result.renamed.length }} 篇。导入的文章全部保存为草稿，不会直接发布。
            </p>
          </div>
          <ul v-if="result.created.length" class="divide-y overflow-hidden rounded-lg border text-xs">
            <li v-for="created in result.created" :key="created.id" class="flex min-w-0 items-center gap-2 px-3 py-2">
              <FileTextIcon class="size-3.5 shrink-0 text-muted-foreground" />
              <span class="min-w-0 flex-1 truncate font-medium">{{ created.title }}</span>
              <span class="truncate text-muted-foreground">/{{ created.slug }}</span>
            </li>
          </ul>
          <ul v-if="result.renamed.length" class="space-y-1 text-xs text-muted-foreground">
            <li v-for="entry in result.renamed" :key="entry.from">
              Slug 重命名：/{{ entry.from }} → /{{ entry.to }}
            </li>
          </ul>
        </div>
      </div>

      <DialogFooter class="flex-row items-center justify-end border-t px-4 py-3 sm:px-5">
        <template v-if="step === 'select'">
          <Button variant="outline" :disabled="uploading" @click="emit('update:open', false)">取消</Button>
          <Button :disabled="selectedFiles.length === 0 || uploading" @click="startImport">
            <LoaderCircleIcon v-if="uploading" class="animate-spin" />
            <UploadIcon v-else />
            {{ uploading ? '正在解析' : `开始导入${selectedFiles.length ? `（${selectedFiles.length} 个文件）` : ''}` }}
          </Button>
        </template>
        <template v-else-if="step === 'preview'">
          <Button variant="outline" :disabled="committing" @click="resetWizard">重新选择</Button>
          <Button :disabled="committing" @click="commitImport">
            <LoaderCircleIcon v-if="committing" class="animate-spin" />
            {{ committing ? '正在导入' : '确认导入为草稿' }}
          </Button>
        </template>
        <Button v-else @click="emit('update:open', false)">完成</Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
