import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import {
  buildCommitItems,
  getMediaInUseUsages,
  importsApi,
  mediaApi,
  validateImportFiles,
  validateMediaFiles,
  type ImportPreviewItem,
  type MediaAsset,
} from './media'

const asset: MediaAsset = {
  id: 1,
  provider: 'local',
  object_key: '2026/08/abc.png',
  url: '/api/media/files/2026/08/abc.png',
  original_name: 'cover.png',
  mime: 'image/png',
  size_bytes: 1024,
  width: 800,
  height: 600,
  sha256: 'deadbeef',
  alt: '',
  status: 'active',
  uploaded_by: 1,
  created_at: '2026-08-05T12:00:00Z',
  updated_at: '2026-08-05T12:00:00Z',
}

const previewItem: ImportPreviewItem = {
  index: 0,
  file_name: 'hello.md',
  title: 'Hello',
  slug: 'hello',
  summary: '',
  tags: ['rust'],
  category: null,
  warnings: [],
  slug_conflict: true,
}

function makeFile(name: string, size: number) {
  return new File([new Uint8Array(size)], name)
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('mediaApi', () => {
  it('lists media assets with filters and pagination', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [asset], total: 1, page: 1, page_size: 24 },
    })

    const result = await mediaApi.list({ page: 1, page_size: 24, keyword: 'cover', provider: 'local', mime: 'image/png' })

    expect(result.items).toEqual([asset])
    expect(get).toHaveBeenCalledWith('/api/admin/media', {
      params: { page: 1, page_size: 24, keyword: 'cover', provider: 'local', mime: 'image/png' },
    })
  })

  it('uploads files as multipart form data with the file[] field name', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: [asset] })
    const files = [makeFile('a.png', 10), makeFile('b.jpg', 20)]
    const onProgress = vi.fn()

    const result = await mediaApi.upload(files, onProgress)

    expect(result).toEqual([asset])
    const [url, body, config] = post.mock.calls[0] as unknown as [string, FormData, { onUploadProgress: (event: { loaded: number; total?: number }) => void }]
    expect(url).toBe('/api/admin/media')
    expect(body).toBeInstanceOf(FormData)
    expect(body.getAll('file[]')).toHaveLength(2)
    expect((body.getAll('file[]')[0] as File).name).toBe('a.png')
    // 进度回调把 AxiosProgressEvent 转换为 0–100 的百分比。
    config.onUploadProgress({ loaded: 50, total: 200 })
    expect(onProgress).toHaveBeenCalledWith(25)
  })

  it('imports a remote image by url', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: asset })

    await expect(mediaApi.uploadRemote('https://example.com/a.png')).resolves.toEqual(asset)
    expect(post).toHaveBeenCalledWith('/api/admin/media/remote', { url: 'https://example.com/a.png' })
  })

  it('updates only alt and original_name, deletes and loads usages', async () => {
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: { ...asset, alt: '封面' } })
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })
    const usage = { id: 9, asset_id: 1, target_type: 'article_cover', target_id: 7, created_at: '2026-08-05T12:00:00Z' }
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: [usage] })

    await expect(mediaApi.update(1, { alt: '封面' })).resolves.toMatchObject({ alt: '封面' })
    await expect(mediaApi.remove(1)).resolves.toBeUndefined()
    await expect(mediaApi.listUsages(1)).resolves.toEqual([usage])
    expect(put).toHaveBeenCalledWith('/api/admin/media/1', { alt: '封面' })
    expect(del).toHaveBeenCalledWith('/api/admin/media/1')
    expect(get).toHaveBeenCalledWith('/api/admin/media/1/usages')
  })

  it('batch deletes media ids with partial-success result buckets', async () => {
    const result = { deleted: [1, 2], referenced: [3], not_found: [99] }
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: result })

    await expect(mediaApi.batchDelete([1, 2, 3, 99])).resolves.toEqual(result)
    expect(post).toHaveBeenCalledWith('/api/admin/media/batch-delete', { ids: [1, 2, 3, 99] })
  })
})

describe('importsApi', () => {
  it('creates an import job with multipart file[] fields', async () => {
    const job = { job_id: 5, status: 'done', items: [previewItem] }
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: job })

    const result = await importsApi.createImport([makeFile('hello.md', 100)])

    expect(result).toEqual(job)
    const [url, body] = post.mock.calls[0] as unknown as [string, FormData]
    expect(url).toBe('/api/admin/articles/imports')
    expect(body.getAll('file[]')).toHaveLength(1)
  })

  it('loads the import preview and commits per-item strategies', async () => {
    const job = { job_id: 5, status: 'done', items: [previewItem] }
    const resultPayload = { created: [{ id: 10, title: 'Hello', slug: 'hello-2' }], skipped: [], renamed: [{ from: 'hello', to: 'hello-2' }] }
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: job })
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: resultPayload })

    await expect(importsApi.getImport(5)).resolves.toEqual(job)
    await expect(importsApi.commitImport(5, [{ index: 0, strategy: 'rename' }])).resolves.toEqual(resultPayload)
    expect(get).toHaveBeenCalledWith('/api/admin/imports/5')
    expect(post).toHaveBeenCalledWith('/api/admin/imports/5/commit', { items: [{ index: 0, strategy: 'rename' }] })
  })
})

describe('validateMediaFiles', () => {
  it('rejects empty selection, oversized batches, bad extensions and oversized files', () => {
    expect(validateMediaFiles([])).toBe('请选择要上传的文件')
    expect(validateMediaFiles(Array.from({ length: 6 }, (_, index) => makeFile(`${index}.png`, 1)))).toContain('最多上传 5 个')
    expect(validateMediaFiles([makeFile('a.txt', 1)])).toContain('不支持的文件类型')
    expect(validateMediaFiles([makeFile('a.png', 5 * 1024 * 1024 + 1)])).toContain('超过 5MB')
    expect(validateMediaFiles([makeFile('a.PNG', 1), makeFile('b.webp', 1024)])).toBeNull()
  })
})

describe('validateImportFiles', () => {
  it('rejects non-markdown files and files over 2MB', () => {
    expect(validateImportFiles([])).toBe('请选择要导入的 Markdown 文件')
    expect(validateImportFiles([makeFile('a.png', 1)])).toContain('仅支持 .md')
    expect(validateImportFiles([makeFile('a.md', 2 * 1024 * 1024 + 1)])).toContain('超过 2MB')
    expect(validateImportFiles([makeFile('a.md', 1024)])).toBeNull()
  })
})

describe('buildCommitItems', () => {
  it('maps strategies only for conflicting items and defaults missing strategies to skip', () => {
    const items: ImportPreviewItem[] = [
      { ...previewItem, index: 0, slug: 'a', slug_conflict: true },
      { ...previewItem, index: 1, slug: 'b', slug_conflict: false },
      { ...previewItem, index: 2, slug: 'c', slug_conflict: true },
    ]

    expect(buildCommitItems(items, { 2: 'rename' })).toEqual([
      { index: 0, strategy: 'skip' },
      { index: 2, strategy: 'rename' },
    ])
    expect(buildCommitItems(items, {})).toEqual([
      { index: 0, strategy: 'skip' },
      { index: 2, strategy: 'skip' },
    ])
    expect(buildCommitItems([{ ...previewItem, slug_conflict: false }], {})).toEqual([])
  })
})

describe('getMediaInUseUsages', () => {
  it('extracts usages from a 409 MEDIA_IN_USE error and ignores other errors', () => {
    const usages = [{ id: 1, asset_id: 2, target_type: 'article_content', target_id: 3, created_at: '2026-08-05T12:00:00Z' }]
    const inUseError = {
      isAxiosError: true,
      response: { status: 409, data: { error: { code: 'MEDIA_IN_USE', details: { reference_count: 1, usages } } } },
    }
    expect(getMediaInUseUsages(inUseError)).toEqual(usages)

    const otherError = {
      isAxiosError: true,
      response: { status: 400, data: { error: { code: 'INVALID_REQUEST' } } },
    }
    expect(getMediaInUseUsages(otherError)).toBeNull()
    expect(getMediaInUseUsages(new Error('network down'))).toBeNull()
  })
})
