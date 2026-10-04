import type {
  ArticleCategoryResponse,
  ArticlePageResponse,
  ArticleResponse,
  ArticleRevisionResponse,
  ArticleStatus,
  ArticleStatusCommand,
  ArticleTagResponse,
} from '@aries/api-client'
import { api } from '@/shared/api/client'

// Response DTO 直接派生自 @aries/api-client（由 docs/openapi.yaml 生成），避免与 Backend 漂移。
// ai_brief 为 Backend 新增字段，api-client 尚未重新生成，先以交叉类型局部扩展，待重新生成后移除。
export type { ArticleStatus, ArticleStatusCommand }
export type ArticleCategory = ArticleCategoryResponse
export type ArticleTag = ArticleTagResponse
export type AdminArticle = ArticleResponse & { ai_brief?: string | null }
export type ArticlePage = ArticlePageResponse
export type ArticleRevision = ArticleRevisionResponse

// Query 参数与请求 Payload 暂无对应命名 Schema（sort/order 为内联枚举），保持手写。
export type ArticleSort = 'updated_at' | 'created_at' | 'published_at' | 'title' | 'sort_order'
export type ArticleSortOrder = 'asc' | 'desc'

// 待迁：与 CreateArticleRequest 的差异在于 summary/markdown_source 在编辑器里恒为必填，
// 且 access_password 三态由客户端组装；待 openapi.yaml 补充编辑器视角的 Schema 后再对齐。
export interface CreateArticlePayload {
  title: string
  slug?: string
  summary: string
  markdown_source: string
  category_id?: number | null
  cover_url?: string | null
  seo_keywords?: string[]
  // 三态语义：字段缺省表示不改动，显式 null 表示清除，字符串表示设置新密码。
  access_password?: string | null
  allow_comments?: boolean
  is_pinned?: boolean
  tag_ids?: number[]
  // AI 导读：空串按未设置处理，提交 null 清除已保存的导读。
  ai_brief?: string | null
}

export const articlesApi = {
  async list(params: {
    page: number
    page_size: number
    keyword?: string
    status?: ArticleStatus
    category_id?: number
    tag_id?: number
    sort?: ArticleSort
    order?: ArticleSortOrder
  }) {
    const { data } = await api.get<ArticlePage>('/api/admin/articles', { params })
    return data
  },

  async create(payload: CreateArticlePayload) {
    const { data } = await api.post<AdminArticle>('/api/admin/articles', payload)
    return data
  },

  async get(articleId: number) {
    const { data } = await api.get<AdminArticle>(`/api/admin/articles/${articleId}`)
    return data
  },

  async update(
    articleId: number,
    payload: CreateArticlePayload & { expected_version: number },
  ) {
    const { data } = await api.put<AdminArticle>(`/api/admin/articles/${articleId}`, payload)
    return data
  },

  async listRevisions(articleId: number) {
    const { data } = await api.get<ArticleRevision[]>(
      `/api/admin/articles/${articleId}/revisions`,
    )
    return data
  },

  async restoreRevision(articleId: number, revisionNo: number, expectedVersion: number) {
    const { data } = await api.post<AdminArticle>(
      `/api/admin/articles/${articleId}/revisions/${revisionNo}/restore`,
      { expected_version: expectedVersion },
    )
    return data
  },

  // 物理删除：仅 recycled 状态的文章允许删除，其余状态 Backend 返回 409。
  async remove(articleId: number) {
    await api.delete(`/api/admin/articles/${articleId}`)
  },

  /**
   * 批量重排文章手动排序值（下锚语义）。ids 为当前可视列表按期望顺序全量提交
   * （典型：排序模式下当前页的完整有序 id）；幂等，重复提交同一列表结果一致。
   */
  async reorder(articleIds: number[]) {
    await api.put('/api/admin/articles/reorder', { article_ids: articleIds })
  },

  async changeStatus(
    articleId: number,
    command: ArticleStatusCommand,
    expected_version: number,
  ) {
    const { data } = await api.patch<AdminArticle>(`/api/admin/articles/${articleId}/status`, {
      command,
      expected_version,
    })
    return data
  },

  async preview(markdown_source: string) {
    const { data } = await api.post<{ rendered_html: string }>('/api/admin/articles/preview', {
      markdown_source,
    })
    return data
  },

  async listCategories() {
    const { data } = await api.get<ArticleCategory[]>('/api/admin/categories')
    return data
  },

  async createCategory(name: string) {
    const { data } = await api.post<ArticleCategory>('/api/admin/categories', { name })
    return data
  },

  async updateCategory(categoryId: number, payload: { name: string; slug?: string }) {
    const { data } = await api.put<ArticleCategory>(
      `/api/admin/categories/${categoryId}`,
      payload,
    )
    return data
  },

  async deleteCategory(categoryId: number) {
    await api.delete(`/api/admin/categories/${categoryId}`)
  },

  async listTags() {
    const { data } = await api.get<ArticleTag[]>('/api/admin/tags')
    return data
  },

  async createTag(name: string) {
    const { data } = await api.post<ArticleTag>('/api/admin/tags', { name })
    return data
  },

  async updateTag(tagId: number, payload: { name: string; slug?: string }) {
    const { data } = await api.put<ArticleTag>(`/api/admin/tags/${tagId}`, payload)
    return data
  },

  async deleteTag(tagId: number) {
    await api.delete(`/api/admin/tags/${tagId}`)
  },
}
