import { afterEach, describe, expect, it, vi } from 'vitest'
import { mediaApi, type MediaAsset } from '../api/media'
import { useMediaUpload } from './useMediaUpload'

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

function makeFile(name: string, size: number) {
  return new File([new Uint8Array(size)], name)
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('useMediaUpload', () => {
  it('rejects invalid files without calling the API', async () => {
    const uploadSpy = vi.spyOn(mediaApi, 'upload')
    const { uploading, error, upload } = useMediaUpload()

    const result = await upload([])

    expect(result).toBeNull()
    expect(error.value).toBe('请选择要上传的文件')
    expect(uploadSpy).not.toHaveBeenCalled()
    expect(uploading.value).toBe(false)
  })

  it('uploads files, tracks progress and surfaces duplicate assets', async () => {
    const duplicate: MediaAsset = { ...asset, id: 2, duplicate_of: 1 }
    let progressCallback: ((percent: number) => void) | undefined
    const uploadSpy = vi.spyOn(mediaApi, 'upload').mockImplementation(async (_files, onProgress) => {
      progressCallback = onProgress
      return [asset, duplicate]
    })
    const onUploaded = vi.fn()
    const { uploading, progress, error, duplicates, upload } = useMediaUpload(onUploaded)

    const promise = upload([makeFile('a.png', 10)])
    expect(uploading.value).toBe(true)
    // 进度回调透传 mediaApi 计算的百分比。
    progressCallback?.(40)
    expect(progress.value).toBe(40)

    const result = await promise
    expect(result).toEqual([asset, duplicate])
    expect(uploading.value).toBe(false)
    expect(error.value).toBe('')
    expect(onUploaded).toHaveBeenCalledWith([asset, duplicate])
    // 同内容 Hash 命中的资产单独归入 duplicates 提示。
    expect(duplicates.value).toEqual([duplicate])
    expect(uploadSpy).toHaveBeenCalledTimes(1)
  })

  it('clears previous error and duplicates on a new upload', async () => {
    vi.spyOn(mediaApi, 'upload').mockResolvedValue([asset])
    const { error, duplicates, upload } = useMediaUpload()

    await upload([])
    expect(error.value).not.toBe('')
    // 上一轮的重复提示与错误在下一次成功上传时清空。
    duplicates.value = [{ ...asset, id: 2, duplicate_of: 1 }]
    await upload([makeFile('a.png', 10)])
    expect(error.value).toBe('')
    expect(duplicates.value).toEqual([])
  })

  it('reports API errors with the backend message and resets uploading', async () => {
    vi.spyOn(mediaApi, 'upload').mockRejectedValue({
      isAxiosError: true,
      response: { status: 400, data: { error: { code: 'INVALID_REQUEST', message: '文件类型不支持' } } },
    })
    const onUploaded = vi.fn()
    const { uploading, error, upload } = useMediaUpload(onUploaded)

    const result = await upload([makeFile('a.png', 10)])

    expect(result).toBeNull()
    expect(error.value).toBe('文件类型不支持')
    expect(onUploaded).not.toHaveBeenCalled()
    expect(uploading.value).toBe(false)
  })

  it('falls back to a generic message for non-API errors', async () => {
    vi.spyOn(mediaApi, 'upload').mockRejectedValue(new Error('network down'))
    const { error, upload } = useMediaUpload()

    await upload([makeFile('a.png', 10)])
    expect(error.value).toBe('文件上传失败')
  })
})
