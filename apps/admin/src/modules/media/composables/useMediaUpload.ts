import { ref } from 'vue'
import { getApiError } from '@/shared/api/client'
import { mediaApi, validateMediaFiles, type MediaAsset } from '../api/media'

// 媒体上传的共享逻辑：校验、进度、重复资产（duplicate_of）提示；MediaView 与 AppMediaPicker 复用。
export function useMediaUpload(onUploaded?: (assets: MediaAsset[]) => void) {
  const uploading = ref(false)
  const progress = ref(0)
  const error = ref('')
  const duplicates = ref<MediaAsset[]>([])

  async function upload(files: File[] | FileList) {
    const list = Array.from(files)
    error.value = ''
    duplicates.value = []
    const validationError = validateMediaFiles(list)
    if (validationError) {
      error.value = validationError
      return null
    }

    uploading.value = true
    progress.value = 0
    try {
      const assets = await mediaApi.upload(list, (percent) => {
        progress.value = percent
      })
      duplicates.value = assets.filter((asset) => asset.duplicate_of != null)
      onUploaded?.(assets)
      return assets
    } catch (requestError) {
      error.value = getApiError(requestError, '文件上传失败')
      return null
    } finally {
      uploading.value = false
    }
  }

  return { uploading, progress, error, duplicates, upload }
}
