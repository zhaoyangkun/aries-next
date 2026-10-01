import type {
  CommentPageResponse,
  CommentResponse,
  CommentStatus,
  CommentTargetType,
} from '@aries/api-client'
import { api } from '@/shared/api/client'

// Response DTO 直接派生自 @aries/api-client（由 docs/openapi.yaml 生成）。
export type { CommentStatus, CommentTargetType }
/** AI 审核风险结论（可空）：开启「评论自动审核」后由 Backend 在提交时写入。 */
export type CommentAiRisk = NonNullable<CommentResponse['ai_risk']>
/** Admin 端评论 DTO：含访客 Email（审核判断依据），Public API 永不返回该字段。 */
export type Comment = CommentResponse
export type CommentPage = CommentPageResponse

export const commentsApi = {
  async list(params: {
    page: number
    page_size: number
    status?: CommentStatus
    target_type?: CommentTargetType
    target_id?: number
    keyword?: string
  }) {
    const { data } = await api.get<CommentPage>('/api/admin/comments', { params })
    return data
  },

  async get(commentId: number) {
    const { data } = await api.get<Comment>(`/api/admin/comments/${commentId}`)
    return data
  },

  /** 审核操作：状态变更遵循 core 状态机，非法转换 Backend 返回 409。 */
  async changeStatus(commentId: number, status: CommentStatus, reason?: string) {
    const { data } = await api.patch<Comment>(`/api/admin/comments/${commentId}/status`, {
      status,
      reason: reason || undefined,
    })
    return data
  },

  /** 管理员回复：Markdown 渲染 + Sanitize 后返回 approved 评论。 */
  async reply(commentId: number, content_markdown: string) {
    const { data } = await api.post<Comment>(`/api/admin/comments/${commentId}/reply`, {
      content_markdown,
    })
    return data
  },

  /** 物理删除：仅 recycled 状态允许，其余 Backend 返回 409。 */
  async remove(commentId: number) {
    await api.delete(`/api/admin/comments/${commentId}`)
  },
}
