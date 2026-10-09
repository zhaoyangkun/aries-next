import { describe, expect, it } from 'vitest'
import type { LogEntry } from '../api/logs'
import { pushTailEntry } from './tail'

function entry(id: number): LogEntry {
  return {
    id,
    ts: '2026-09-06T12:00:00Z',
    level: 'INFO',
    target: 'aries_server::http',
    message: `log-${id}`,
    span_name: null,
    request_id: null,
    fields: {},
  }
}

describe('pushTailEntry', () => {
  it('unshifts new entries at the head', () => {
    const list = [entry(1)]

    expect(pushTailEntry(list, entry(2), 10)).toBe(true)

    expect(list.map(item => item.id)).toEqual([2, 1])
  })

  it('deduplicates an already-present entry id (reconnect backfill replay edge)', () => {
    const list = [entry(2), entry(1)]

    expect(pushTailEntry(list, entry(2), 10)).toBe(false)

    expect(list.map(item => item.id)).toEqual([2, 1])
  })

  it('trims the list from the tail when exceeding max', () => {
    const list = [entry(3), entry(2), entry(1)]

    expect(pushTailEntry(list, entry(4), 3)).toBe(true)

    expect(list.map(item => item.id)).toEqual([4, 3, 2])
  })

  it('accepts backfilled entries that were never delivered before (not treated as duplicates)', () => {
    // 断开窗口的日志 id 大于已收条目但此前从未到达客户端：正常入列。
    const list = [entry(2), entry(1)]

    expect(pushTailEntry(list, entry(3), 10)).toBe(true)
    expect(pushTailEntry(list, entry(4), 10)).toBe(true)

    expect(list.map(item => item.id)).toEqual([4, 3, 2, 1])
  })
})
