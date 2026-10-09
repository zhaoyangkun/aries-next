import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { logsApi, type LogEntry } from './logs'

const entry: LogEntry = {
  id: 1,
  ts: '2026-09-06T12:00:00Z',
  level: 'ERROR',
  target: 'aries_server::http',
  message: 'request failed',
  span_name: 'http_request',
  request_id: 'req-1',
  fields: { status: 500 },
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('logsApi', () => {
  it('lists logs with filters, pagination and level counts', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: {
        items: [entry],
        total: 1,
        page: 2,
        page_size: 50,
        level_counts: { ERROR: 1, WARN: 2, INFO: 3, DEBUG: 0, TRACE: 0 },
      },
    })

    const result = await logsApi.list({
      page: 2,
      page_size: 50,
      level: 'WARN',
      target: 'aries_server::http',
      keyword: 'failed',
      start: '2026-09-06T00:00:00Z',
      end: '2026-09-07T00:00:00Z',
    })

    expect(result.items).toEqual([entry])
    expect(result.level_counts.ERROR).toBe(1)
    expect(get).toHaveBeenCalledWith('/api/admin/logs', {
      params: {
        page: 2,
        page_size: 50,
        level: 'WARN',
        target: 'aries_server::http',
        keyword: 'failed',
        start: '2026-09-06T00:00:00Z',
        end: '2026-09-07T00:00:00Z',
      },
    })
  })

  it('builds a trace query with request_id and ascending order', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: {
        items: [entry],
        total: 1,
        page: 1,
        page_size: 20,
        level_counts: { ERROR: 1, WARN: 0, INFO: 0, DEBUG: 0, TRACE: 0 },
      },
    })

    await logsApi.list({ page: 1, page_size: 20, request_id: 'req-1', order: 'asc' })

    expect(get).toHaveBeenCalledWith('/api/admin/logs', {
      params: { page: 1, page_size: 20, request_id: 'req-1', order: 'asc' },
    })
  })

  it('passes exclude_target to hide a noisy target', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: {
        items: [],
        total: 0,
        page: 1,
        page_size: 20,
        level_counts: { ERROR: 0, WARN: 0, INFO: 0, DEBUG: 0, TRACE: 0 },
      },
    })

    await logsApi.list({ page: 1, page_size: 20, exclude_target: 'sqlx::query' })

    expect(get).toHaveBeenCalledWith('/api/admin/logs', {
      params: { page: 1, page_size: 20, exclude_target: 'sqlx::query' },
    })
  })

  it('builds an around_id context query', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: {
        items: [entry],
        total: 41,
        page: 1,
        page_size: 41,
        level_counts: { ERROR: 1, WARN: 0, INFO: 0, DEBUG: 0, TRACE: 0 },
      },
    })

    await logsApi.list({ page: 1, page_size: 20, around_id: 42 })

    expect(get).toHaveBeenCalledWith('/api/admin/logs', {
      params: { page: 1, page_size: 20, around_id: 42 },
    })
  })

  it('passes a keyset cursor and exposes next_cursor from the response', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: {
        items: [entry],
        total: 42,
        page: 1,
        page_size: 20,
        next_cursor: '1788602580000000000_5',
        level_counts: { ERROR: 1, WARN: 0, INFO: 0, DEBUG: 0, TRACE: 0 },
      },
    })

    const result = await logsApi.list({ page: 1, page_size: 20, cursor: '1788602520000000000_8' })

    expect(result.next_cursor).toBe('1788602580000000000_5')
    expect(get).toHaveBeenCalledWith('/api/admin/logs', {
      params: { page: 1, page_size: 20, cursor: '1788602520000000000_8' },
    })
  })

  it('loads level stats buckets with an hours window', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: {
        hours: 24,
        buckets: [{ bucket: '2026-09-06T00:00:00Z', ERROR: 1, WARN: 2, INFO: 3, DEBUG: 0, TRACE: 0 }],
      },
    })

    const result = await logsApi.stats(24)

    expect(result.hours).toBe(24)
    expect(result.buckets[0]?.ERROR).toBe(1)
    expect(get).toHaveBeenCalledWith('/api/admin/logs/stats', { params: { hours: 24 } })
  })

  it('loads log targets for the filter dropdown', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: { items: ['aries_server::http'] } })

    await expect(logsApi.targets()).resolves.toEqual({ items: ['aries_server::http'] })
    expect(get).toHaveBeenCalledWith('/api/admin/logs/targets')
  })

  it('reads and toggles the sql log switch', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: { enabled: false } })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: { enabled: true } })

    await expect(logsApi.getSqlLog()).resolves.toEqual({ enabled: false })
    await expect(logsApi.setSqlLog(true)).resolves.toEqual({ enabled: true })
    expect(get).toHaveBeenCalledWith('/api/admin/logs/sql')
    expect(put).toHaveBeenCalledWith('/api/admin/logs/sql', { enabled: true })
  })

  it('reads and sets the runtime level filter override', async () => {
    const override = { directives: 'aries_server=debug', restore_seconds_remaining: 900 }
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: override })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: override })

    await expect(logsApi.getFilterOverride()).resolves.toEqual(override)
    await expect(logsApi.setFilterOverride({ directives: 'aries_server=debug', restore_minutes: 15 }))
      .resolves.toEqual(override)
    expect(get).toHaveBeenCalledWith('/api/admin/logs/filter')
    expect(put).toHaveBeenCalledWith('/api/admin/logs/filter', {
      directives: 'aries_server=debug',
      restore_minutes: 15,
    })
  })

  it('clears the filter override with empty directives and without restore_minutes', async () => {
    const put = vi.spyOn(api, 'put').mockResolvedValue({
      data: { directives: '', restore_seconds_remaining: null },
    })

    await expect(logsApi.setFilterOverride({ directives: '' })).resolves.toEqual({
      directives: '',
      restore_seconds_remaining: null,
    })
    expect(put).toHaveBeenCalledWith('/api/admin/logs/filter', { directives: '' })
  })

  it('returns level stats including the channel dropped count', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: {
        hours: 24,
        buckets: [{ bucket: '2026-09-06T00:00:00Z', ERROR: 0, WARN: 0, INFO: 1, DEBUG: 0, TRACE: 0 }],
        channel_dropped: 3,
      },
    })

    const result = await logsApi.stats(24)

    expect(result.channel_dropped).toBe(3)
    expect(get).toHaveBeenCalledWith('/api/admin/logs/stats', { params: { hours: 24 } })
  })

  it('builds a tail SSE url with filters and omits empty params', () => {
    const url = logsApi.buildTailUrl({
      level: 'WARN',
      target: 'aries_server::http',
      exclude_target: 'sqlx::query',
      request_id: 'req-1',
    })

    expect(url).toBe(
      '/api/admin/logs/tail?level=WARN&target=aries_server%3A%3Ahttp&exclude_target=sqlx%3A%3Aquery&request_id=req-1',
    )
  })

  it('builds a bare tail SSE url when all params are empty', () => {
    expect(logsApi.buildTailUrl({})).toBe('/api/admin/logs/tail')
    expect(logsApi.buildTailUrl({ keyword: '', target: '' })).toBe('/api/admin/logs/tail')
  })
})
