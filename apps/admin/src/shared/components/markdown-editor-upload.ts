import { mediaApi, validateMediaFiles } from '@/modules/media/api/media'
import { getApiError } from '@/shared/api/client'

// MarkdownEditor 的默认图片上传实现：走媒体库接口，成功后以图片 Markdown 插入正文。
// 返回字符串即视为错误提示（Vditor handler 语义），返回 null 表示成功。
export async function uploadEditorImages(
  files: File[],
  insert: (markdown: string) => void,
): Promise<string | null> {
  const validationError = validateMediaFiles(files)
  if (validationError) return validationError
  try {
    const uploaded = await mediaApi.upload(files)
    const markdown = uploaded
      .map(asset => `![${asset.alt || asset.original_name}](${asset.url})`)
      .join('\n')
    insert(`${markdown}\n`)
    return null
  }
  catch (requestError) {
    return getApiError(requestError, '图片上传失败')
  }
}
