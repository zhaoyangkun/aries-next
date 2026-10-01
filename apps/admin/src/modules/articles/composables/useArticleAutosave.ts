import { ref, type Ref } from 'vue'
import axios from 'axios'
import { getApiError, getApiErrorCode } from '@/shared/api/client'
import type { AdminArticle, CreateArticlePayload } from '../api/articles'

// 自动保存状态机：idle → pending → saving → saved / offline / conflict / failed；
// 新建文章（无 ID）无法走服务端保存，只写 localStorage emergency backup（backed_up）。
export type AutosavePhase =
  | 'idle'
  | 'pending'
  | 'saving'
  | 'saved'
  | 'offline'
  | 'conflict'
  | 'failed'
  | 'backed_up'

export interface ArticleBackup {
  saved_at: string
  // 备份时服务端版本的时间基线，用于重开编辑器时判断备份是否比服务器新。
  base_updated_at: string | null
  payload: CreateArticlePayload
}

export function backupStorageKey(articleId: number | null) {
  return `aries:article-autosave:${articleId ?? 'new'}`
}

export function readBackup(storage: Storage, articleId: number | null): ArticleBackup | null {
  try {
    const raw = storage.getItem(backupStorageKey(articleId))
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<ArticleBackup>
    if (typeof parsed.saved_at !== 'string' || typeof parsed.payload !== 'object' || !parsed.payload) {
      return null
    }
    return {
      saved_at: parsed.saved_at,
      base_updated_at: typeof parsed.base_updated_at === 'string' ? parsed.base_updated_at : null,
      payload: parsed.payload as CreateArticlePayload,
    }
  } catch {
    // 备份损坏时静默忽略，绝不阻塞编辑器打开。
    return null
  }
}

export function writeBackup(storage: Storage, articleId: number | null, backup: ArticleBackup) {
  try {
    storage.setItem(backupStorageKey(articleId), JSON.stringify(backup))
  } catch {
    // localStorage 可能已满或被禁用，备份失败不应打断编辑。
  }
}

export function clearBackup(storage: Storage, articleId: number | null) {
  try {
    storage.removeItem(backupStorageKey(articleId))
  } catch {
    // 同上，清理失败忽略。
  }
}

// 备份时间晚于服务器 updated_at 才提示恢复；时间无法解析时保守视为“可能更新”，由用户决定。
export function isBackupNewer(backup: ArticleBackup, serverUpdatedAt: string | null) {
  const backupTime = Date.parse(backup.saved_at)
  const serverTime = serverUpdatedAt ? Date.parse(serverUpdatedAt) : Number.NaN
  if (Number.isNaN(backupTime)) return false
  if (Number.isNaN(serverTime)) return true
  return backupTime > serverTime
}

// 无 response 的 Axios 错误视为网络故障（离线/断网），进入 Offline 状态。
export function isNetworkError(error: unknown) {
  return axios.isAxiosError(error) && !error.response
}

export interface AutosaveSnapshot {
  payload: CreateArticlePayload
  // 发送时点的表单快照串，保存成功后作为“已保存”基线，避免覆盖保存期间的新输入。
  snapshot: string
}

export interface UseArticleAutosaveOptions {
  articleId: () => number | null
  baseUpdatedAt: () => string | null
  buildSnapshot: () => AutosaveSnapshot
  // 保存既有文章（内部带 expected_version 乐观锁）；可注入 mock 便于测试。
  save: (articleId: number, payload: CreateArticlePayload) => Promise<AdminArticle>
  onSaved?: (article: AdminArticle, sent: AutosaveSnapshot) => void
  onConflict?: () => void
  debounceMs?: number
  storage?: Storage
  setTimeoutFn?: typeof setTimeout
  clearTimeoutFn?: typeof clearTimeout
  now?: () => Date
}

export interface ArticleAutosave {
  phase: Ref<AutosavePhase>
  lastError: Ref<string>
  notifyChange: () => void
  flush: () => Promise<void>
  markSavedExternally: () => void
  reset: () => void
  dispose: () => void
}

export function useArticleAutosave(options: UseArticleAutosaveOptions): ArticleAutosave {
  const debounceMs = options.debounceMs ?? 30_000
  const storage = options.storage ?? window.localStorage
  const setTimeoutFn = options.setTimeoutFn ?? setTimeout
  const clearTimeoutFn = options.clearTimeoutFn ?? clearTimeout
  const now = options.now ?? (() => new Date())

  const phase: Ref<AutosavePhase> = ref('idle')
  const lastError = ref('')
  let timer: ReturnType<typeof setTimeout> | undefined
  let pending: AutosaveSnapshot | null = null

  function cancelTimer() {
    if (timer !== undefined) clearTimeoutFn(timer)
    timer = undefined
  }

  function persistBackup(articleId: number | null, snapshot: AutosaveSnapshot) {
    writeBackup(storage, articleId, {
      saved_at: now().toISOString(),
      base_updated_at: options.baseUpdatedAt(),
      payload: snapshot.payload,
    })
  }

  // 内容变化时调用：既有文章防抖后自动 PUT；新文章只写 emergency backup。
  function notifyChange() {
    const snapshot = options.buildSnapshot()
    pending = snapshot
    const articleId = options.articleId()
    if (articleId === null) {
      persistBackup(null, snapshot)
      phase.value = 'backed_up'
      return
    }
    phase.value = 'pending'
    cancelTimer()
    timer = setTimeoutFn(() => void flush(), debounceMs)
  }

  async function flush() {
    cancelTimer()
    const articleId = options.articleId()
    const snapshot = pending
    if (articleId === null || !snapshot || phase.value === 'saving') return
    // 标题为空时 Backend 会拒绝整篇保存，退化为只写本地备份。
    if (!snapshot.payload.title.trim()) {
      persistBackup(articleId, snapshot)
      phase.value = 'backed_up'
      return
    }

    phase.value = 'saving'
    try {
      const saved = await options.save(articleId, snapshot.payload)
      options.onSaved?.(saved, snapshot)
      clearBackup(storage, articleId)
      if (pending !== snapshot) {
        // 保存期间又有新输入，重新调度一轮而不是标记 saved。
        phase.value = 'pending'
        timer = setTimeoutFn(() => void flush(), debounceMs)
      } else {
        pending = null
        phase.value = 'saved'
      }
    } catch (error) {
      if (getApiErrorCode(error) === 'ARTICLE_CONFLICT') {
        // 409 版本冲突：绝不自动覆盖，交给用户选择重新加载或保留本地副本。
        phase.value = 'conflict'
        options.onConflict?.()
      } else if (isNetworkError(error)) {
        persistBackup(articleId, snapshot)
        phase.value = 'offline'
        lastError.value = ''
      } else {
        phase.value = 'failed'
        lastError.value = getApiError(error, '自动保存失败')
      }
    }
  }

  // 手动保存成功后调用：取消挂起的自动保存并回到 saved 基线。
  function markSavedExternally() {
    cancelTimer()
    pending = null
    phase.value = 'saved'
    lastError.value = ''
  }

  function reset() {
    cancelTimer()
    pending = null
    phase.value = 'idle'
    lastError.value = ''
  }

  function dispose() {
    cancelTimer()
  }

  return { phase, lastError, notifyChange, flush, markSavedExternally, reset, dispose }
}
