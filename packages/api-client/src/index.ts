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
export type PublicCategory = Schema['PublicCategory']
export type PublicTag = Schema['PublicTag']
export type PublicArchiveMonth = Schema['PublicArchiveMonth']
export type PublicPageDetail = Schema['PublicPageDetail']
export type PublicJournalItem = Schema['PublicJournalItem']
export type PublicJournalPage = Schema['PublicJournalPageResponse']
export type PublicGallerySummary = Schema['PublicGallerySummary']
export type PublicGalleryPage = Schema['PublicGalleryPageResponse']
export type PublicGalleryItem = Schema['PublicGalleryItem']
export type PublicGalleryDetail = Schema['PublicGalleryDetail']
export type PublicPhoto = Schema['PublicPhoto']
export type PublicLink = Schema['PublicLink']
export type PublicNavigationNode = Schema['PublicNavigationNode']
export type PublicSearchSuggestion = Schema['PublicSearchSuggestion']
export type PublicSearchAskRequest = Schema['PublicSearchAskRequest']
export type RelatedArticle = Schema['RelatedArticle']
/** 分类/标签引用：openapi.yaml 中内联于 PublicArticleDetail.category，取其对象形态 */
export type PublicTaxonomyRef = NonNullable<Schema['PublicArticleDetail']['category']>
/** 上一篇/下一篇引用：openapi.yaml 中内联于 PublicArticleDetail.previous/next */
export type PublicArticleNeighbor = NonNullable<Schema['PublicArticleDetail']['previous']>
export type PublicComment = Schema['PublicComment']
export type PublicCommentCreated = Schema['PublicCommentCreated']
export type PublicCommentPage = Schema['PublicCommentPageResponse']
export type CreatePublicCommentRequest = Schema['CreatePublicCommentRequest']
export type ReplyPublicCommentRequest = Schema['ReplyPublicCommentRequest']

// ---- Admin（apps/admin）核心模块类型 ----
export type ArticleStatus = Schema['ArticleStatus']
export type ArticleStatusCommand = Schema['ArticleStatusCommand']
export type ArticleResponse = Schema['ArticleResponse']
export type ArticlePageResponse = Schema['ArticlePageResponse']
export type ArticleCategoryResponse = Schema['ArticleCategoryResponse']
export type ArticleTagResponse = Schema['ArticleTagResponse']
export type ArticleRevisionResponse = Schema['ArticleRevisionResponse']
export type CreateArticleRequest = Schema['CreateArticleRequest']
export type UpdateArticleRequest = Schema['UpdateArticleRequest']
export type CommentStatus = Schema['CommentStatus']
export type CommentTargetType = Schema['CommentResponse']['target_type']
export type CommentResponse = Schema['CommentResponse']
export type CommentPageResponse = Schema['CommentPageResponse']

export type MediaProvider = Schema['MediaAssetResponse']['provider']
export type MediaAssetStatus = Schema['MediaAssetResponse']['status']
export type MediaAssetResponse = Schema['MediaAssetResponse']
export type MediaPageResponse = Schema['MediaPageResponse']
export type MediaUsageTargetType = Schema['MediaUsageResponse']['target_type']
export type MediaUsageResponse = Schema['MediaUsageResponse']
export type MediaBatchDeleteResult = Schema['MediaBatchDeleteResult']
export type ImportPreviewItem = Schema['ImportPreviewItem']
export type ImportJobResponse = Schema['ImportJobResponse']
export type CommitImportResult = Schema['CommitImportResult']
export type CommitImportRequest = Schema['CommitImportRequest']
export type UpdateMediaRequest = Schema['UpdateMediaRequest']
