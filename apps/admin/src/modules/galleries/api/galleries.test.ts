import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { galleriesApi, type Gallery, type GalleryItem } from './galleries'

const gallery: Gallery = {
  id: 1,
  category_id: 2,
  slug: 'travel',
  title: '旅行',
  description: '',
  cover_media_id: null,
  status: 'draft',
  sort_order: 0,
  created_at: '2026-08-05T12:00:00Z',
  updated_at: '2026-08-05T12:00:00Z',
}

const item: GalleryItem = {
  id: 10,
  gallery_id: 1,
  media_asset_id: 5,
  alt: '山顶',
  location: '黄山',
  sort_order: 0,
  media: {
    id: 5,
    url: '/api/media/files/2026/08/abc.png',
    alt: '山顶',
    width: 800,
    height: 600,
  },
  created_at: '2026-08-05T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('galleriesApi', () => {
  it('lists galleries with filters', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [gallery], total: 1, page: 1, page_size: 20 },
    })

    const result = await galleriesApi.list({ page: 1, page_size: 20, status: 'published', category_id: 2 })

    expect(result.items).toEqual([gallery])
    expect(get).toHaveBeenCalledWith('/api/admin/galleries', {
      params: { page: 1, page_size: 20, status: 'published', category_id: 2 },
    })
  })

  it('creates and updates a gallery', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: gallery })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: gallery })
    const payload = {
      category_id: 2,
      slug: 'travel',
      title: '旅行',
      description: '',
      cover_media_id: 5,
      status: 'published' as const,
      sort_order: 1,
    }

    await expect(galleriesApi.create(payload)).resolves.toEqual(gallery)
    await expect(galleriesApi.update(1, payload)).resolves.toEqual(gallery)
    expect(post).toHaveBeenCalledWith('/api/admin/galleries', payload)
    expect(put).toHaveBeenCalledWith('/api/admin/galleries/1', payload)
  })

  it('manages gallery items and atomic reordering', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: [item] })
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: item })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: item })
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(galleriesApi.listItems(1)).resolves.toEqual([item])
    // 条目响应内联 media 摘要（资产软删除时为 null）。
    expect(item.media?.url).toBe('/api/media/files/2026/08/abc.png')
    await galleriesApi.addItem(1, { media_asset_id: 5, alt: '山顶', location: '黄山' })
    await galleriesApi.updateItem(1, 10, { alt: '山顶日落' })
    await galleriesApi.reorderItems(1, [10, 11])
    await galleriesApi.removeItem(1, 10)

    expect(get).toHaveBeenCalledWith('/api/admin/galleries/1/items')
    expect(post).toHaveBeenCalledWith('/api/admin/galleries/1/items', {
      media_asset_id: 5,
      alt: '山顶',
      location: '黄山',
    })
    expect(put).toHaveBeenNthCalledWith(1, '/api/admin/galleries/1/items/10', { alt: '山顶日落' })
    expect(put).toHaveBeenNthCalledWith(2, '/api/admin/galleries/1/items/order', { item_ids: [10, 11] })
    expect(del).toHaveBeenCalledWith('/api/admin/galleries/1/items/10')
  })

  it('manages gallery categories', async () => {
    const category = { id: 2, parent_id: null, kind: 'gallery', name: '摄影', slug: 'photo', description: '' }
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: [category] })
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: category })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: category })
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(galleriesApi.listCategories()).resolves.toEqual([category])
    await galleriesApi.createCategory({ name: '摄影' })
    await galleriesApi.updateCategory(2, { name: '摄影集', slug: 'photo' })
    await galleriesApi.removeCategory(2)

    expect(get).toHaveBeenCalledWith('/api/admin/galleries/categories')
    expect(post).toHaveBeenCalledWith('/api/admin/galleries/categories', { name: '摄影' })
    expect(put).toHaveBeenCalledWith('/api/admin/galleries/categories/2', { name: '摄影集', slug: 'photo' })
    expect(del).toHaveBeenCalledWith('/api/admin/galleries/categories/2')
  })
})
