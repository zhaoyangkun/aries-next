import type { LogEntry } from '../api/logs'

/**
 * 实时跟踪推送入列：去重后前插，超出 max 从尾部截掉。
 * 去重针对「服务端已发送但客户端未处理就断连，重连后按 Last-Event-ID 补齐重发」
 * 的边缘：正常补齐的条目此前从未到达客户端，不会命中去重。
 * 返回是否真正入列（重复条目返回 false，调用方不更新计数）。
 */
export function pushTailEntry(list: LogEntry[], entry: LogEntry, max: number): boolean {
  if (list.some(item => item.id === entry.id))
    return false
  list.unshift(entry)
  if (list.length > max)
    list.splice(max)
  return true
}
