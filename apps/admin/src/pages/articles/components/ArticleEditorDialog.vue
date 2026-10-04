<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import type Vditor from 'vditor'
import {
  ChevronDownIcon,
  CircleAlertIcon,
  HistoryIcon,
  LoaderCircleIcon,
  LockIcon,
  PlusIcon,
  RotateCcwIcon,
  SaveIcon,
  SendIcon,
  SparklesIcon,
  TriangleAlertIcon,
  XIcon,
} from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import {
  articlesApi,
  type AdminArticle,
  type ArticleCategory,
  type ArticleRevision,
  type ArticleTag,
  type CreateArticlePayload,
} from '@/modules/articles/api/articles'
import {
  clearBackup,
  isBackupNewer,
  readBackup,
  useArticleAutosave,
  type ArticleBackup,
} from '@/modules/articles/composables/useArticleAutosave'
import { useSessionStore } from '@/modules/auth/stores/session'
import {
  MEDIA_MAX_FILE_SIZE,
  MEDIA_UPLOAD_ACCEPT,
  mediaApi,
  validateMediaFiles,
} from '@/modules/media/api/media'
import type { MediaAsset } from '@/modules/media/api/media'
import { getApiError, getApiErrorCode } from '@/shared/api/client'
import AppConfirmDialog from '@/shared/components/AppConfirmDialog.vue'
import AppMediaPicker from '@/shared/components/AppMediaPicker.vue'
import ArticleAiAssistDialog, {
  type AiAssistApplyPayload,
  type AiAssistFeature,
} from './ArticleAiAssistDialog.vue'

const props = defineProps<{
  open: boolean
  article?: AdminArticle | null
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  saved: [article: AdminArticle]
}>()

const session = useSessionStore()

const title = ref('')
const slug = ref('')
const summary = ref('')
const aiBrief = ref('')
const content = ref('')
const coverUrl = ref('')
const seoKeywords = ref<string[]>([])
const keywordInput = ref('')
const keywordError = ref('')
const allowComments = ref(true)
const isPinned = ref(false)
const categoryId = ref<number | null>(null)
const tagIds = ref<number[]>([])
const passwordProtected = ref(false)
const newPassword = ref('')
const clearPassword = ref(false)
const passwordError = ref('')
const categories = ref<ArticleCategory[]>([])
const tags = ref<ArticleTag[]>([])
const taxonomyLoading = ref(false)
const taxonomyError = ref('')
const newCategoryName = ref('')
const newTagName = ref('')
const creatingTaxonomy = ref<'category' | 'tag' | null>(null)
const saving = ref(false)
const saveIntent = ref<'draft' | 'publish' | null>(null)
const loadingArticle = ref(false)
const loadedArticle = ref<AdminArticle | null>(null)
const saveError = ref('')
const titleError = ref('')
const conflict = ref(false)
const discardConfirmOpen = ref(false)
const revisions = ref<ArticleRevision[]>([])
const revisionsLoading = ref(false)
const revisionsError = ref('')
const expandedRevisionNo = ref<number | null>(null)
const restoreConfirmOpen = ref(false)
const pendingRestore = ref<ArticleRevision | null>(null)
const restoring = ref(false)
const savedSnapshot = ref('')
const coverPickerOpen = ref(false)
const backupPrompt = ref<ArticleBackup | null>(null)
const editorNotice = ref('')
// AI 助手面板状态：request 在打开瞬间快照，流式期间正文继续编辑不影响本次请求。
const aiDialogOpen = ref(false)
const aiFeature = ref<AiAssistFeature>('rewrite')
const aiRequest = ref<Record<string, string>>({})
const applyingAiTags = ref(false)
const vditorEl = ref<HTMLElement | null>(null)
let vditorInstance: Vditor | null = null
// setValue 等程序化写入不应触发自动保存调度。
let applyingServerState = false
let noticeTimer: ReturnType<typeof setTimeout> | undefined

const characterCount = computed(() => content.value.trim().length)
const editing = computed(() => Boolean(loadedArticle.value ?? props.article))
const currentArticleId = computed(() => loadedArticle.value?.id ?? props.article?.id ?? null)
const currentStatus = computed(() => loadedArticle.value?.status ?? props.article?.status ?? 'draft')
const canSave = computed(() => !loadingArticle.value && (!props.article || loadedArticle.value !== null))
const currentStatusText = computed(() => {
  if (currentStatus.value === 'published') return '已发布'
  if (currentStatus.value === 'recycled') return '回收站'
  return '草稿'
})
// 快照对比驱动 Unsaved Guard：任何字段偏离最近一次加载/保存的状态都视为未保存修改。
const isDirty = computed(() => props.open && formSnapshot() !== savedSnapshot.value)

// 自动保存：既有文章防抖 PUT（带乐观锁），新文章只写 localStorage emergency backup。
const autosave = useArticleAutosave({
  articleId: () => currentArticleId.value,
  baseUpdatedAt: () => loadedArticle.value?.updated_at ?? props.article?.updated_at ?? null,
  buildSnapshot: () => ({ payload: buildPayload(), snapshot: formSnapshot() }),
  save: (articleId, payload) => {
    const base = loadedArticle.value ?? props.article
    return articlesApi.update(articleId, { ...payload, expected_version: base?.version ?? 0 })
  },
  onSaved: (saved, sent) => {
    // 只更新版本与“已保存”基线，不回写表单，避免覆盖保存期间的新输入。
    loadedArticle.value = saved
    savedSnapshot.value = sent.snapshot
  },
  onConflict: () => {
    conflict.value = true
  },
})

const autosaveStatusText = computed(() => {
  switch (autosave.phase.value) {
    case 'pending':
      return '等待自动保存…'
    case 'saving':
      return '正在自动保存…'
    case 'saved':
      return '已自动保存'
    case 'offline':
      return '离线：已备份到本地，网络恢复后将自动续存'
    case 'conflict':
      return '版本冲突，等待处理'
    case 'failed':
      return autosave.lastError.value || '自动保存失败'
    case 'backed_up':
      return '已备份到本地（尚未保存到服务器）'
    default:
      return ''
  }
})

const autosaveStatusTone = computed(() => {
  if (autosave.phase.value === 'failed' || autosave.phase.value === 'conflict') return 'text-destructive'
  if (autosave.phase.value === 'offline' || autosave.phase.value === 'backed_up') return 'text-amber-600 dark:text-amber-500'
  return 'text-muted-foreground'
})

watch(
  () => props.open,
  async (open) => {
    if (!open) {
      destroyVditor()
      autosave.reset()
      return
    }
    resetEditor()
    loadTaxonomy()
    await nextTick()
    void createVditor()
    if (props.article) void loadArticle(props.article.id)
    else checkBackupPrompt(null)
  },
)

// 任何表单字段变化都会触发自动保存调度；加载/恢复服务器状态期间不调度。
watch(formSnapshot, () => {
  if (!props.open || applyingServerState || loadingArticle.value) return
  if (formSnapshot() === savedSnapshot.value) return
  autosave.notifyChange()
})

function handleOnline() {
  // 网络恢复后，离线期间写入 localStorage 的快照自动续存。
  if (autosave.phase.value === 'offline') void autosave.flush()
}

window.addEventListener('online', handleOnline)

onBeforeUnmount(() => {
  window.removeEventListener('online', handleOnline)
  if (noticeTimer) clearTimeout(noticeTimer)
  destroyVditor()
  autosave.dispose()
})

// Vditor 体积较大，动态加载避免拖慢首屏；CSS 一并按需引入。
async function createVditor() {
  if (!vditorEl.value || vditorInstance) return
  const [{ default: VditorCtor }] = await Promise.all([
    import('vditor'),
    import('vditor/dist/index.css'),
  ])
  // 加载期间 Dialog 可能已关闭，此时容器已卸载，放弃创建。
  if (!vditorEl.value || vditorInstance) return
  vditorInstance = new VditorCtor(vditorEl.value, {
    mode: 'ir',
    // 运行期资产（lute / hljs / KaTeX / icons 等）走 npmmirror（淘宝 npm 镜像，
    // 国内访问最快且稳定）；默认 unpkg 国内超时会卡编辑器加载。
    // 注意：升级 vditor 依赖时需同步下面的版本号。
    cdn: 'https://registry.npmmirror.com/vditor/3.11.3/files',
    height: 420,
    toolbarConfig: { pin: true },
    cache: { enable: false },
    counter: { enable: true, max: 100000 },
    placeholder: '使用 Markdown 编写正文...',
    value: content.value,
    input: (value) => {
      if (applyingServerState) return
      content.value = value
    },
    upload: {
      accept: MEDIA_UPLOAD_ACCEPT,
      multiple: true,
      max: MEDIA_MAX_FILE_SIZE,
      fieldName: 'file[]',
      // Vditor 类型未声明 Promise<string | null>，此处收窄为联合类型。
      handler: (files: File[]) => handleVditorUpload(files) as Promise<string> | Promise<null>,
    },
  })
}

function destroyVditor() {
  if (!vditorInstance) return
  try {
    vditorInstance.destroy()
  } catch {
    // 销毁阶段的内部异常不影响 Dialog 关闭。
  }
  vditorInstance = null
}

// Vditor 自定义上传：走媒体库接口，成功后逐个插入图片 Markdown；返回字符串即视为错误提示。
async function handleVditorUpload(files: File[]): Promise<string | null> {
  const validationError = validateMediaFiles(files)
  if (validationError) return validationError
  try {
    const uploaded = await mediaApi.upload(files)
    const markdown = uploaded
      .map((asset) => `![${asset.alt || asset.original_name}](${asset.url})`)
      .join('\n')
    vditorInstance?.insertValue(`${markdown}\n`)
    if (uploaded.some((asset) => asset.duplicate_of != null)) {
      showEditorNotice('部分图片与媒体库已有内容相同，已自动复用原文件。')
    }
    return null
  } catch (requestError) {
    return getApiError(requestError, '图片上传失败')
  }
}

function showEditorNotice(message: string) {
  editorNotice.value = message
  if (noticeTimer) clearTimeout(noticeTimer)
  noticeTimer = setTimeout(() => {
    editorNotice.value = ''
  }, 5000)
}

// AI 助手：改写需要当前选中文本；摘要/SEO/导读/标签需要已有正文。入参快照后打开流式面板。
function openAiAssist(feature: AiAssistFeature) {
  if (!vditorInstance) return
  if (feature === 'rewrite') {
    const selection = vditorInstance.getSelection().trim()
    if (!selection) {
      showEditorNotice('请先在正文中选中要改写的文本。')
      return
    }
    if (selection.length > 8000) {
      showEditorNotice('选中内容不能超过 8000 字。')
      return
    }
    aiRequest.value = { text: selection }
  }
  else {
    if (!content.value.trim()) {
      showEditorNotice('请先编写正文内容，再使用 AI 助手。')
      return
    }
    aiRequest.value = { title: title.value.trim(), content: content.value }
  }
  aiFeature.value = feature
  aiDialogOpen.value = true
}

// AI 结果只在用户确认后写回：改写走 Vditor 选区替换，摘要/SEO 映射到对应表单字段，
// 导读写入 aiBrief，标签按名称匹配现有列表（不存在的先创建再选中）。
function handleAiApply(payload: AiAssistApplyPayload) {
  if (aiFeature.value === 'rewrite' && payload.text) {
    vditorInstance?.deleteValue()
    vditorInstance?.insertValue(payload.text)
    return
  }
  if ((aiFeature.value === 'summary' || aiFeature.value === 'brief') && payload.text) {
    if (aiFeature.value === 'summary') summary.value = payload.text.trim()
    else aiBrief.value = payload.text.trim()
    return
  }
  if (aiFeature.value === 'metadata') {
    if (payload.slug) slug.value = payload.slug
    if (payload.description) summary.value = payload.description
    if (payload.keywords?.length) {
      // 与现有 SEO 关键词对齐：去重后追加，沿用 20 个上限。
      const merged = [...seoKeywords.value]
      for (const keyword of payload.keywords) {
        if (merged.length >= 20) break
        if (!merged.includes(keyword)) merged.push(keyword)
      }
      seoKeywords.value = merged
    }
  }
  if (aiFeature.value === 'tags' && payload.tags?.length) {
    void applyAiTags(payload.tags)
  }
}

// 标签推荐：按名称匹配现有标签，不存在的先创建再选中；重复推荐只处理一次。
async function applyAiTags(names: string[]) {
  if (applyingAiTags.value) return
  applyingAiTags.value = true
  try {
    const merged = [...tagIds.value]
    for (const name of names) {
      const trimmed = name.trim()
      if (!trimmed) continue
      let tag = tags.value.find(item => item.name === trimmed)
      if (!tag) {
        tag = await articlesApi.createTag(trimmed)
        tags.value = [...tags.value, tag]
      }
      if (!merged.includes(tag.id)) merged.push(tag.id)
    }
    tagIds.value = merged
    showEditorNotice('AI 推荐标签已选中，未存在的标签已自动创建。')
  } catch (requestError) {
    showEditorNotice(getApiError(requestError, '标签创建失败，部分标签未能选中'))
  } finally {
    applyingAiTags.value = false
  }
}

function handleCoverSelect(asset: MediaAsset) {
  coverUrl.value = asset.url
  coverPickerOpen.value = false
}

// 重新打开编辑器时，若本地备份比服务器版本更新，提示用户选择恢复或丢弃，绝不自动覆盖。
function checkBackupPrompt(article: AdminArticle | null) {
  const backup = readBackup(window.localStorage, article?.id ?? null)
  if (!backup) return
  if (article && !isBackupNewer(backup, article.updated_at)) {
    clearBackup(window.localStorage, article.id)
    return
  }
  backupPrompt.value = backup
}

function restoreBackup() {
  const backup = backupPrompt.value
  if (!backup) return
  const { payload } = backup
  title.value = payload.title
  slug.value = payload.slug ?? slug.value
  summary.value = payload.summary
  aiBrief.value = payload.ai_brief ?? ''
  content.value = payload.markdown_source
  vditorInstance?.setValue(payload.markdown_source)
  coverUrl.value = payload.cover_url ?? ''
  seoKeywords.value = [...(payload.seo_keywords ?? [])]
  allowComments.value = payload.allow_comments ?? true
  isPinned.value = payload.is_pinned ?? false
  categoryId.value = payload.category_id ?? null
  tagIds.value = [...(payload.tag_ids ?? [])]
  backupPrompt.value = null
}

function discardBackup() {
  clearBackup(window.localStorage, currentArticleId.value)
  backupPrompt.value = null
}

function formSnapshot() {
  return JSON.stringify({
    title: title.value,
    slug: slug.value,
    summary: summary.value,
    aiBrief: aiBrief.value,
    content: content.value,
    coverUrl: coverUrl.value,
    seoKeywords: seoKeywords.value,
    allowComments: allowComments.value,
    isPinned: isPinned.value,
    categoryId: categoryId.value,
    tagIds: [...tagIds.value].sort((left, right) => left - right),
    newPassword: newPassword.value,
    clearPassword: clearPassword.value,
  })
}

function resetEditor() {
  title.value = ''
  slug.value = ''
  summary.value = ''
  aiBrief.value = ''
  content.value = ''
  coverUrl.value = ''
  seoKeywords.value = []
  keywordInput.value = ''
  keywordError.value = ''
  allowComments.value = true
  isPinned.value = false
  categoryId.value = null
  tagIds.value = []
  passwordProtected.value = false
  newPassword.value = ''
  clearPassword.value = false
  passwordError.value = ''
  saveError.value = ''
  titleError.value = ''
  conflict.value = false
  discardConfirmOpen.value = false
  backupPrompt.value = null
  editorNotice.value = ''
  coverPickerOpen.value = false
  loadedArticle.value = null
  loadingArticle.value = false
  saveIntent.value = null
  taxonomyError.value = ''
  newCategoryName.value = ''
  newTagName.value = ''
  revisions.value = []
  revisionsError.value = ''
  expandedRevisionNo.value = null
  restoreConfirmOpen.value = false
  pendingRestore.value = null
  savedSnapshot.value = formSnapshot()
}

// 将服务器返回的文章整体写回表单，并把当前状态记为“已保存”基线。
function applyArticle(article: AdminArticle) {
  applyingServerState = true
  loadedArticle.value = article
  title.value = article.title
  slug.value = article.slug
  summary.value = article.summary
  aiBrief.value = article.ai_brief ?? ''
  content.value = article.markdown_source
  vditorInstance?.setValue(article.markdown_source)
  coverUrl.value = article.cover_url ?? ''
  seoKeywords.value = [...article.seo_keywords]
  allowComments.value = article.allow_comments
  isPinned.value = article.is_pinned
  categoryId.value = article.category_id ?? null
  tagIds.value = [...article.tag_ids]
  passwordProtected.value = article.password_protected
  newPassword.value = ''
  clearPassword.value = false
  passwordError.value = ''
  conflict.value = false
  savedSnapshot.value = formSnapshot()
  // watcher 在下一微任务才触发，需在 flush 后再解除标记，避免把服务器回写误判为用户输入。
  void nextTick(() => {
    applyingServerState = false
  })
}

async function loadTaxonomy() {
  taxonomyLoading.value = true
  taxonomyError.value = ''
  try {
    const [loadedCategories, loadedTags] = await Promise.all([
      articlesApi.listCategories(),
      articlesApi.listTags(),
    ])
    categories.value = loadedCategories
    tags.value = loadedTags
  } catch (requestError) {
    taxonomyError.value = getApiError(requestError, '分类和标签加载失败')
  } finally {
    taxonomyLoading.value = false
  }
}

async function loadArticle(articleId: number) {
  loadingArticle.value = true
  saveError.value = ''
  try {
    const article = await articlesApi.get(articleId)
    applyArticle(article)
    checkBackupPrompt(article)
    await loadRevisions(articleId)
  } catch (requestError) {
    saveError.value = getApiError(requestError, '文章加载失败')
  } finally {
    loadingArticle.value = false
  }
}

async function loadRevisions(articleId: number) {
  revisionsLoading.value = true
  revisionsError.value = ''
  try {
    revisions.value = await articlesApi.listRevisions(articleId)
  } catch (requestError) {
    revisionsError.value = getApiError(requestError, '历史版本加载失败')
  } finally {
    revisionsLoading.value = false
  }
}

// SEO Keywords：回车或逗号（含中文逗号）分隔提交，去重，上限与 Backend 校验保持一致。
function commitKeywordInput() {
  const raw = keywordInput.value.replace(/，/g, ',')
  keywordError.value = ''
  if (!raw.trim()) {
    keywordInput.value = ''
    return
  }
  for (const part of raw.split(',').map((value) => value.trim()).filter(Boolean)) {
    if (part.length > 50) {
      keywordError.value = '每个关键词不能超过 50 个字符'
      continue
    }
    if (seoKeywords.value.includes(part)) continue
    if (seoKeywords.value.length >= 20) {
      keywordError.value = '最多添加 20 个关键词'
      break
    }
    seoKeywords.value = [...seoKeywords.value, part]
  }
  keywordInput.value = ''
}

function handleKeywordKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter' || event.key === ',') {
    event.preventDefault()
    keywordError.value = ''
    commitKeywordInput()
  }
}

function removeKeyword(keyword: string) {
  keywordError.value = ''
  seoKeywords.value = seoKeywords.value.filter((value) => value !== keyword)
}

function buildPayload(): CreateArticlePayload {
  const payload: CreateArticlePayload = {
    title: title.value.trim(),
    summary: summary.value.trim(),
    markdown_source: content.value,
    category_id: categoryId.value,
    cover_url: coverUrl.value.trim() || null,
    seo_keywords: [...seoKeywords.value],
    allow_comments: allowComments.value,
    is_pinned: isPinned.value,
    tag_ids: [...tagIds.value],
    ai_brief: aiBrief.value.trim() || null,
  }
  // Slug 留空时：新建由 Backend 按标题生成，更新则保持原值，因此只在非空时发送。
  const trimmedSlug = slug.value.trim()
  if (trimmedSlug) payload.slug = trimmedSlug
  // 访问密码三态：输入了新密码优先设置，其次显式清除，否则不发送该字段。
  if (newPassword.value) payload.access_password = newPassword.value
  else if (clearPassword.value) payload.access_password = null
  return payload
}

async function persistArticle() {
  const existing = loadedArticle.value ?? props.article
  const payload = buildPayload()
  if (existing) {
    return articlesApi.update(existing.id, {
      ...payload,
      expected_version: existing.version,
    })
  }
  return articlesApi.create(payload)
}

async function createTaxonomy(kind: 'category' | 'tag') {
  const name = (kind === 'category' ? newCategoryName.value : newTagName.value).trim()
  if (!name || creatingTaxonomy.value) return
  creatingTaxonomy.value = kind
  taxonomyError.value = ''
  try {
    if (kind === 'category') {
      const category = await articlesApi.createCategory(name)
      categories.value = [...categories.value, category]
      categoryId.value = category.id
      newCategoryName.value = ''
    } else {
      const tag = await articlesApi.createTag(name)
      tags.value = [...tags.value, tag]
      tagIds.value = [...tagIds.value, tag.id]
      newTagName.value = ''
    }
  } catch (requestError) {
    taxonomyError.value = getApiError(requestError, 'Taxonomy 创建失败')
  } finally {
    creatingTaxonomy.value = null
  }
}

async function saveArticle(publishAfterSave: boolean) {
  commitKeywordInput()
  titleError.value = title.value.trim() ? '' : '请输入文章标题'
  passwordError.value =
    newPassword.value && newPassword.value.trim().length < 6 ? '访问密码至少需要 6 个字符' : ''
  saveError.value = ''
  if (titleError.value || passwordError.value || saving.value || !canSave.value) return

  // 手动保存与自动保存互斥：先取消挂起的自动保存，避免同一版本号被并发 PUT 触发 409。
  autosave.reset()
  const wasNew = currentArticleId.value === null
  saving.value = true
  saveIntent.value = publishAfterSave ? 'publish' : 'draft'
  let persisted: AdminArticle | null = null
  try {
    persisted = await persistArticle()
    applyArticle(persisted)
    // 保存成功后清理本地 emergency backup（新建文章对应 'new' 键）。
    clearBackup(window.localStorage, wasNew ? null : persisted.id)
    autosave.markSavedExternally()
    const result = publishAfterSave && persisted.status === 'draft'
      ? await articlesApi.changeStatus(persisted.id, 'publish', persisted.version)
      : persisted
    if (result.status !== persisted.status) loadedArticle.value = result
    void loadRevisions(persisted.id)
    emit('saved', result)
  } catch (requestError) {
    // 409 ARTICLE_CONFLICT 进入明确的冲突状态，由用户决定重新加载或保留本地副本，绝不静默覆盖。
    if (getApiErrorCode(requestError) === 'ARTICLE_CONFLICT') {
      conflict.value = true
    } else {
      const message = getApiError(requestError, publishAfterSave ? '文章发布失败' : '文章保存失败')
      saveError.value = publishAfterSave && persisted ? `草稿已保存，但发布失败：${message}` : message
    }
  } finally {
    saving.value = false
    saveIntent.value = null
  }
}

function reloadAfterConflict() {
  conflict.value = false
  autosave.reset()
  if (currentArticleId.value !== null) loadArticle(currentArticleId.value)
}

function toggleRevision(revisionNo: number) {
  expandedRevisionNo.value = expandedRevisionNo.value === revisionNo ? null : revisionNo
}

function requestRestore(revision: ArticleRevision) {
  revisionsError.value = ''
  pendingRestore.value = revision
  restoreConfirmOpen.value = true
}

async function confirmRestore() {
  const article = loadedArticle.value
  const revision = pendingRestore.value
  if (!article || !revision || restoring.value) return
  restoring.value = true
  try {
    const restored = await articlesApi.restoreRevision(article.id, revision.revision_no, article.version)
    applyArticle(restored)
    restoreConfirmOpen.value = false
    pendingRestore.value = null
    expandedRevisionNo.value = null
    await loadRevisions(restored.id)
  } catch (requestError) {
    if (getApiErrorCode(requestError) === 'ARTICLE_CONFLICT') {
      conflict.value = true
      restoreConfirmOpen.value = false
    } else {
      revisionsError.value = getApiError(requestError, '版本恢复失败')
    }
  } finally {
    restoring.value = false
  }
}

function handleOpenChange(open: boolean) {
  if (open) {
    emit('update:open', true)
    return
  }
  if (saving.value) return
  // 有未保存修改时先确认，避免误触遮罩或 Escape 丢失内容。
  if (isDirty.value) {
    discardConfirmOpen.value = true
    return
  }
  emit('update:open', false)
}

function confirmDiscard() {
  discardConfirmOpen.value = false
  autosave.reset()
  // 用户明确放弃修改时，对应的 emergency backup 一并清理，避免下次打开又提示恢复。
  clearBackup(window.localStorage, currentArticleId.value)
  emit('update:open', false)
}

function categoryName(id: number | null) {
  if (id === null) return '无分类'
  return categories.value.find((category) => category.id === id)?.name ?? `分类 #${id}`
}

function operatorText(operatorId: number) {
  if (operatorId === session.user?.id) return session.user.display_name
  return `User #${operatorId}`
}

function formatRevisionTime(value: string) {
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
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent
      class="grid h-[calc(100dvh-1rem)] w-[calc(100%-1rem)] max-w-[1360px]! grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden rounded-lg p-0 sm:h-[min(860px,calc(100dvh-2rem))] sm:max-w-[calc(100%-2rem)]! xl:max-w-[1360px]!"
    >
      <DialogHeader class="border-b px-4 py-4 pr-14 text-left sm:px-5">
        <DialogTitle class="text-base">{{ editing ? '编辑文章' : '写文章' }}</DialogTitle>
        <DialogDescription class="mt-0.5">{{ editing ? '修改后保存为当前版本。' : '保存后进入草稿列表，不会直接发布。' }}</DialogDescription>
      </DialogHeader>

      <div class="min-h-0 overflow-y-auto lg:grid lg:grid-cols-[minmax(0,1fr)_300px] lg:overflow-hidden">
        <section class="min-w-0 space-y-4 p-4 sm:p-5 lg:flex lg:min-h-0 lg:flex-col" aria-label="文章正文">
          <div class="grid gap-2">
            <label for="dialog-article-title" class="text-sm font-medium leading-none">标题</label>
            <Input
              id="dialog-article-title"
              v-model="title"
              class="h-9 text-sm"
              placeholder="输入文章标题"
              :aria-invalid="Boolean(titleError)"
              aria-describedby="dialog-article-title-message"
              @update:model-value="titleError = ''"
            />
            <p
              v-if="titleError"
              id="dialog-article-title-message"
              role="alert"
              class="flex items-center gap-1.5 text-xs font-medium text-destructive"
            >
              <CircleAlertIcon class="size-3.5" />
              {{ titleError }}
            </p>
          </div>

          <div class="min-w-0 lg:flex lg:min-h-0 lg:flex-1 lg:flex-col">
            <div class="flex min-h-9 flex-wrap items-center justify-between gap-x-3 gap-y-1 border-b pb-1.5">
              <span v-if="loadingArticle" class="flex items-center gap-1.5 text-xs text-muted-foreground">
                <LoaderCircleIcon class="size-3.5 animate-spin" />
                正在加载
              </span>
              <span
                v-else-if="autosaveStatusText"
                class="flex items-center gap-1.5 text-xs"
                :class="autosaveStatusTone"
                role="status"
              >
                <LoaderCircleIcon v-if="autosave.phase.value === 'saving'" class="size-3.5 animate-spin" />
                {{ autosaveStatusText }}
              </span>
              <span v-else class="text-xs text-muted-foreground">正文支持 Markdown，可直接粘贴或拖拽图片上传</span>
              <span class="flex items-center gap-2">
                <DropdownMenu>
                  <DropdownMenuTrigger as-child>
                    <Button variant="outline" size="sm" class="h-7 gap-1 px-2 text-xs" :disabled="loadingArticle || saving">
                      <SparklesIcon class="size-3.5" />
                      AI 助手
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" class="w-44">
                    <DropdownMenuItem @click="openAiAssist('rewrite')">
                      改写选中文本
                    </DropdownMenuItem>
                    <DropdownMenuItem @click="openAiAssist('summary')">
                      生成摘要
                    </DropdownMenuItem>
                    <DropdownMenuItem @click="openAiAssist('metadata')">
                      SEO 建议
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
                <span class="text-xs tabular-nums text-muted-foreground">{{ characterCount }} 字符</span>
              </span>
            </div>
            <p v-if="editorNotice" class="mt-1.5 text-xs text-muted-foreground">{{ editorNotice }}</p>
            <!-- Vditor 即时渲染模式自带预览与计数器；cache 关闭，草稿持久化由自动保存与 localStorage 备份负责。 -->
            <div ref="vditorEl" class="mt-2 min-h-0 w-full lg:flex-1" aria-label="文章正文" />
          </div>
        </section>

        <aside class="space-y-5 border-t bg-muted/15 p-4 sm:p-5 lg:min-h-0 lg:overflow-y-auto lg:border-l lg:border-t-0" aria-label="文章设置">
          <div
            v-if="backupPrompt"
            role="alert"
            class="space-y-2 rounded-lg border border-amber-500/50 bg-amber-500/10 p-3 text-xs"
          >
            <p class="flex items-center gap-1.5 font-medium text-amber-600 dark:text-amber-500">
              <TriangleAlertIcon class="size-3.5" />
              检测到本地未保存的备份
            </p>
            <p class="leading-5 text-muted-foreground">
              备份时间 {{ formatRevisionTime(backupPrompt.saved_at) }}，比服务器上的内容更新。可以恢复该备份继续编辑，或丢弃备份使用服务器版本。
            </p>
            <div class="flex gap-2">
              <Button variant="outline" size="sm" @click="restoreBackup">恢复本地备份</Button>
              <Button variant="ghost" size="sm" @click="discardBackup">丢弃备份</Button>
            </div>
          </div>
          <div>
            <h2 class="text-sm font-semibold">文章设置</h2>
          </div>
          <div class="space-y-1.5">
            <label for="dialog-article-slug" class="text-xs font-medium">Slug</label>
            <Input
              id="dialog-article-slug"
              v-model="slug"
              class="h-9 bg-background text-sm"
              :placeholder="editing ? '留空保持当前 Slug' : '留空按标题自动生成'"
            />
            <p class="text-xs text-muted-foreground">
              {{ slug.trim() ? `将保存为：${slug.trim()}` : editing ? '留空则不修改当前 Slug。' : '保存时按标题自动生成。' }}
            </p>
          </div>
          <div class="space-y-1.5">
            <span class="text-xs font-medium">封面</span>
            <div v-if="coverUrl" class="overflow-hidden rounded-md border bg-muted">
              <img :src="coverUrl" alt="封面预览" class="h-28 w-full object-cover" />
            </div>
            <p v-else class="rounded-md border border-dashed px-3 py-4 text-center text-xs text-muted-foreground">
              暂未设置封面
            </p>
            <div class="flex gap-2">
              <Button type="button" variant="outline" size="sm" :disabled="saving" @click="coverPickerOpen = true">
                从媒体库选择
              </Button>
              <Button v-if="coverUrl" type="button" variant="ghost" size="sm" :disabled="saving" @click="coverUrl = ''">
                清除封面
              </Button>
            </div>
          </div>
          <div class="space-y-1.5">
            <label for="dialog-article-keywords" class="text-xs font-medium">SEO 关键词</label>
            <div v-if="seoKeywords.length" class="flex flex-wrap gap-1.5">
              <span
                v-for="keyword in seoKeywords"
                :key="keyword"
                class="inline-flex items-center gap-1 rounded-md border bg-background px-2 py-0.5 text-xs"
              >
                {{ keyword }}
                <button
                  type="button"
                  class="text-muted-foreground hover:text-foreground"
                  :aria-label="`移除关键词 ${keyword}`"
                  :disabled="saving"
                  @click="removeKeyword(keyword)"
                >
                  <XIcon class="size-3" />
                </button>
              </span>
            </div>
            <Input
              id="dialog-article-keywords"
              v-model="keywordInput"
              class="h-9 bg-background text-sm"
              placeholder="回车或逗号分隔，最多 20 个"
              :disabled="saving"
              @keydown="handleKeywordKeydown"
              @blur="commitKeywordInput"
            />
            <p v-if="keywordError" role="alert" class="text-xs font-medium text-destructive">{{ keywordError }}</p>
          </div>
          <div class="space-y-1.5">
            <label for="dialog-article-summary" class="text-xs font-medium">摘要</label>
            <Textarea id="dialog-article-summary" v-model="summary" class="min-h-28 resize-y bg-background" placeholder="简要说明文章内容" />
          </div>
          <div class="space-y-1.5">
            <div class="flex items-center justify-between gap-2">
              <label for="dialog-article-ai-brief" class="text-xs font-medium">导读</label>
              <Button
                type="button"
                variant="outline"
                size="sm"
                class="h-7 gap-1 px-2 text-xs"
                :disabled="loadingArticle || saving"
                @click="openAiAssist('brief')"
              >
                <SparklesIcon class="size-3.5" />
                AI 生成
              </Button>
            </div>
            <Textarea
              id="dialog-article-ai-brief"
              v-model="aiBrief"
              class="min-h-24 resize-y bg-background"
              placeholder="150 字以内的 AI 导读，生成后可直接编辑"
            />
            <p class="text-xs text-muted-foreground">导读用于文章页头部展示，留空则不显示。</p>
          </div>
          <div class="space-y-1.5">
            <label for="dialog-article-category" class="text-xs font-medium">分类</label>
            <select
              id="dialog-article-category"
              v-model="categoryId"
              class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
              :disabled="taxonomyLoading || saving"
            >
              <option :value="null">未选择分类</option>
              <option v-for="category in categories" :key="category.id" :value="category.id">{{ category.name }}</option>
            </select>
            <div class="flex gap-2">
              <Input v-model="newCategoryName" class="h-8 bg-background text-xs" placeholder="快速新建分类" @keydown.enter.prevent="createTaxonomy('category')" />
              <Button type="button" size="icon-sm" variant="outline" :disabled="!newCategoryName.trim() || creatingTaxonomy !== null" aria-label="新建分类" @click="createTaxonomy('category')">
                <LoaderCircleIcon v-if="creatingTaxonomy === 'category'" class="animate-spin" />
                <PlusIcon v-else />
              </Button>
            </div>
          </div>
          <div class="space-y-1.5">
            <div class="flex items-center justify-between gap-2">
              <span class="text-xs font-medium">标签</span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                class="h-7 gap-1 px-2 text-xs"
                :disabled="loadingArticle || saving"
                @click="openAiAssist('tags')"
              >
                <SparklesIcon class="size-3.5" />
                AI 推荐
              </Button>
            </div>
            <div v-if="tags.length" class="flex flex-wrap gap-2">
              <label v-for="tag in tags" :key="tag.id" class="inline-flex cursor-pointer items-center gap-1.5 rounded-md border bg-background px-2 py-1 text-xs has-[:checked]:border-primary has-[:checked]:bg-primary/10">
                <input v-model="tagIds" type="checkbox" class="accent-primary" :value="tag.id" :disabled="saving" />
                {{ tag.name }}
              </label>
            </div>
            <p v-else class="text-xs text-muted-foreground">暂无标签</p>
            <div class="flex gap-2">
              <Input v-model="newTagName" class="h-8 bg-background text-xs" placeholder="快速新建标签" @keydown.enter.prevent="createTaxonomy('tag')" />
              <Button type="button" size="icon-sm" variant="outline" :disabled="!newTagName.trim() || creatingTaxonomy !== null" aria-label="新建标签" @click="createTaxonomy('tag')">
                <LoaderCircleIcon v-if="creatingTaxonomy === 'tag'" class="animate-spin" />
                <PlusIcon v-else />
              </Button>
            </div>
          </div>
          <!-- 无 Switch 组件，用原生 Checkbox 加 peer 样式实现开关，行为与原生控件一致。 -->
          <div class="space-y-2">
            <label class="flex cursor-pointer items-center justify-between gap-3 text-xs font-medium">
              允许评论
              <input v-model="allowComments" type="checkbox" class="peer sr-only" :disabled="saving" />
              <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
            </label>
            <label class="flex cursor-pointer items-center justify-between gap-3 text-xs font-medium">
              置顶文章
              <input v-model="isPinned" type="checkbox" class="peer sr-only" :disabled="saving" />
              <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
            </label>
          </div>
          <div class="space-y-1.5">
            <span class="flex items-center gap-1.5 text-xs font-medium">
              <LockIcon class="size-3.5 text-muted-foreground" />
              访问密码
            </span>
            <p class="text-xs text-muted-foreground">
              {{ passwordProtected ? '已设置访问密码，原文不会回显。' : '未设置访问密码。' }}
            </p>
            <Input
              id="dialog-article-password"
              v-model="newPassword"
              type="password"
              class="h-9 bg-background text-sm"
              placeholder="输入新密码（至少 6 个字符）"
              autocomplete="new-password"
              :disabled="saving"
              @update:model-value="passwordError = ''; clearPassword = false"
            />
            <p v-if="passwordError" role="alert" class="text-xs font-medium text-destructive">{{ passwordError }}</p>
            <div v-if="passwordProtected && !newPassword" class="space-y-1.5">
              <Button
                v-if="!clearPassword"
                type="button"
                variant="outline"
                size="sm"
                :disabled="saving"
                @click="clearPassword = true"
              >
                清除访问密码
              </Button>
              <p v-else class="flex items-center justify-between gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-2.5 py-1.5 text-xs text-destructive">
                保存后将清除访问密码
                <button type="button" class="font-medium underline" :disabled="saving" @click="clearPassword = false">撤销</button>
              </p>
            </div>
          </div>
          <p v-if="taxonomyError" role="alert" class="text-xs font-medium leading-5 text-destructive">{{ taxonomyError }}</p>
          <dl class="divide-y overflow-hidden rounded-lg border bg-background text-xs">
            <div class="flex items-center justify-between gap-3 px-3 py-2.5">
              <dt class="text-muted-foreground">状态</dt>
              <dd><Badge :variant="currentStatus === 'published' ? 'default' : 'secondary'">{{ currentStatusText }}</Badge></dd>
            </div>
            <div class="flex items-center justify-between gap-3 px-3 py-2.5">
              <dt class="text-muted-foreground">可见性</dt>
              <dd class="font-medium">{{ currentStatus === 'published' ? '公开可见' : '后台可见' }}</dd>
            </div>
          </dl>

          <section v-if="editing" class="space-y-2" aria-label="历史版本">
            <h3 class="flex items-center gap-1.5 text-xs font-medium">
              <HistoryIcon class="size-3.5 text-muted-foreground" />
              历史版本
            </h3>
            <p v-if="revisionsLoading" class="flex items-center gap-1.5 text-xs text-muted-foreground">
              <LoaderCircleIcon class="size-3.5 animate-spin" />
              正在加载历史版本
            </p>
            <div v-else-if="revisionsError" class="space-y-1.5">
              <p role="alert" class="text-xs font-medium text-destructive">{{ revisionsError }}</p>
              <Button variant="outline" size="sm" :disabled="currentArticleId === null" @click="currentArticleId !== null && loadRevisions(currentArticleId)">重试</Button>
            </div>
            <p v-else-if="revisions.length === 0" class="text-xs text-muted-foreground">暂无历史版本，保存或恢复后会自动留档。</p>
            <div v-else class="space-y-1.5">
              <div v-for="revision in revisions" :key="revision.revision_no" class="overflow-hidden rounded-md border bg-background">
                <button
                  type="button"
                  class="flex w-full items-center gap-2 px-2.5 py-2 text-left"
                  :aria-expanded="expandedRevisionNo === revision.revision_no"
                  @click="toggleRevision(revision.revision_no)"
                >
                  <Badge variant="secondary" class="shrink-0">v{{ revision.revision_no }}</Badge>
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-xs font-medium">{{ revision.title || '（无标题）' }}</span>
                    <span class="block text-[11px] text-muted-foreground">
                      {{ formatRevisionTime(revision.created_at) }} · {{ operatorText(revision.operator_id) }}
                    </span>
                  </span>
                  <ChevronDownIcon
                    class="size-3.5 shrink-0 text-muted-foreground transition-transform"
                    :class="expandedRevisionNo === revision.revision_no ? 'rotate-180' : ''"
                  />
                </button>
                <div v-if="expandedRevisionNo === revision.revision_no" class="space-y-2 border-t px-2.5 py-2">
                  <dl class="space-y-1 text-[11px] leading-5">
                    <div class="flex gap-2">
                      <dt class="shrink-0 text-muted-foreground">Slug</dt>
                      <dd class="min-w-0 truncate">{{ revision.slug }}</dd>
                    </div>
                    <div class="flex gap-2">
                      <dt class="shrink-0 text-muted-foreground">分类</dt>
                      <dd class="min-w-0 truncate">{{ categoryName(revision.category_id ?? null) }}</dd>
                    </div>
                    <div v-if="revision.seo_keywords.length" class="flex gap-2">
                      <dt class="shrink-0 text-muted-foreground">关键词</dt>
                      <dd class="min-w-0">{{ revision.seo_keywords.join('、') }}</dd>
                    </div>
                    <div v-if="revision.cover_url" class="flex gap-2">
                      <dt class="shrink-0 text-muted-foreground">封面</dt>
                      <dd class="min-w-0 truncate">{{ revision.cover_url }}</dd>
                    </div>
                    <div class="flex flex-wrap gap-1 pt-0.5">
                      <Badge v-if="revision.password_protected" variant="secondary">访问密码</Badge>
                      <Badge v-if="revision.is_pinned" variant="secondary">置顶</Badge>
                      <Badge v-if="!revision.allow_comments" variant="secondary">关闭评论</Badge>
                    </div>
                  </dl>
                  <pre class="max-h-40 overflow-auto whitespace-pre-wrap rounded bg-muted/50 p-2 font-mono text-[11px] leading-5">{{ revision.markdown_source }}</pre>
                  <Button variant="outline" size="sm" :disabled="restoring || saving" @click="requestRestore(revision)">
                    <RotateCcwIcon />
                    恢复此版本
                  </Button>
                </div>
              </div>
            </div>
          </section>

          <div v-if="conflict" role="alert" class="space-y-2 rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-xs">
            <p class="flex items-center gap-1.5 font-medium text-destructive">
              <TriangleAlertIcon class="size-3.5" />
              服务器版本已变更
            </p>
            <p class="leading-5 text-muted-foreground">
              文章在其他地方被修改过，当前内容基于旧版本，无法直接保存。可以重新加载服务器最新内容（放弃本地修改），或保留本地副本后手动合并。
            </p>
            <div class="flex gap-2">
              <Button variant="outline" size="sm" :disabled="loadingArticle" @click="reloadAfterConflict">
                <LoaderCircleIcon v-if="loadingArticle" class="animate-spin" />
                重新加载
              </Button>
              <Button variant="ghost" size="sm" @click="conflict = false">保留本地副本</Button>
            </div>
          </div>
          <p v-if="saveError" role="alert" class="flex items-start gap-1.5 text-xs font-medium leading-5 text-destructive">
            <CircleAlertIcon class="mt-0.5 size-3.5 shrink-0" />
            {{ saveError }}
          </p>
        </aside>
      </div>

      <DialogFooter class="flex-row items-center justify-end border-t bg-background px-4 py-3 sm:px-5">
        <DialogClose as-child>
          <Button variant="outline" :disabled="saving">关闭</Button>
        </DialogClose>
        <Button
          v-if="currentStatus === 'draft'"
          variant="outline"
          :disabled="saving || !canSave"
          @click="saveArticle(false)"
        >
          <LoaderCircleIcon v-if="saveIntent === 'draft'" class="animate-spin" data-icon="inline-start" />
          <SaveIcon v-else data-icon="inline-start" />
          {{ saveIntent === 'draft' ? '正在保存' : '保存草稿' }}
        </Button>
        <Button
          v-if="currentStatus === 'draft'"
          :disabled="saving || !canSave"
          @click="saveArticle(true)"
        >
          <LoaderCircleIcon v-if="saveIntent === 'publish'" class="animate-spin" data-icon="inline-start" />
          <SendIcon v-else data-icon="inline-start" />
          {{ saveIntent === 'publish' ? '正在发布' : '发布' }}
        </Button>
        <Button v-else :disabled="saving || !canSave" @click="saveArticle(false)">
          <LoaderCircleIcon v-if="saveIntent === 'draft'" class="animate-spin" data-icon="inline-start" />
          <SaveIcon v-else data-icon="inline-start" />
          {{ saving ? '正在保存' : '保存修改' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>

  <AppConfirmDialog
    v-model:open="discardConfirmOpen"
    title="放弃未保存的修改？"
    description="当前编辑内容尚未保存，关闭后将丢失。"
    confirm-label="放弃修改"
    destructive
    @confirm="confirmDiscard"
  />
  <AppConfirmDialog
    v-model:open="restoreConfirmOpen"
    :title="`恢复到版本 v${pendingRestore?.revision_no ?? ''}？`"
    description="编辑器内容将被该版本覆盖，当前版本会先自动留档为新的历史版本。"
    confirm-label="恢复此版本"
    :busy="restoring"
    @confirm="confirmRestore"
  />
  <AppMediaPicker v-model:open="coverPickerOpen" @select="handleCoverSelect" />
  <ArticleAiAssistDialog
    v-model:open="aiDialogOpen"
    :feature="aiFeature"
    :request="aiRequest"
    @apply="handleAiApply"
  />
</template>
