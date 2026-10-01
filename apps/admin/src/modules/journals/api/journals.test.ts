import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { journalsApi, type Journal } from './journals'

const journal: Journal = {
  id: 1,
  content_markdown: '今天天气不错',
  content_html: '<p>今天天气不错</p>',
  visibility: 'public',
  created_by: 1,
  created_at: '2026-08-05T12:00:00Z',
  updated_at: '2026-08-05T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('journalsApi', () => {
  it('lists journals with visibility filter', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [journal], total: 1, page: 1, page_size: 20 },
    })

    const result = await journalsApi.list({ page: 1, page_size: 20, visibility: 'private' })

    expect(result.items).toEqual([journal])
    expect(get).toHaveBeenCalledWith('/api/admin/journals', {
      params: { page: 1, page_size: 20, visibility: 'private' },
    })
  })

  it('creates and updates a journal through the server API', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: journal })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: journal })
    const payload = { content_markdown: '今天天气不错', visibility: 'public' as const }

    await expect(journalsApi.create(payload)).resolves.toEqual(journal)
    await expect(journalsApi.update(1, payload)).resolves.toEqual(journal)
    expect(post).toHaveBeenCalledWith('/api/admin/journals', payload)
    expect(put).toHaveBeenCalledWith('/api/admin/journals/1', payload)
  })

  it('gets and deletes a journal by id', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: journal })
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(journalsApi.get(1)).resolves.toEqual(journal)
    await expect(journalsApi.remove(1)).resolves.toBeUndefined()
    expect(get).toHaveBeenCalledWith('/api/admin/journals/1')
    expect(del).toHaveBeenCalledWith('/api/admin/journals/1')
  })
})
