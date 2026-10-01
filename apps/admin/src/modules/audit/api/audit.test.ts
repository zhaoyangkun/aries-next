import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { auditApi, type AuditLog } from './audit'

const entry: AuditLog = {
  id: 1,
  actor_user_id: 7,
  actor_username: 'owner',
  action: 'article.update',
  target_type: 'article',
  target_id: '42',
  metadata: { status: 'published' },
  created_at: '2026-09-06T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('auditApi', () => {
  it('lists audit logs with full filters and pagination', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [entry], total: 1, page: 2, page_size: 50 },
    })

    const result = await auditApi.list({
      page: 2,
      page_size: 50,
      actor_user_id: 7,
      action: 'article.update',
      target_type: 'article',
      target_id: '42',
      start: '2026-09-05T00:00:00Z',
      end: '2026-09-06T00:00:00Z',
    })

    expect(result.items).toEqual([entry])
    expect(result.total).toBe(1)
    expect(get).toHaveBeenCalledWith('/api/admin/audit-logs', {
      params: {
        page: 2,
        page_size: 50,
        actor_user_id: 7,
        action: 'article.update',
        target_type: 'article',
        target_id: '42',
        start: '2026-09-05T00:00:00Z',
        end: '2026-09-06T00:00:00Z',
      },
    })
  })

  it('lists audit logs with pagination only', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [], total: 0, page: 1, page_size: 20 },
    })

    const result = await auditApi.list({ page: 1, page_size: 20 })

    expect(result.items).toEqual([])
    expect(get).toHaveBeenCalledWith('/api/admin/audit-logs', {
      params: { page: 1, page_size: 20 },
    })
  })
})
