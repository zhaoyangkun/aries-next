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
  /** 分页查询图库，支持状态/分类/关键字筛选。 */
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

  /** 读取单个图库详情。 */
  async get(galleryId: number) {
    const { data } = await api.get<Gallery>(`/api/admin/galleries/${galleryId}`)
    return data
  },

  /** 新建图库。 */
  async create(payload: UpsertGalleryPayload) {
    const { data } = await api.post<Gallery>('/api/admin/galleries', payload)
    return data
  },

  /** 更新图库。 */
  async update(galleryId: number, payload: UpsertGalleryPayload) {
    const { data } = await api.put<Gallery>(`/api/admin/galleries/${galleryId}`, payload)
    return data
  },

  /** 删除图库。 */
  async remove(galleryId: number) {
    await api.delete(`/api/admin/galleries/${galleryId}`)
  },

  /** 图库条目列表（内联媒体摘要，替代前端逐个拉取消除 N+1）。 */
  async listItems(galleryId: number) {
    const { data } = await api.get<GalleryItem[]>(`/api/admin/galleries/${galleryId}/items`)
    return data
  },

  /** 向图库添加条目。 */
  async addItem(galleryId: number, payload: { media_asset_id: number, alt?: string, location?: string, sort_order?: number }) {
    const { data } = await api.post<GalleryItem>(`/api/admin/galleries/${galleryId}/items`, payload)
    return data
  },

  /** 更新图库条目（alt/location/sort_order）。 */
  async updateItem(galleryId: number, itemId: number, payload: { alt?: string, location?: string, sort_order?: number }) {
    const { data } = await api.put<GalleryItem>(`/api/admin/galleries/${galleryId}/items/${itemId}`, payload)
    return data
  },

  /** 移除图库条目。 */
  async removeItem(galleryId: number, itemId: number) {
    await api.delete(`/api/admin/galleries/${galleryId}/items/${itemId}`)
  },

  // 原子批量排序：item_ids 为完整列表，Backend 事务内一次性应用。
  async reorderItems(galleryId: number, itemIds: number[]) {
    await api.put(`/api/admin/galleries/${galleryId}/items/order`, { item_ids: itemIds })
  },

  /** 图库分类列表（复用 categories 表 kind=gallery）。 */
  async listCategories() {
    const { data } = await api.get<GalleryCategory[]>('/api/admin/galleries/categories')
    return data
  },

  /** 新建图库分类。 */
  async createCategory(payload: { name: string, slug?: string }) {
    const { data } = await api.post<GalleryCategory>('/api/admin/galleries/categories', payload)
    return data
  },

  /** 更新图库分类。 */
  async updateCategory(categoryId: number, payload: { name: string, slug?: string }) {
    const { data } = await api.put<GalleryCategory>(`/api/admin/galleries/categories/${categoryId}`, payload)
    return data
  },

  /** 删除图库分类。 */
  async removeCategory(categoryId: number) {
    await api.delete(`/api/admin/galleries/categories/${categoryId}`)
  },
}
