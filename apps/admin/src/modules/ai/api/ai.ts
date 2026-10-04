import { api, type ApiErrorResponse } from '@/shared/api/client'

export type AiFeature = 'editor_rewrite' | 'editor_summary' | 'editor_metadata' | 'editor_tags' | 'editor_brief' | 'comment_moderation'
export type AiRequestStatus = 'success' | 'failed' | 'cancelled'

export interface AiUsageItem {
  id: number
  feature: AiFeature
  operator_user_id: number | null
  model: string
  status: AiRequestStatus
  prompt_tokens: number | null
  completion_tokens: number | null
  latency_ms: number
  error_category: string | null
  created_at: string
}

export interface AiUsagePage {
  items: AiUsageItem[]
  total: number
  page: number
  page_size: number
}

export const aiApi = {
  async listUsage(params: { page: number, page_size: number, feature?: AiFeature }) {
    const { data } = await api.get<AiUsagePage>('/api/admin/ai/usage', { params })
    return data
  },

  /** 从服务端已保存的 Provider 配置拉取可用模型列表（服务端代理请求，不触浏览器 CORS）。 */
  async listModels() {
    const { data } = await api.get<{ models: string[] }>('/api/admin/ai/models')
    return data.models
  },
}

// ============================================================
// SSE 流式编辑器助手
// ============================================================

export interface AiStreamStart {
  feature: string
  model: string
  prompt_version: string
}

export interface AiStreamUsage {
  prompt_tokens: number | null
  completion_tokens: number | null
}

// 门禁错误：请求在进入 SSE 流之前被拒（统一错误体），如 AI_DISABLED / AI_NOT_CONFIGURED / RATE_LIMITED。
export class AiGateError extends Error {
  readonly code: string

  constructor(code: string, message: string) {
    super(message)
    this.name = 'AiGateError'
    this.code = code
  }
}

// 流内错误：error 事件终止流（AI_PROVIDER_FAILED / AI_PROVIDER_TIMEOUT / AI_RATE_LIMITED / AI_INVALID_OUTPUT）。
export class AiStreamError extends Error {
  readonly code: string

  constructor(code: string) {
    super(code)
    this.name = 'AiStreamError'
    this.code = code
  }
}

export interface AiStreamHandlers {
  onStart?: (info: AiStreamStart) => void
  onDelta?: (text: string) => void
  onUsage?: (usage: AiStreamUsage) => void
  onDone?: (fullText: string) => void
}

// 错误码 → 中文提示；门禁类错误附设置页引导。
export function getAiErrorMessage(code: string): string {
  switch (code) {
    case 'AI_DISABLED':
      return 'AI 功能未启用，请到「设置 → AI 设置」中开启'
    case 'AI_FEATURE_DISABLED':
      return '编辑器助手未开启，请到「设置 → AI 设置」中开启该功能'
    case 'AI_NOT_CONFIGURED':
      return 'AI 尚未完成 Provider 配置（Base URL / Model / API Key），请到「设置 → AI 设置」中配置'
    case 'RATE_LIMITED':
    case 'AI_RATE_LIMITED':
      return '请求过于频繁（每用户每分钟 10 次），请稍后再试'
    case 'AI_PROVIDER_TIMEOUT':
      return 'AI 服务响应超时，请稍后重试'
    case 'AI_INVALID_OUTPUT':
      return 'AI 输出格式无效，请重新生成'
    case 'INVALID_AI_INPUT':
      return '内容超出 AI 处理长度上限（约 6 万字符），请缩短正文或仅选中片段使用改写'
    case 'AI_PROVIDER_FAILED':
      return 'AI 服务调用失败，请稍后重试'
    default:
      return 'AI 请求失败，请稍后重试'
  }
}

function dispatchSseEvent(eventName: string, data: string, handlers: AiStreamHandlers, accumulated: { text: string }) {
  let payload: Record<string, unknown>
  try {
    payload = data ? JSON.parse(data) : {}
  }
  catch {
    // 无法解析的事件直接忽略，不中断流。
    return
  }
  switch (eventName) {
    case 'start':
      handlers.onStart?.(payload as unknown as AiStreamStart)
      break
    case 'delta': {
      const text = typeof payload.text === 'string' ? payload.text : ''
      accumulated.text += text
      handlers.onDelta?.(text)
      break
    }
    case 'usage':
      handlers.onUsage?.(payload as unknown as AiStreamUsage)
      break
    case 'done':
      handlers.onDone?.(accumulated.text)
      break
    case 'error':
      throw new AiStreamError(typeof payload.code === 'string' ? payload.code : 'AI_PROVIDER_FAILED')
  }
}

// 逐行解析 text/event-stream：事件以空行分隔，支持跨 chunk 的分片与 CRLF。
export async function consumeAiSseStream(
  stream: ReadableStream<Uint8Array>,
  handlers: AiStreamHandlers,
): Promise<void> {
  const reader = stream.getReader()
  const decoder = new TextDecoder()
  const accumulated = { text: '' }
  let buffer = ''
  let eventName = 'message'
  let dataLines: string[] = []

  const flushEvent = () => {
    if (dataLines.length === 0) return
    const data = dataLines.join('\n')
    dataLines = []
    const name = eventName
    eventName = 'message'
    dispatchSseEvent(name, data, handlers, accumulated)
  }

  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      let newlineIndex = buffer.indexOf('\n')
      while (newlineIndex >= 0) {
        let line = buffer.slice(0, newlineIndex)
        buffer = buffer.slice(newlineIndex + 1)
        if (line.endsWith('\r')) line = line.slice(0, -1)
        if (line === '') {
          flushEvent()
        }
        else if (line.startsWith(':')) {
          // 注释行（心跳），忽略。
        }
        else if (line.startsWith('event:')) {
          eventName = line.slice(6).trim()
        }
        else if (line.startsWith('data:')) {
          dataLines.push(line.slice(5).replace(/^ /, ''))
        }
        newlineIndex = buffer.indexOf('\n')
      }
    }
    flushEvent()
  }
  finally {
    reader.releaseLock()
  }
}

export interface AiStreamHandle {
  /** done 事件后 resolve；门禁错误/流内 error/取消时 reject（取消抛 AbortError）。 */
  finished: Promise<void>
  cancel: () => void
}

async function readGateError(response: Response): Promise<AiGateError> {
  try {
    const body = (await response.json()) as ApiErrorResponse
    const code = body.error?.code
    if (code) return new AiGateError(code, body.error?.message ?? code)
  }
  catch {
    // 非 JSON 错误体：按状态码兜底。
  }
  return new AiGateError(
    response.status === 429 ? 'RATE_LIMITED' : `HTTP_${response.status}`,
    `AI request failed with status ${response.status}`,
  )
}

// 发起 SSE 编辑器请求（fetch + ReadableStream；axios 不支持 SSE）。
// Cookie 认证：credentials: 'include'；Origin 由浏览器自动携带，满足 admin Origin 校验。
export function streamAiEditor(
  endpoint: 'rewrite' | 'summary' | 'metadata' | 'tags' | 'brief',
  body: Record<string, string>,
  handlers: AiStreamHandlers,
): AiStreamHandle {
  const controller = new AbortController()
  const baseUrl = import.meta.env.VITE_API_BASE_URL ?? ''
  const finished = (async () => {
    const response = await fetch(`${baseUrl}/api/admin/ai/editor/${endpoint}`, {
      method: 'POST',
      credentials: 'include',
      headers: { 'Content-Type': 'application/json', Accept: 'text/event-stream' },
      body: JSON.stringify(body),
      signal: controller.signal,
    })
    if (!response.ok) throw await readGateError(response)
    if (!response.body) throw new AiGateError('NO_STREAM', 'AI response has no stream body')
    await consumeAiSseStream(response.body, handlers)
  })()
  return {
    finished,
    cancel: () => controller.abort(),
  }
}
