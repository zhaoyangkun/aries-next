import { api } from '@/shared/api/client'

export type PageStatus = 'draft' | 'published'

export interface CustomPage {
  id: number
  slug: string
  title: string
  content_markdown: string
  content_html: string
  status: PageStatus
  sort_order: number
  created_by: number | null
  created_at: string
  updated_at: string
}

export interface CustomPagePage {
  items: CustomPage[]
  total: number
  page: number
  page_size: number
}

export interface UpsertPagePayload {
  slug: string
  title: string
  content_markdown: string
  status?: PageStatus
  sort_order?: number
}

export const pagesApi = {
  /** 分页查询自定义页面，支持状态/关键字筛选。 */
  async list(params: { page: number, page_size: number, status?: PageStatus, keyword?: string }) {
    const { data } = await api.get<CustomPagePage>('/api/admin/pages', { params })
    return data
  },

  /** 读取单个自定义页面详情。 */
  async get(pageId: number) {
    const { data } = await api.get<CustomPage>(`/api/admin/pages/${pageId}`)
    return data
  },

  // content_html 由 Backend 渲染入库；slug 冲突返回 409。
  async create(payload: UpsertPagePayload) {
    const { data } = await api.post<CustomPage>('/api/admin/pages', payload)
    return data
  },

  /** 更新自定义页面；content_html 由 Backend 渲染入库。 */
  async update(pageId: number, payload: UpsertPagePayload) {
    const { data } = await api.put<CustomPage>(`/api/admin/pages/${pageId}`, payload)
    return data
  },

  /** 删除自定义页面。 */
  async remove(pageId: number) {
    await api.delete(`/api/admin/pages/${pageId}`)
  },
}
