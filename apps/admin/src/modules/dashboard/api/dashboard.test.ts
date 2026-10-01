import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import type { Comment } from '@/modules/comments/api/comments'
import { dashboardApi, type DashboardData } from './dashboard'

const pendingComment: Comment = {
  id: 1,
  target_type: 'article',
  target_id: 42,
  root_id: null,
  parent_id: null,
  author_name: 'Alice',
  author_email: 'alice@example.com',
  author_url: null,
  content_markdown: 'hello',
  content_html: '<p>hello</p>',
  status: 'pending',
  is_admin_reply: false,
  moderation_reason: null,
  ai_risk: null,
  ai_reason: null,
  ai_confidence: null,
  moderated_at: null,
  created_at: '2026-09-06T12:00:00Z',
  updated_at: '2026-09-06T12:00:00Z',
}

const dashboard: DashboardData = {
  articles: { total: 10, draft: 2, published: 7, recycled: 1 },
  comments: { total: 30, pending: 3, today: 5 },
  recent_pending_comments: [pendingComment],
  recent_failed_jobs: [
    {
      id: 9,
      kind: 'comment_notification',
      attempts: 3,
      max_attempts: 5,
      last_error: 'smtp timeout',
      updated_at: '2026-09-06T11:00:00Z',
    },
  ],
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('dashboardApi', () => {
  it('loads dashboard data with counts and recent items', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: dashboard })

    const result = await dashboardApi.get()

    expect(result).toEqual(dashboard)
    expect(result.articles.published).toBe(7)
    expect(result.recent_pending_comments).toEqual([pendingComment])
    expect(result.recent_failed_jobs[0]?.kind).toBe('comment_notification')
    expect(get).toHaveBeenCalledWith('/api/admin/dashboard')
  })
})
