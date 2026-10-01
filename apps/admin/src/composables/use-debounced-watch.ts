import type { WatchSource } from 'vue'
import { getCurrentScope, onScopeDispose, watch } from 'vue'

// 防抖监听：源变化后延迟执行回调，重复触发只保留最后一次；所在作用域销毁时自动清理定时器。
export function useDebouncedWatch(
  sources: WatchSource | WatchSource[],
  callback: () => void,
  delay = 300,
) {
  let timer: ReturnType<typeof setTimeout> | undefined

  const clear = () => {
    if (timer) {
      clearTimeout(timer)
      timer = undefined
    }
  }

  watch(sources as WatchSource[], () => {
    clear()
    timer = setTimeout(callback, delay)
  })

  if (getCurrentScope())
    onScopeDispose(clear)
}
