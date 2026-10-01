import type {
  CommitImportRequest,
  CommitImportResult,
  ImportJobResponse,
  ImportPreviewItem,
  MediaAssetResponse,
  MediaBatchDeleteResult,
  MediaPageResponse,
  MediaUsageResponse,
  UpdateMediaRequest,
} from '@aries/api-client'
import axios from 'axios'
import { api, type ApiErrorResponse } from '@/shared/api/client'

// Response DTO 直接派生自 @aries/api-client（由 docs/openapi.yaml 生成），避免与 Backend 漂移。
export type {
  CommitImportResult,
  ImportPreviewItem,
  MediaBatchDeleteResult,
}
export type MediaProvider = MediaAssetResponse['provider']
export type MediaAssetStatus = MediaAssetResponse['status']
export type MediaUsageTargetType = MediaUsageResponse['target_type']
export type MediaAsset = MediaAssetResponse
export type MediaPage = MediaPageResponse
export type MediaUsage = MediaUsageResponse
export type ImportJob = ImportJobResponse
export type UpdateMediaPayload = UpdateMediaRequest

/** 待迁：与 CommitImportRequest.items 元素一致，这里保留手写名以保持调用方可读。 */
export type ImportStrategy = NonNullable<CommitImportRequest['items']>[number]['strategy']
export interface CommitImportItem {
  index: number
  strategy: ImportStrategy
}

// 与 Backend 上传校验保持一致：扩展名白名单、单文件 ≤ 5MB、每批 1–5 个。
export const MEDIA_UPLOAD_ACCEPT = '.jpg,.jpeg,.png,.gif,.bmp,.webp'
export const MEDIA_MAX_FILE_SIZE = 5 * 1024 * 1024
export const MEDIA_MAX_BATCH = 5

// Markdown 导入约束：1–10 个 .md 文件，单文件 ≤ 2MB。
export const IMPORT_ACCEPT = '.md,.markdown'
export const IMPORT_MAX_FILES = 10
export const IMPORT_MAX_FILE_SIZE = 2 * 1024 * 1024

const MEDIA_EXTENSIONS = new Set(['jpg', 'jpeg', 'png', 'gif', 'bmp', 'webp'])

// 客户端预校验，提前给出明确错误；最终约束仍以 Backend 为准。
export function validateMediaFiles(files: File[]): string | null {
  if (files.length === 0) return '请选择要上传的文件'
  if (files.length > MEDIA_MAX_BATCH) return `一次最多上传 ${MEDIA_MAX_BATCH} 个文件`
  for (const file of files) {
    const extension = file.name.split('.').pop()?.toLowerCase() ?? ''
    if (!MEDIA_EXTENSIONS.has(extension)) {
      return `不支持的文件类型：${file.name}（仅支持 jpg/jpeg/png/gif/bmp/webp）`
    }
    if (file.size > MEDIA_MAX_FILE_SIZE) return `文件 ${file.name} 超过 5MB 限制`
  }
  return null
}

export function validateImportFiles(files: File[]): string | null {
  if (files.length === 0) return '请选择要导入的 Markdown 文件'
  if (files.length > IMPORT_MAX_FILES) return `一次最多导入 ${IMPORT_MAX_FILES} 个文件`
  for (const file of files) {
    const extension = file.name.split('.').pop()?.toLowerCase() ?? ''
    if (extension !== 'md' && extension !== 'markdown') {
      return `不支持的文件类型：${file.name}（仅支持 .md）`
    }
    if (file.size > IMPORT_MAX_FILE_SIZE) return `文件 ${file.name} 超过 2MB 限制`
  }
  return null
}

// 由导入预览与逐篇策略生成 Commit payload：仅冲突项需要策略，未选择的冲突项默认 skip。
export function buildCommitItems(
  items: ImportPreviewItem[],
  strategies: Record<number, ImportStrategy>,
): CommitImportItem[] {
  return items
    .filter((item) => item.slug_conflict)
    .map((item) => ({ index: item.index, strategy: strategies[item.index] ?? 'skip' }))
}

// 读取 409 MEDIA_IN_USE 错误 details 中的引用摘要；非引用冲突时返回 null。
export function getMediaInUseUsages(error: unknown): MediaUsage[] | null {
  if (!axios.isAxiosError<ApiErrorResponse>(error)) return null
  const apiError = error.response?.data?.error
  if (apiError?.code !== 'MEDIA_IN_USE') return null
  const usages = apiError.details?.usages
  return Array.isArray(usages) ? (usages as MediaUsage[]) : []
}

function buildUploadFormData(files: File[]) {
  const form = new FormData()
  // Multipart 字段名与 Backend 约定为 `file[]`。
  for (const file of files) form.append('file[]', file)
  return form
}

function toPercent(event: { loaded: number; total?: number }) {
  if (!event.total) return 0
  return Math.min(100, Math.round((event.loaded / event.total) * 100))
}

export const mediaApi = {
  async list(params: {
    page: number
    page_size: number
    keyword?: string
    provider?: MediaProvider
    mime?: string
  }) {
    const { data } = await api.get<MediaPage>('/api/admin/media', { params })
    return data
  },

  async upload(files: File[], onUploadProgress?: (percent: number) => void) {
    const { data } = await api.post<MediaAsset[]>('/api/admin/media', buildUploadFormData(files), {
      onUploadProgress: (event) => onUploadProgress?.(toPercent(event)),
    })
    return data
  },

  async uploadRemote(url: string) {
    const { data } = await api.post<MediaAsset>('/api/admin/media/remote', { url })
    return data
  },

  async get(assetId: number) {
    const { data } = await api.get<MediaAsset>(`/api/admin/media/${assetId}`)
    return data
  },

  async update(assetId: number, payload: UpdateMediaPayload) {
    const { data } = await api.put<MediaAsset>(`/api/admin/media/${assetId}`, payload)
    return data
  },

  // 软删除；仍存在引用时 Backend 返回 409 MEDIA_IN_USE 并携带 usages 摘要。
  async remove(assetId: number) {
    await api.delete(`/api/admin/media/${assetId}`)
  },

  // 批量软删除，部分成功：被引用项跳过并归类返回（1–100 个 ID，由 Backend 校验）。
  async batchDelete(ids: number[]) {
    const { data } = await api.post<MediaBatchDeleteResult>('/api/admin/media/batch-delete', { ids })
    return data
  },

  async listUsages(assetId: number) {
    const { data } = await api.get<MediaUsage[]>(`/api/admin/media/${assetId}/usages`)
    return data
  },
}

export const importsApi = {
  async createImport(files: File[], onUploadProgress?: (percent: number) => void) {
    const { data } = await api.post<ImportJob>(
      '/api/admin/articles/imports',
      buildUploadFormData(files),
      { onUploadProgress: (event) => onUploadProgress?.(toPercent(event)) },
    )
    return data
  },

  async getImport(jobId: number) {
    const { data } = await api.get<ImportJob>(`/api/admin/imports/${jobId}`)
    return data
  },

  // 所有导入文章一律落 Draft；Job 只允许 Commit 一次。
  async commitImport(jobId: number, items: CommitImportItem[]) {
    const { data } = await api.post<CommitImportResult>(`/api/admin/imports/${jobId}/commit`, {
      items,
    })
    return data
  },
}

