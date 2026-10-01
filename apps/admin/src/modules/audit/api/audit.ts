import { api } from '@/shared/api/client'

export interface AuditLog {
  id: number
  actor_user_id: number | null
  actor_username: string | null
  action: string
  target_type: string
  target_id: string | null
  metadata: Record<string, unknown>
  created_at: string
}

export interface AuditPage {
  items: AuditLog[]
  total: number
  page: number
  page_size: number
}

/** 时间范围参数为 RFC 3339 字符串（如 `2026-09-05T00:00:00Z`），左闭右开。 */
export const auditApi = {
  async list(params: {
    page: number
    page_size: number
    actor_user_id?: number
    action?: string
    target_type?: string
    target_id?: string
    start?: string
    end?: string
  }) {
    const { data } = await api.get<AuditPage>('/api/admin/audit-logs', { params })
    return data
  },
}
