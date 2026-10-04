// 由 docs/openapi.yaml 生成的类型（pnpm --filter @aries/api-client generate 重新生成），
// 这里导出 components / operations 原始命名空间，以及两端共用的 Schema 友好别名。
export type { components, operations, paths } from './schema'

import type { components } from './schema'

/** openapi.yaml components.schemas 的简写 */
export type Schema = components['schemas']

// ---- Public Web（apps/web）常用类型 ----
export type PublicSite = Schema['PublicSiteResponse']
export type PublicArticleListItem = Schema['PublicArticleListItem']
export type PublicArticleDetail = Schema['PublicArticleDetail']
export type PublicArticlePage = Schema['PublicArticlePageResponse']
export type PublicCategory = Schema['PublicCategoryResponse']
export type PublicTag = Schema['PublicTagResponse']
export type PublicArchiveMonth = Schema['ArchiveMonthResponse']
export type PublicPageDetail = Schema['PublicPageResponse']
export type PublicJournalItem = Schema['PublicJournalResponse']
export type PublicJournalPage = Schema['PublicJournalPageResponse']
export type PublicGallerySummary = Schema['PublicGallerySummary']
export type PublicGalleryPage = Schema['PublicGalleryPageResponse']
export type PublicGalleryItem = Schema['PublicGalleryItemResponse']
export type PublicGalleryDetail = Schema['PublicGalleryResponse']
export type PublicPhoto = Schema['PublicPhotoResponse']
export type PublicLink = Schema['PublicLinkResponse']
export type PublicNavigationNode = Schema['PublicNavigationNode']
export type PublicSearchSuggestion = Schema['SearchSuggestionResponse']
export type PublicSearchAskRequest = Schema['AskRequest']
export type RelatedArticle = Schema['RelatedArticleResponse']
/** 分类/标签引用：内联于 PublicArticleDetail.category，取其对象形态 */
export type PublicTaxonomyRef = NonNullable<Schema['PublicArticleDetail']['category']>
/** 上一篇/下一篇引用：内联于 PublicArticleDetail.previous/next */
export type PublicArticleNeighbor = NonNullable<Schema['PublicArticleDetail']['previous']>
export type PublicComment = Schema['PublicCommentResponse']
export type PublicCommentCreated = Schema['PublicCommentCreatedResponse']
export type PublicCommentPage = Schema['PublicCommentPageResponse']
export type CreatePublicCommentRequest = Schema['CreateCommentRequest']
export type ReplyPublicCommentRequest = Schema['ReplyCommentRequest']

// ---- Admin（apps/admin）核心模块类型 ----
// ArticleStatus / CommentStatus：手写契约时代是独立 enum 组件，utoipa 迁移后
// DTO 的对应字段为带描述的 string；这里保留字面量联合以维持类型安全。
export type ArticleStatus = 'draft' | 'published' | 'recycled'
export type CommentStatus = 'pending' | 'approved' | 'rejected' | 'spam' | 'recycled'
export type ArticleStatusCommand = Schema['ArticleStatusCommand']
export type ArticleResponse = Schema['ArticleResponse']
export type ArticlePageResponse = Schema['ArticlePageResponse']
export type ArticleCategoryResponse = Schema['CategoryResponse']
export type ArticleTagResponse = Schema['TagResponse']
export type ArticleRevisionResponse = Schema['RevisionResponse']
export type CreateArticleRequest = Schema['CreateArticleRequest']
export type UpdateArticleRequest = Schema['UpdateArticleRequest']
export type CommentTargetType = Schema['CommentResponse']['target_type']
export type CommentResponse = Schema['CommentResponse']
export type CommentPageResponse = Schema['CommentPageResponse']

export type MediaProvider = Schema['MediaAssetResponse']['provider']
export type MediaAssetStatus = Schema['MediaAssetResponse']['status']
export type MediaAssetResponse = Schema['MediaAssetResponse']
export type MediaPageResponse = Schema['MediaPageResponse']
export type MediaUsageTargetType = Schema['MediaUsageResponse']['target_type']
export type MediaUsageResponse = Schema['MediaUsageResponse']
export type MediaBatchDeleteResult = Schema['BatchDeleteMediaResponse']
export type ImportPreviewItem = Schema['ImportPreview']
export type ImportJobResponse = Schema['ImportJobResponse']
export type CommitImportResult = Schema['CommitResultResponse']
export type CommitImportRequest = Schema['CommitImportRequest']
export type UpdateMediaRequest = Schema['UpdateMediaRequest']
