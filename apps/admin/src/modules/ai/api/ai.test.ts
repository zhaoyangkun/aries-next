import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import {
  AiStreamError,
  aiApi,
  consumeAiSseStream,
  getAiErrorMessage,
  streamAiEditor,
  type AiUsageItem,
} from './ai'

const encoder = new TextEncoder()

// 把若干 SSE 文本块组装成一个 ReadableStream（每个元素一个 chunk，模拟分片）。
function streamOf(chunks: string[]): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      controller.close()
    },
  })
}

const usageItem: AiUsageItem = {
  id: 1,
  feature: 'editor_rewrite',
  operator_user_id: 2,
  model: 'gpt-test',
  status: 'success',
  prompt_tokens: 100,
  completion_tokens: 50,
  latency_ms: 800,
  error_category: null,
  created_at: '2026-09-06T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('aiApi.listUsage', () => {
  it('queries usage with pagination and feature filter', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [usageItem], total: 1, page: 1, page_size: 20 },
    })

    const result = await aiApi.listUsage({ page: 1, page_size: 20, feature: 'editor_summary' })

    expect(result.items).toEqual([usageItem])
    expect(get).toHaveBeenCalledWith('/api/admin/ai/usage', {
      params: { page: 1, page_size: 20, feature: 'editor_summary' },
    })
  })
})

describe('consumeAiSseStream', () => {
  it('aggregates delta events and reports start/usage/done in order', async () => {
    const events: string[] = []
    const deltas: string[] = []
    let fullText = ''
    let usage: unknown = null

    await consumeAiSseStream(
      streamOf([
        'event: start\ndata: {"feature":"editor_rewrite","model":"gpt-test","prompt_version":"v1"}\n\n',
        'event: delta\ndata: {"text":"你好"}\n\nevent: delta\ndata: {"text":"，世界"}\n\n',
        'event: usage\ndata: {"prompt_tokens":10,"completion_tokens":5}\n\nevent: done\ndata: {}\n\n',
      ]),
      {
        onStart: info => events.push(`start:${info.model}`),
        onDelta: text => deltas.push(text),
        onUsage: value => (usage = value),
        onDone: (text) => {
          events.push('done')
          fullText = text
        },
      },
    )

    expect(events).toEqual(['start:gpt-test', 'done'])
    expect(deltas).toEqual(['你好', '，世界'])
    expect(fullText).toBe('你好，世界')
    expect(usage).toEqual({ prompt_tokens: 10, completion_tokens: 5 })
  })

  it('handles events split across chunks and CRLF line endings', async () => {
    const deltas: string[] = []

    await consumeAiSseStream(
      streamOf(['event: delta\r\ndata: {"text":"abc', 'def"}\r\n\r\nevent: done\r\ndata: {}\r\n\r\n']),
      { onDelta: text => deltas.push(text) },
    )

    expect(deltas).toEqual(['abcdef'])
  })

  it('rejects with AiStreamError carrying the code on error events', async () => {
    const promise = consumeAiSseStream(
      streamOf(['event: delta\ndata: {"text":"partial"}\n\nevent: error\ndata: {"code":"AI_PROVIDER_TIMEOUT"}\n\n']),
      {},
    )

    await expect(promise).rejects.toBeInstanceOf(AiStreamError)
    await expect(promise).rejects.toMatchObject({ code: 'AI_PROVIDER_TIMEOUT' })
  })

  it('ignores heartbeat comments and unparseable events', async () => {
    let doneText = ''

    await consumeAiSseStream(
      streamOf([': heartbeat\n\nevent: delta\ndata: not-json\n\nevent: done\ndata: {}\n\n']),
      { onDone: text => (doneText = text) },
    )

    expect(doneText).toBe('')
  })
})

describe('streamAiEditor', () => {
  it('posts json and consumes the event stream', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(streamOf(['event: delta\ndata: {"text":"ok"}\n\nevent: done\ndata: {}\n\n']), {
        status: 200,
        headers: { 'Content-Type': 'text/event-stream' },
      }),
    )
    vi.stubGlobal('fetch', fetchMock)
    const deltas: string[] = []

    const handle = streamAiEditor('rewrite', { text: '原文' }, { onDelta: text => deltas.push(text) })
    await handle.finished

    expect(fetchMock).toHaveBeenCalledWith('/api/admin/ai/editor/rewrite', expect.objectContaining({
      method: 'POST',
      credentials: 'include',
      body: JSON.stringify({ text: '原文' }),
    }))
    expect(deltas).toEqual(['ok'])
  })

  it('rejects with AiGateError on non-sse gate errors', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'AI_DISABLED', message: 'AI is disabled' } }), { status: 403 }),
    ))

    const handle = streamAiEditor('summary', { title: 't', content: 'c' }, {})
    await expect(handle.finished).rejects.toMatchObject({ name: 'AiGateError', code: 'AI_DISABLED' })
  })

  it('maps 429 without a json body to RATE_LIMITED', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('slow down', { status: 429 })))

    const handle = streamAiEditor('metadata', { title: 't', content: 'c' }, {})
    await expect(handle.finished).rejects.toMatchObject({ name: 'AiGateError', code: 'RATE_LIMITED' })
  })

  it('posts to the tags and brief endpoints', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(streamOf(['event: done\ndata: {}\n\n']), {
        status: 200,
        headers: { 'Content-Type': 'text/event-stream' },
      }),
    )
    vi.stubGlobal('fetch', fetchMock)

    const tagsHandle = streamAiEditor('tags', { title: 't', content: 'c' }, {})
    await tagsHandle.finished
    expect(fetchMock).toHaveBeenCalledWith('/api/admin/ai/editor/tags', expect.objectContaining({
      method: 'POST',
      body: JSON.stringify({ title: 't', content: 'c' }),
    }))

    const briefHandle = streamAiEditor('brief', { title: 't', content: 'c' }, {})
    await briefHandle.finished
    expect(fetchMock).toHaveBeenCalledWith('/api/admin/ai/editor/brief', expect.objectContaining({
      method: 'POST',
      body: JSON.stringify({ title: 't', content: 'c' }),
    }))
  })

  it('cancel aborts the request and rejects the finished promise', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation((_url: string, init: RequestInit) =>
      new Promise<Response>((resolve, reject) => {
        init.signal?.addEventListener('abort', () =>
          reject(new DOMException('The operation was aborted.', 'AbortError')))
        // 异步 resolve，模拟网络在途：同步调用的 cancel() 必须先于响应生效。
        setTimeout(() => resolve(new Response(streamOf(['event: delta\ndata: {"text":"x"}\n\n']), { status: 200 })), 0)
      }),
    ))

    const handle = streamAiEditor('rewrite', { text: '原文' }, {})
    handle.cancel()
    await expect(handle.finished).rejects.toMatchObject({ name: 'AbortError' })
  })
})

describe('getAiErrorMessage', () => {
  it('maps known codes to Chinese guidance and falls back for unknown ones', () => {
    expect(getAiErrorMessage('AI_DISABLED')).toContain('AI 设置')
    expect(getAiErrorMessage('AI_FEATURE_DISABLED')).toContain('编辑器助手')
    expect(getAiErrorMessage('AI_NOT_CONFIGURED')).toContain('Provider')
    expect(getAiErrorMessage('RATE_LIMITED')).toContain('频繁')
    expect(getAiErrorMessage('AI_RATE_LIMITED')).toContain('频繁')
    expect(getAiErrorMessage('AI_PROVIDER_TIMEOUT')).toContain('超时')
    expect(getAiErrorMessage('AI_INVALID_OUTPUT')).toContain('格式无效')
    expect(getAiErrorMessage('INVALID_AI_INPUT')).toContain('长度上限')
    expect(getAiErrorMessage('WHATEVER')).toBe('AI 请求失败，请稍后重试')
  })
})
