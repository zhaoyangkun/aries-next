import { api } from '@/shared/api/client'

export type JournalVisibility = 'public' | 'private'

export interface Journal {
  id: number
  content_markdown: string
  content_html: string
  visibility: JournalVisibility
  created_by: number | null
  created_at: string
  updated_at: string
}

export interface JournalPage {
  items: Journal[]
  total: number
  page: number
  page_size: number
}

export interface UpsertJournalPayload {
  content_markdown: string
  visibility?: JournalVisibility
}

export const journalsApi = {
  async list(params: { page: number, page_size: number, visibility?: JournalVisibility }) {
    const { data } = await api.get<JournalPage>('/api/admin/journals', { params })
    return data
  },

  async get(journalId: number) {
    const { data } = await api.get<Journal>(`/api/admin/journals/${journalId}`)
    return data
  },

  // private 日志不出现在 Public 端点；content_html 由 Backend 渲染入库。
  async create(payload: UpsertJournalPayload) {
    const { data } = await api.post<Journal>('/api/admin/journals', payload)
    return data
  },

  async update(journalId: number, payload: UpsertJournalPayload) {
    const { data } = await api.put<Journal>(`/api/admin/journals/${journalId}`, payload)
    return data
  },

  async remove(journalId: number) {
    await api.delete(`/api/admin/journals/${journalId}`)
  },
}
