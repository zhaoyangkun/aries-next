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
  /** 分页查询文章列表，支持状态/分类/标签/关键字筛选与排序。 */
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

  /** 新建文章；payload 的三态语义见 CreateArticlePayload。 */
  async create(payload: CreateArticlePayload) {
    const { data } = await api.post<AdminArticle>('/api/admin/articles', payload)
    return data
  },

  /** 读取单篇文章详情。 */
  async get(articleId: number) {
    const { data } = await api.get<AdminArticle>(`/api/admin/articles/${articleId}`)
    return data
  },

  /** 全量更新文章，携带 expected_version 乐观锁，冲突返回 409。 */
  async update(
    articleId: number,
    payload: CreateArticlePayload & { expected_version: number },
  ) {
    const { data } = await api.put<AdminArticle>(`/api/admin/articles/${articleId}`, payload)
    return data
  },

  /** 列出文章的历史版本。 */
  async listRevisions(articleId: number) {
    const { data } = await api.get<ArticleRevision[]>(
      `/api/admin/articles/${articleId}/revisions`,
    )
    return data
  },

  /** 回滚到指定历史版本（乐观锁），返回恢复后的文章。 */
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

  // 跨页边界移动一篇文章（排序模式下页首上移/页尾下移，top/bottom 移到所在置顶分组最前/最后）；
  // moved=false 表示文章已在置顶分组边界，位置未变（前端据此提示）。
  async move(articleId: number, direction: 'up' | 'down' | 'top' | 'bottom') {
    const { data } = await api.put<{ moved: boolean }>(`/api/admin/articles/${articleId}/move`, { direction })
    return data.moved
  },

  /** 文章状态流转（发布/下线/回收/还原等）；非法转换 Backend 返回 409。 */
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

  /** Markdown 实时预览：返回 Backend 渲染并 Sanitize 后的 HTML。 */
  async preview(markdown_source: string) {
    const { data } = await api.post<{ rendered_html: string }>('/api/admin/articles/preview', {
      markdown_source,
    })
    return data
  },

  /** 全部分类（不分页，供筛选下拉与编辑器选择）。 */
  async listCategories() {
    const { data } = await api.get<ArticleCategory[]>('/api/admin/categories')
    return data
  },

  /** 新建分类。 */
  async createCategory(name: string) {
    const { data } = await api.post<ArticleCategory>('/api/admin/categories', { name })
    return data
  },

  /** 更新分类（name/slug）。 */
  async updateCategory(categoryId: number, payload: { name: string; slug?: string }) {
    const { data } = await api.put<ArticleCategory>(
      `/api/admin/categories/${categoryId}`,
      payload,
    )
    return data
  },

  /** 删除分类；存在文章引用时 Backend 拒绝。 */
  async deleteCategory(categoryId: number) {
    await api.delete(`/api/admin/categories/${categoryId}`)
  },

  /** 全部标签（不分页）。 */
  async listTags() {
    const { data } = await api.get<ArticleTag[]>('/api/admin/tags')
    return data
  },

  /** 新建标签。 */
  async createTag(name: string) {
    const { data } = await api.post<ArticleTag>('/api/admin/tags', { name })
    return data
  },

  /** 更新标签（name/slug）。 */
  async updateTag(tagId: number, payload: { name: string; slug?: string }) {
    const { data } = await api.put<ArticleTag>(`/api/admin/tags/${tagId}`, payload)
    return data
  },

  /** 删除标签。 */
  async deleteTag(tagId: number) {
    await api.delete(`/api/admin/tags/${tagId}`)
  },
}
