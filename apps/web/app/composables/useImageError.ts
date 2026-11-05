// 图片加载失败状态：封面/缩略图统一兜底（对齐 CommentItem 头像的 @error 退化）。
// 返回 failed 标记与 img @error 处理器；显式引入 ref 以便测试直接 import 本文件。
import { ref } from 'vue'

export function useImageError() {
  const failed = ref(false)
  function onImageError() {
    failed.value = true
  }
  return { failed, onImageError }
}
