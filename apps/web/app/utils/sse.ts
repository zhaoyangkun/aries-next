// text/event-stream 手动解析：fetch + getReader 场景下没有原生 EventSource 可用，
// 按 SSE 规范以空行分帧，收集 event: 与 data: 行（data 允许多行，以 \n 拼接）
export interface SseEvent {
  event: string
  data: string
}

function parseFrame(frame: string): SseEvent | null {
  let event = ''
  const dataLines: string[] = []
  for (const line of frame.split('\n')) {
    if (line === '' || line.startsWith(':')) continue
    const colon = line.indexOf(':')
    const field = colon === -1 ? line : line.slice(0, colon)
    // 字段值去掉冒号后的第一个空格（SSE 规范）
    let value = colon === -1 ? '' : line.slice(colon + 1)
    if (value.startsWith(' ')) value = value.slice(1)
    if (field === 'event') event = value
    else if (field === 'data') dataLines.push(value)
  }
  if (dataLines.length === 0) return null
  return { event: event || 'message', data: dataLines.join('\n') }
}

/** 从响应流中逐帧产出 SSE 事件；调用方通过 AbortController 终止时 reader.read() 会抛 AbortError */
export async function* readSseEvents(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<SseEvent, void, unknown> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      buffer = buffer.replace(/\r\n/g, '\n')
      let boundary = buffer.indexOf('\n\n')
      while (boundary !== -1) {
        const frame = buffer.slice(0, boundary)
        buffer = buffer.slice(boundary + 2)
        const parsed = parseFrame(frame)
        if (parsed) yield parsed
        boundary = buffer.indexOf('\n\n')
      }
    }
    // 流结束时没有尾随空行的最后一帧也要处理
    const parsed = parseFrame(buffer)
    if (parsed) yield parsed
  } finally {
    // 调用方提前 break / AbortController 终止时主动取消底层流，释放连接
    await reader.cancel().catch(() => {})
  }
}
