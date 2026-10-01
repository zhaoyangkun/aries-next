import { api } from '@/shared/api/client'

export type GalleryStatus = 'draft' | 'published'

export interface Gallery {
  id: number
  category_id: number
  slug: string
  title: string
  description: string
  cover_media_id: number | null
  status: GalleryStatus
  sort_order: number
  created_at: string
  updated_at: string
}

export interface GalleryPage {
  items: Gallery[]
  total: number
  page: number
  page_size: number
}

export interface UpsertGalleryPayload {
  category_id: number
  slug: string
  title: string
  description?: string
  cover_media_id?: number | null
  status?: GalleryStatus
  sort_order?: number
}

// 条目内联的媒体摘要：资产被软删除时为 null（破图占位由页面处理）。
export interface GalleryItemMedia {
  id: number
  url: string
  alt: string
  width: number | null
  height: number | null
}

export interface GalleryItem {
  id: number
  gallery_id: number
  media_asset_id: number
  alt: string
  location: string
  sort_order: number
  // 内联媒体摘要，替代前端按 media_asset_id 逐个拉取（消除 N+1）。
  media: GalleryItemMedia | null
  created_at: string
}

// 图库分类复用 categories 表（kind = gallery），与 taxonomy 的 CategoryResponse 一致。
export interface GalleryCategory {
  id: number
  parent_id: number | null
  kind: string
  name: string
  slug: string
  description: string
}

export const galleriesApi = {
  async list(params: {
    page: number
    page_size: number
    status?: GalleryStatus
    category_id?: number
    keyword?: string
  }) {
    const { data } = await api.get<GalleryPage>('/api/admin/galleries', { params })
    return data
  },

  async get(galleryId: number) {
    const { data } = await api.get<Gallery>(`/api/admin/galleries/${galleryId}`)
    return data
  },

  async create(payload: UpsertGalleryPayload) {
    const { data } = await api.post<Gallery>('/api/admin/galleries', payload)
    return data
  },

  async update(galleryId: number, payload: UpsertGalleryPayload) {
    const { data } = await api.put<Gallery>(`/api/admin/galleries/${galleryId}`, payload)
    return data
  },

  async remove(galleryId: number) {
    await api.delete(`/api/admin/galleries/${galleryId}`)
  },

  async listItems(galleryId: number) {
    const { data } = await api.get<GalleryItem[]>(`/api/admin/galleries/${galleryId}/items`)
    return data
  },

  async addItem(galleryId: number, payload: { media_asset_id: number, alt?: string, location?: string, sort_order?: number }) {
    const { data } = await api.post<GalleryItem>(`/api/admin/galleries/${galleryId}/items`, payload)
    return data
  },

  async updateItem(galleryId: number, itemId: number, payload: { alt?: string, location?: string, sort_order?: number }) {
    const { data } = await api.put<GalleryItem>(`/api/admin/galleries/${galleryId}/items/${itemId}`, payload)
    return data
  },

  async removeItem(galleryId: number, itemId: number) {
    await api.delete(`/api/admin/galleries/${galleryId}/items/${itemId}`)
  },

  // 原子批量排序：item_ids 为完整列表，Backend 事务内一次性应用。
  async reorderItems(galleryId: number, itemIds: number[]) {
    await api.put(`/api/admin/galleries/${galleryId}/items/order`, { item_ids: itemIds })
  },

  async listCategories() {
    const { data } = await api.get<GalleryCategory[]>('/api/admin/galleries/categories')
    return data
  },

  async createCategory(payload: { name: string, slug?: string }) {
    const { data } = await api.post<GalleryCategory>('/api/admin/galleries/categories', payload)
    return data
  },

  async updateCategory(categoryId: number, payload: { name: string, slug?: string }) {
    const { data } = await api.put<GalleryCategory>(`/api/admin/galleries/categories/${categoryId}`, payload)
    return data
  },

  async removeCategory(categoryId: number) {
    await api.delete(`/api/admin/galleries/categories/${categoryId}`)
  },
}
