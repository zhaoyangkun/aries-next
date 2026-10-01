import { api } from '@/shared/api/client'

export type LinkStatus = 'active' | 'inactive'

export interface Link {
  id: number
  category_id: number | null
  title: string
  url: string
  icon_url: string | null
  description: string
  status: LinkStatus
  sort_order: number
  created_at: string
  updated_at: string
}

export interface LinkPage {
  items: Link[]
  total: number
  page: number
  page_size: number
}

export interface UpsertLinkPayload {
  category_id?: number | null
  title: string
  url: string
  icon_url?: string | null
  description?: string
  status?: LinkStatus
  sort_order?: number
}

// 友链分类复用 categories 表（kind = link）。
export interface LinkCategory {
  id: number
  parent_id: number | null
  kind: string
  name: string
  slug: string
  description: string
}

export const linksApi = {
  async list(params: {
    page: number
    page_size: number
    status?: LinkStatus
    category_id?: number
    keyword?: string
  }) {
    const { data } = await api.get<LinkPage>('/api/admin/links', { params })
    return data
  },

  async get(linkId: number) {
    const { data } = await api.get<Link>(`/api/admin/links/${linkId}`)
    return data
  },

  // URL 仅允许 http/https，Backend 会二次校验。
  async create(payload: UpsertLinkPayload) {
    const { data } = await api.post<Link>('/api/admin/links', payload)
    return data
  },

  async update(linkId: number, payload: UpsertLinkPayload) {
    const { data } = await api.put<Link>(`/api/admin/links/${linkId}`, payload)
    return data
  },

  async remove(linkId: number) {
    await api.delete(`/api/admin/links/${linkId}`)
  },

  async listCategories() {
    const { data } = await api.get<LinkCategory[]>('/api/admin/links/categories')
    return data
  },

  async createCategory(payload: { name: string, slug?: string }) {
    const { data } = await api.post<LinkCategory>('/api/admin/links/categories', payload)
    return data
  },

  async updateCategory(categoryId: number, payload: { name: string, slug?: string }) {
    const { data } = await api.put<LinkCategory>(`/api/admin/links/categories/${categoryId}`, payload)
    return data
  },

  async removeCategory(categoryId: number) {
    await api.delete(`/api/admin/links/categories/${categoryId}`)
  },
}
