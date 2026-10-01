import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { commentsApi, type Comment } from './comments'

const comment: Comment = {
  id: 1,
  target_type: 'article',
  target_id: 42,
  root_id: null,
  parent_id: null,
  author_name: 'Alice',
  author_email: 'alice@example.com',
  author_url: null,
  content_markdown: '**hello**',
  content_html: '<p><strong>hello</strong></p>',
  status: 'pending',
  is_admin_reply: false,
  moderation_reason: null,
  ai_risk: 'safe',
  ai_reason: 'looks fine',
  ai_confidence: 0.98,
  moderated_at: null,
  created_at: '2026-09-06T12:00:00Z',
  updated_at: '2026-09-06T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('commentsApi', () => {
  it('lists comments with filters and pagination', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [comment], total: 1, page: 2, page_size: 50 },
    })

    const result = await commentsApi.list({
      page: 2,
      page_size: 50,
      status: 'pending',
      target_type: 'article',
      target_id: 42,
      keyword: 'hello',
    })

    expect(result.items).toEqual([comment])
    expect(result.total).toBe(1)
    expect(get).toHaveBeenCalledWith('/api/admin/comments', {
      params: {
        page: 2,
        page_size: 50,
        status: 'pending',
        target_type: 'article',
        target_id: 42,
        keyword: 'hello',
      },
    })
  })

  it('loads a single comment by id', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: comment })

    const result = await commentsApi.get(1)

    expect(result).toEqual(comment)
    expect(get).toHaveBeenCalledWith('/api/admin/comments/1')
  })

  it('changes status with a moderation reason', async () => {
    const patch = vi
      .spyOn(api, 'patch')
      .mockResolvedValue({ data: { ...comment, status: 'rejected', moderation_reason: 'spam link' } })

    const result = await commentsApi.changeStatus(1, 'rejected', 'spam link')

    expect(result.status).toBe('rejected')
    expect(result.moderation_reason).toBe('spam link')
    expect(patch).toHaveBeenCalledWith('/api/admin/comments/1/status', {
      status: 'rejected',
      reason: 'spam link',
    })
  })

  it('omits the reason when it is empty', async () => {
    const patch = vi
      .spyOn(api, 'patch')
      .mockResolvedValue({ data: { ...comment, status: 'approved' } })

    await commentsApi.changeStatus(1, 'approved')

    expect(patch).toHaveBeenCalledWith('/api/admin/comments/1/status', {
      status: 'approved',
      reason: undefined,
    })
  })

  it('posts an admin reply with markdown content', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({
      data: { ...comment, is_admin_reply: true, status: 'approved' },
    })

    const result = await commentsApi.reply(1, '**thanks**')

    expect(result.is_admin_reply).toBe(true)
    expect(post).toHaveBeenCalledWith('/api/admin/comments/1/reply', {
      content_markdown: '**thanks**',
    })
  })

  it('deletes a comment without returning data', async () => {
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(commentsApi.remove(1)).resolves.toBeUndefined()
    expect(del).toHaveBeenCalledWith('/api/admin/comments/1')
  })
})
