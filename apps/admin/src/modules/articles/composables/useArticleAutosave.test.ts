import { describe, expect, it, vi } from 'vitest'
import type { AdminArticle, CreateArticlePayload } from '../api/articles'
import {
  backupStorageKey,
  isBackupNewer,
  isNetworkError,
  readBackup,
  useArticleAutosave,
  type ArticleBackup,
} from './useArticleAutosave'

const article: AdminArticle = {
  id: 1,
  author_id: 1,
  category_id: null,
  status: 'draft',
  slug: 'first-post',
  title: 'First post',
  summary: '',
  cover_url: null,
  markdown_source: '# First post',
  rendered_html: '<h1>First post</h1>',
  seo_keywords: [],
  tag_ids: [],
  password_protected: false,
  allow_comments: true,
  is_pinned: false,
  version: 1,
  published_at: null,
  created_at: '2026-08-05T12:00:00Z',
  updated_at: '2026-08-05T12:00:00Z',
}

function buildPayload(title = 'First post'): CreateArticlePayload {
  return { title, summary: '', markdown_source: '# content' }
}

// node 环境没有 localStorage，用内存实现注入。
function createMemoryStorage(): Storage {
  const map = new Map<string, string>()
  return {
    get length() {
      return map.size
    },
    clear: () => map.clear(),
    getItem: (key: string) => map.get(key) ?? null,
    key: (index: number) => [...map.keys()][index] ?? null,
    removeItem: (key: string) => {
      map.delete(key)
    },
    setItem: (key: string, value: string) => {
      map.set(key, value)
    },
  }
}

// 手动定时器：捕获回调而不是真实等待，测试可精确推进。
function createManualTimers() {
  const callbacks: Array<() => void> = []
  return {
    callbacks,
    setTimeoutFn: ((fn: () => void) => {
      callbacks.push(fn)
      return callbacks.length
    }) as unknown as typeof setTimeout,
    clearTimeoutFn: (() => undefined) as unknown as typeof clearTimeout,
    async runLatest() {
      const fn = callbacks.pop()
      fn?.()
      // 等待 flush 内部的 Promise 链 settle。
      await new Promise((resolve) => setTimeout(resolve, 0))
    },
  }
}

function networkError() {
  return { isAxiosError: true }
}

function conflictError() {
  return {
    isAxiosError: true,
    response: { status: 409, data: { error: { code: 'ARTICLE_CONFLICT', message: 'conflict' } } },
  }
}

function serverError() {
  return {
    isAxiosError: true,
    response: { status: 500, data: { error: { code: 'INTERNAL', message: '服务器错误' } } },
  }
}

function createAutosave(overrides: {
  articleId?: number | null
  save?: (articleId: number, payload: CreateArticlePayload) => Promise<AdminArticle>
  onSaved?: (savedArticle: AdminArticle) => void
  onConflict?: () => void
  storage?: Storage
  payload?: CreateArticlePayload
}) {
  const timers = createManualTimers()
  const storage = overrides.storage ?? createMemoryStorage()
  const payload = overrides.payload ?? buildPayload()
  const autosave = useArticleAutosave({
    articleId: () => overrides.articleId === undefined ? 1 : overrides.articleId,
    baseUpdatedAt: () => article.updated_at,
    buildSnapshot: () => ({ payload, snapshot: JSON.stringify(payload) }),
    save: overrides.save ?? vi.fn(async () => ({ ...article, version: 2 })),
    onSaved: overrides.onSaved,
    onConflict: overrides.onConflict,
    debounceMs: 30_000,
    storage,
    setTimeoutFn: timers.setTimeoutFn,
    clearTimeoutFn: timers.clearTimeoutFn,
  })
  return { autosave, timers, storage }
}

describe('useArticleAutosave', () => {
  it('saves an existing article after the debounce and reports saved', async () => {
    const save = vi.fn(async () => ({ ...article, version: 2 }))
    const onSaved = vi.fn()
    const { autosave, timers, storage } = createAutosave({ save, onSaved })

    autosave.notifyChange()
    expect(autosave.phase.value).toBe('pending')
    expect(save).not.toHaveBeenCalled()

    await timers.runLatest()
    expect(save).toHaveBeenCalledTimes(1)
    expect(autosave.phase.value).toBe('saved')
    expect(onSaved).toHaveBeenCalledTimes(1)
    // 保存成功后清理 emergency backup。
    expect(readBackup(storage, 1)).toBeNull()
  })

  it('writes an emergency backup instead of saving for a brand-new article', async () => {
    const save = vi.fn(async () => article)
    const { autosave, storage } = createAutosave({ articleId: null, save })

    autosave.notifyChange()
    expect(autosave.phase.value).toBe('backed_up')
    expect(save).not.toHaveBeenCalled()
    expect(readBackup(storage, null)?.payload).toEqual(buildPayload())
  })

  it('falls back to a local backup when the title is empty', async () => {
    const save = vi.fn(async () => article)
    const { autosave, timers, storage } = createAutosave({ save, payload: buildPayload('  ') })

    autosave.notifyChange()
    await timers.runLatest()
    expect(save).not.toHaveBeenCalled()
    expect(autosave.phase.value).toBe('backed_up')
    expect(readBackup(storage, 1)).not.toBeNull()
  })

  it('goes offline on network failure and persists a backup for later retry', async () => {
    const save = vi.fn().mockRejectedValueOnce(networkError()).mockResolvedValue({ ...article, version: 2 })
    const { autosave, timers, storage } = createAutosave({ save })

    autosave.notifyChange()
    await timers.runLatest()
    expect(autosave.phase.value).toBe('offline')
    const backup = readBackup(storage, 1)
    expect(backup?.payload).toEqual(buildPayload())
    expect(backup?.base_updated_at).toBe(article.updated_at)

    // 网络恢复后 flush 续存成功，备份被清理。
    await autosave.flush()
    expect(autosave.phase.value).toBe('saved')
    expect(save).toHaveBeenCalledTimes(2)
    expect(readBackup(storage, 1)).toBeNull()
  })

  it('enters conflict state on a 409 version conflict without overwriting', async () => {
    const save = vi.fn().mockRejectedValue(conflictError())
    const onConflict = vi.fn()
    const { autosave, timers, storage } = createAutosave({ save, onConflict })

    autosave.notifyChange()
    await timers.runLatest()
    expect(autosave.phase.value).toBe('conflict')
    expect(onConflict).toHaveBeenCalledTimes(1)
    // 冲突时不写备份也不重试，交给用户决策。
    expect(readBackup(storage, 1)).toBeNull()
  })

  it('reports failed for non-network server errors', async () => {
    const save = vi.fn().mockRejectedValue(serverError())
    const { autosave, timers } = createAutosave({ save })

    autosave.notifyChange()
    await timers.runLatest()
    expect(autosave.phase.value).toBe('failed')
    expect(autosave.lastError.value).toBe('服务器错误')
  })

  it('cancels a pending autosave when a manual save succeeds', async () => {
    const save = vi.fn(async () => article)
    const { autosave, timers } = createAutosave({ save })

    autosave.notifyChange()
    autosave.markSavedExternally()
    expect(autosave.phase.value).toBe('saved')
    await timers.runLatest()
    expect(save).not.toHaveBeenCalled()
  })
})

describe('backup helpers', () => {
  it('round-trips backups through storage and tolerates corrupted payloads', () => {
    const storage = createMemoryStorage()
    expect(readBackup(storage, 42)).toBeNull()

    const backup: ArticleBackup = {
      saved_at: '2026-08-06T00:00:00Z',
      base_updated_at: article.updated_at,
      payload: buildPayload(),
    }
    storage.setItem(backupStorageKey(42), JSON.stringify(backup))
    expect(readBackup(storage, 42)).toEqual(backup)

    storage.setItem(backupStorageKey(42), '{broken json')
    expect(readBackup(storage, 42)).toBeNull()
  })

  it('compares backup time with the server updated_at baseline', () => {
    const backup: ArticleBackup = {
      saved_at: '2026-08-06T00:00:00Z',
      base_updated_at: null,
      payload: buildPayload(),
    }
    expect(isBackupNewer(backup, '2026-08-05T12:00:00Z')).toBe(true)
    expect(isBackupNewer(backup, '2026-08-07T00:00:00Z')).toBe(false)
    // 服务器时间缺失时保守视为备份可能更新，交给用户选择。
    expect(isBackupNewer(backup, null)).toBe(true)
  })
})

describe('isNetworkError', () => {
  it('treats axios errors without a response as network failures', () => {
    expect(isNetworkError(networkError())).toBe(true)
    expect(isNetworkError(serverError())).toBe(false)
    expect(isNetworkError(new Error('boom'))).toBe(false)
  })
})
