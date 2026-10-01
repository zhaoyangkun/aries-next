import { api } from '@/shared/api/client'
import type { Comment } from '@/modules/comments/api/comments'

export interface DashboardArticles {
  total: number
  draft: number
  published: number
  recycled: number
}

export interface DashboardComments {
  total: number
  pending: number
  today: number
}

export interface FailedJob {
  id: number
  kind: string
  attempts: number
  max_attempts: number
  last_error: string | null
  updated_at: string
}

export interface DashboardData {
  articles: DashboardArticles
  comments: DashboardComments
  recent_pending_comments: Comment[]
  recent_failed_jobs: FailedJob[]
}

export const dashboardApi = {
  async get() {
    const { data } = await api.get<DashboardData>('/api/admin/dashboard')
    return data
  },
}
