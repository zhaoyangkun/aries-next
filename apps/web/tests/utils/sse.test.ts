import { describe, expect, it } from 'vitest'

import { readSseEvents } from '../../app/utils/sse'

const encoder = new TextEncoder()

function streamOf(chunks: string[]): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      controller.close()
    },
  })
}

async function collect(chunks: string[]) {
  const events: { event: string, data: string }[] = []
  for await (const event of readSseEvents(streamOf(chunks))) events.push(event)
  return events
}

describe('readSseEvents', () => {
  it('parses multiple event frames with event and data lines', async () => {
    const events = await collect([
      'event: start\ndata: {"feature":"ask"}\n\n',
      'event: delta\ndata: {"text":"你好"}\n\n',
      'event: done\ndata: {}\n\n',
    ])
    expect(events).toEqual([
      { event: 'start', data: '{"feature":"ask"}' },
      { event: 'delta', data: '{"text":"你好"}' },
      { event: 'done', data: '{}' },
    ])
  })

  it('reassembles frames split across chunk boundaries', async () => {
    const events = await collect(['event: del', 'ta\nda', 'ta: {"text', '":"a"}\n', '\nevent: done\ndata: {}\n\n'])
    expect(events).toEqual([
      { event: 'delta', data: '{"text":"a"}' },
      { event: 'done', data: '{}' },
    ])
  })

  it('normalizes CRLF line endings', async () => {
    const events = await collect(['event: delta\r\ndata: {"text":"x"}\r\n\r\n'])
    expect(events).toEqual([{ event: 'delta', data: '{"text":"x"}' }])
  })

  it('joins multi-line data with newlines', async () => {
    const events = await collect(['event: delta\ndata: line1\ndata: line2\n\n'])
    expect(events).toEqual([{ event: 'delta', data: 'line1\nline2' }])
  })

  it('keeps an explicit empty data line but skips comment-only frames', async () => {
    const events = await collect(['event: ping\ndata:\n\n', ': heartbeat\n\n', 'event: delta\ndata: {"text":"a"}\n\n'])
    expect(events).toEqual([
      { event: 'ping', data: '' },
      { event: 'delta', data: '{"text":"a"}' },
    ])
  })

  it('defaults the event name to message when event: is absent', async () => {
    const events = await collect(['data: plain\n\n'])
    expect(events).toEqual([{ event: 'message', data: 'plain' }])
  })

  it('parses a trailing frame without a closing blank line', async () => {
    const events = await collect(['event: done\ndata: {}'])
    expect(events).toEqual([{ event: 'done', data: '{}' }])
  })

  it('cancels the underlying stream when iteration stops early', async () => {
    let cancelled = false
    const stream = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(encoder.encode('event: delta\ndata: {"text":"a"}\n\n'))
        controller.enqueue(encoder.encode('event: done\ndata: {}\n\n'))
      },
      cancel() {
        cancelled = true
      },
    })
    for await (const event of readSseEvents(stream)) {
      if (event.event === 'delta') break
    }
    expect(cancelled).toBe(true)
  })
})
