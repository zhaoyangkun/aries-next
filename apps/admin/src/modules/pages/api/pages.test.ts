import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { pagesApi, type CustomPage } from './pages'

const page: CustomPage = {
  id: 1,
  slug: 'about',
  title: '关于',
  content_markdown: '# 关于',
  content_html: '<h1>关于</h1>',
  status: 'published',
  sort_order: 0,
  created_by: 1,
  created_at: '2026-08-05T12:00:00Z',
  updated_at: '2026-08-05T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('pagesApi', () => {
  it('lists pages with status and keyword filters', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [page], total: 1, page: 1, page_size: 20 },
    })

    const result = await pagesApi.list({ page: 1, page_size: 20, status: 'draft', keyword: '关于' })

    expect(result.items).toEqual([page])
    expect(get).toHaveBeenCalledWith('/api/admin/pages', {
      params: { page: 1, page_size: 20, status: 'draft', keyword: '关于' },
    })
  })

  it('creates and updates a page through the server API', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: page })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: page })
    const payload = {
      slug: 'about',
      title: '关于',
      content_markdown: '# 关于',
      status: 'published' as const,
      sort_order: 1,
    }

    await expect(pagesApi.create(payload)).resolves.toEqual(page)
    await expect(pagesApi.update(1, payload)).resolves.toEqual(page)
    expect(post).toHaveBeenCalledWith('/api/admin/pages', payload)
    expect(put).toHaveBeenCalledWith('/api/admin/pages/1', payload)
  })

  it('gets and deletes a page by id', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: page })
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(pagesApi.get(1)).resolves.toEqual(page)
    await expect(pagesApi.remove(1)).resolves.toBeUndefined()
    expect(get).toHaveBeenCalledWith('/api/admin/pages/1')
    expect(del).toHaveBeenCalledWith('/api/admin/pages/1')
  })
})
