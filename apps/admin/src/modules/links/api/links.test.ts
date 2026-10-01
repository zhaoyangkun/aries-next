import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { linksApi, type Link } from './links'

const link: Link = {
  id: 1,
  category_id: 2,
  title: 'Vue',
  url: 'https://vuejs.org',
  icon_url: null,
  description: '',
  status: 'active',
  sort_order: 0,
  created_at: '2026-08-05T12:00:00Z',
  updated_at: '2026-08-05T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('linksApi', () => {
  it('lists links with filters', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [link], total: 1, page: 1, page_size: 20 },
    })

    const result = await linksApi.list({ page: 1, page_size: 20, status: 'active', category_id: 2 })

    expect(result.items).toEqual([link])
    expect(get).toHaveBeenCalledWith('/api/admin/links', {
      params: { page: 1, page_size: 20, status: 'active', category_id: 2 },
    })
  })

  it('creates and updates a link', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: link })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: link })
    const payload = {
      category_id: 2,
      title: 'Vue',
      url: 'https://vuejs.org',
      icon_url: null,
      description: '',
      status: 'active' as const,
      sort_order: 1,
    }

    await expect(linksApi.create(payload)).resolves.toEqual(link)
    await expect(linksApi.update(1, payload)).resolves.toEqual(link)
    expect(post).toHaveBeenCalledWith('/api/admin/links', payload)
    expect(put).toHaveBeenCalledWith('/api/admin/links/1', payload)
  })

  it('gets and deletes a link by id', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: link })
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(linksApi.get(1)).resolves.toEqual(link)
    await expect(linksApi.remove(1)).resolves.toBeUndefined()
    expect(get).toHaveBeenCalledWith('/api/admin/links/1')
    expect(del).toHaveBeenCalledWith('/api/admin/links/1')
  })

  it('manages link categories', async () => {
    const category = { id: 2, parent_id: null, kind: 'link', name: '技术', slug: 'tech', description: '' }
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: [category] })
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: category })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: category })
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(linksApi.listCategories()).resolves.toEqual([category])
    await linksApi.createCategory({ name: '技术' })
    await linksApi.updateCategory(2, { name: '技术圈', slug: 'tech' })
    await linksApi.removeCategory(2)

    expect(get).toHaveBeenCalledWith('/api/admin/links/categories')
    expect(post).toHaveBeenCalledWith('/api/admin/links/categories', { name: '技术' })
    expect(put).toHaveBeenCalledWith('/api/admin/links/categories/2', { name: '技术圈', slug: 'tech' })
    expect(del).toHaveBeenCalledWith('/api/admin/links/categories/2')
  })
})
