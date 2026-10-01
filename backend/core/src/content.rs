use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArticleStatus {
    Draft,
    Published,
    Recycled,
}

impl ArticleStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Recycled => "recycled",
        }
    }

    /// 状态迁移必须走明确 Command，避免客户端直接写入任意状态。
    pub const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Draft, Self::Published | Self::Recycled)
                | (Self::Published, Self::Recycled)
                | (Self::Recycled, Self::Draft)
        )
    }
}

impl fmt::Display for ArticleStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ArticleStatus {
    type Err = ContentError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "published" => Ok(Self::Published),
            "recycled" => Ok(Self::Recycled),
            _ => Err(ContentError::InvalidStatus),
        }
    }
}

/// 分类用途：同一物理表按 kind 隔离文章/友链/图库三套分类体系，
/// 唯一约束为 (kind, slug)，跨 kind 允许同名 slug。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategoryKind {
    Article,
    Link,
    Gallery,
}

impl CategoryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Article => "article",
            Self::Link => "link",
            Self::Gallery => "gallery",
        }
    }
}

impl fmt::Display for CategoryKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CategoryKind {
    type Err = ContentError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "article" => Ok(Self::Article),
            "link" => Ok(Self::Link),
            "gallery" => Ok(Self::Gallery),
            _ => Err(ContentError::InvalidCategoryKind),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Article {
    pub id: i64,
    pub author_id: i64,
    pub category_id: Option<i64>,
    pub status: ArticleStatus,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub ai_brief: Option<String>,
    pub cover_url: Option<String>,
    pub markdown_source: String,
    pub rendered_html: String,
    pub seo_keywords: Vec<String>,
    pub tag_ids: Vec<i64>,
    pub password_protected: bool,
    pub allow_comments: bool,
    pub is_pinned: bool,
    pub version: i64,
    pub visit_count: i64,
    pub comment_count: i64,
    pub published_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct NewArticle {
    pub author_id: i64,
    pub category_id: Option<i64>,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub ai_brief: Option<String>,
    pub cover_url: Option<String>,
    pub markdown_source: String,
    pub rendered_html: String,
    pub seo_keywords: Vec<String>,
    pub access_password_hash: Option<String>,
    pub allow_comments: bool,
    pub is_pinned: bool,
    pub tag_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct ArticleUpdate {
    pub category_id: Option<i64>,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub ai_brief: Option<String>,
    pub cover_url: Option<String>,
    pub markdown_source: String,
    pub rendered_html: String,
    pub seo_keywords: Vec<String>,
    /// 三层语义：`None` 不改动密码，`Some(None)` 清除密码，`Some(Some(hash))` 设置新密码。
    pub access_password_hash: Option<Option<String>>,
    pub allow_comments: bool,
    pub is_pinned: bool,
    pub tag_ids: Vec<i64>,
    pub expected_version: i64,
    pub operator_id: i64,
}

/// 列表排序字段白名单，避免把客户端输入直接拼进 SQL。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArticleSort {
    #[default]
    UpdatedAt,
    CreatedAt,
    PublishedAt,
    Title,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

#[derive(Debug, Clone, Default)]
pub struct ArticleListQuery {
    pub page: u32,
    pub page_size: u32,
    pub keyword: Option<String>,
    pub status: Option<ArticleStatus>,
    pub category_id: Option<i64>,
    pub tag_id: Option<i64>,
    pub sort: ArticleSort,
    pub order: SortOrder,
}

#[derive(Debug, Clone)]
pub struct ArticlePage {
    pub items: Vec<Article>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// 公开搜索命中项：文章 + 正文命中片段。
/// 摘要已含关键词时 `snippet` 为 `None`（调用方直接高亮摘要即可）；
/// 仅当命中发生在正文且摘要未覆盖时才生成片段，见 `search::build_snippet`。
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub article: Article,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchPage {
    pub items: Vec<SearchHit>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// 搜索建议：仅 Slug 与 Title，供输入即搜下拉使用，不泄露其他字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchSuggestion {
    pub slug: String,
    pub title: String,
}

/// 公开文章列表查询：状态恒为 Published，排序固定为
/// `is_pinned DESC, sort_order ASC, published_at DESC, id DESC`（对齐旧版首页语义），
/// 客户端只能选分页与过滤条件。
#[derive(Debug, Clone, Default)]
pub struct PublicArticleQuery {
    pub page: u32,
    pub page_size: u32,
    pub keyword: Option<String>,
    pub category_id: Option<i64>,
    pub tag_id: Option<i64>,
    pub category_slug: Option<String>,
    pub tag_slug: Option<String>,
}

/// 公开详情页的文章导航：仅 Slug 与 Title，不泄露邻居文章的其他字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleNeighbor {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone, Default)]
pub struct ArticleNeighbors {
    /// 排序中靠前（更新）的一篇。
    pub previous: Option<ArticleNeighbor>,
    /// 排序中靠后（更旧）的一篇。
    pub next: Option<ArticleNeighbor>,
}

/// 公开分类/标签摘要：文章数为实时 Count，不信任手工维护字段。
#[derive(Debug, Clone)]
pub struct CategorySummary {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub article_count: i64,
}

#[derive(Debug, Clone)]
pub struct TagSummary {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub article_count: i64,
}

#[derive(Debug, Clone)]
pub struct ArchiveArticle {
    pub slug: String,
    pub title: String,
    pub published_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct ArchiveMonth {
    pub year: i32,
    pub month: i32,
    pub count: i64,
    pub articles: Vec<ArchiveArticle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub parent_id: Option<i64>,
    /// 分类用途：article / link / gallery，与数据库 CHECK 一一对应。
    pub kind: CategoryKind,
    pub name: String,
    pub slug: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct NewCategory {
    pub parent_id: Option<i64>,
    pub kind: CategoryKind,
    pub name: String,
    pub slug: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone)]
pub struct NewTag {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone)]
pub struct CategoryUpdate {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone)]
pub struct TagUpdate {
    pub name: String,
    pub slug: String,
}

/// 修订快照中的元数据子集：只保留恢复时需要回写的字段，
/// `status`/`published_at` 属于生命周期状态，不随恢复回滚。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionMetadata {
    pub title: String,
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub ai_brief: Option<String>,
    pub category_id: Option<i64>,
    pub cover_url: Option<String>,
    #[serde(default)]
    pub seo_keywords: Vec<String>,
    pub access_password_hash: Option<String>,
    pub allow_comments: bool,
    pub is_pinned: bool,
    #[serde(default)]
    pub tag_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct ArticleRevision {
    pub article_id: i64,
    pub revision_no: i64,
    pub markdown_source: String,
    pub metadata: RevisionMetadata,
    pub operator_id: i64,
    pub created_at: OffsetDateTime,
}

/// 恢复修订的入参；`rendered_html` 由调用方（HTTP 层）渲染后传入，
/// 与 update 流程保持一致——Markdown 渲染不属于 Repository 职责。
#[derive(Debug, Clone)]
pub struct RevisionRestore {
    pub revision_no: i64,
    pub rendered_html: String,
    pub expected_version: i64,
    pub operator_id: i64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ContentError {
    #[error("invalid article status")]
    InvalidStatus,
    #[error("invalid article state transition")]
    InvalidTransition,
    #[error("article title is too long")]
    InvalidTitle,
    #[error("article slug must contain 1 to 160 characters")]
    InvalidSlug,
    #[error("article summary is too long")]
    InvalidSummary,
    #[error("taxonomy name must contain 1 to 100 characters")]
    InvalidTaxonomyName,
    #[error("published article content is incomplete")]
    PublishValidation,
    #[error("invalid category kind")]
    InvalidCategoryKind,
    #[error("resource conflict")]
    Conflict,
    #[error("resource not found")]
    NotFound,
    #[error("article revision not found")]
    RevisionNotFound,
    /// 快照数据损坏属于内部数据问题，不向客户端暴露细节。
    #[error("article revision snapshot is invalid")]
    InvalidRevisionSnapshot,
    #[error("article must be recycled before deletion")]
    NotRecycled,
    /// 删除被引用的 Taxonomy 时携带引用数，便于前端提示具体影响范围。
    #[error("resource is still referenced")]
    Referenced { count: i64 },
    #[error("content store unavailable")]
    StoreUnavailable,
    #[error("markdown rendering failed")]
    RenderFailed,
}

pub fn normalize_slug(value: &str) -> Result<String, ContentError> {
    let mut normalized = String::with_capacity(value.len());
    let mut pending_separator = false;

    for character in value.trim().chars() {
        if character.is_alphanumeric() {
            if pending_separator && !normalized.is_empty() {
                normalized.push('-');
            }
            for lowercase in character.to_lowercase() {
                normalized.push(lowercase);
            }
            pending_separator = false;
        } else if matches!(character, '-' | '_' | ' ') || character.is_whitespace() {
            pending_separator = true;
        }
    }

    if !(1..=160).contains(&normalized.chars().count()) {
        return Err(ContentError::InvalidSlug);
    }
    Ok(normalized)
}

pub fn validate_draft(title: &str, summary: &str, slug: &str) -> Result<(), ContentError> {
    if title.chars().count() > 200 {
        return Err(ContentError::InvalidTitle);
    }
    if summary.chars().count() > 500 {
        return Err(ContentError::InvalidSummary);
    }
    normalize_slug(slug)?;
    Ok(())
}

pub fn validate_taxonomy_name(value: &str) -> Result<String, ContentError> {
    let value = value.trim();
    if !(1..=100).contains(&value.chars().count()) {
        return Err(ContentError::InvalidTaxonomyName);
    }
    Ok(value.to_owned())
}

pub fn validate_publish(article: &Article) -> Result<(), ContentError> {
    validate_draft(&article.title, &article.summary, &article.slug)?;
    if article.title.trim().is_empty()
        || article.markdown_source.trim().is_empty()
        || article.rendered_html.trim().is_empty()
    {
        return Err(ContentError::PublishValidation);
    }
    Ok(())
}

pub trait MarkdownRenderer: Send + Sync {
    fn render(&self, source: &str) -> Result<String, ContentError>;
}

#[async_trait]
pub trait ContentRepository: Send + Sync {
    async fn create_article(&self, article: NewArticle) -> Result<Article, ContentError>;
    async fn find_article(&self, article_id: i64) -> Result<Option<Article>, ContentError>;
    /// Public 站按 Slug 读取已发布文章；草稿与回收站内容对外不可见。
    async fn find_published_article_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<Article>, ContentError>;
    /// 管理端按 Slug 查找任意状态文章（排除已物理删除），
    /// 用于导入预览的 Slug 冲突检测；Slug 唯一约束覆盖所有未删除行。
    async fn find_article_by_slug(&self, slug: &str) -> Result<Option<Article>, ContentError>;
    async fn list_articles(&self, query: ArticleListQuery) -> Result<ArticlePage, ContentError>;
    async fn update_article(
        &self,
        article_id: i64,
        update: ArticleUpdate,
    ) -> Result<Article, ContentError>;
    async fn transition_article(
        &self,
        article_id: i64,
        expected_version: i64,
        target: ArticleStatus,
        operator_id: i64,
    ) -> Result<Article, ContentError>;
    /// 仅允许物理删除 `recycled` 状态的文章，其他状态返回 `NotRecycled`。
    async fn delete_article(&self, article_id: i64) -> Result<(), ContentError>;
    async fn list_revisions(&self, article_id: i64) -> Result<Vec<ArticleRevision>, ContentError>;
    async fn find_revision(
        &self,
        article_id: i64,
        revision_no: i64,
    ) -> Result<Option<ArticleRevision>, ContentError>;
    async fn restore_revision(
        &self,
        article_id: i64,
        restore: RevisionRestore,
    ) -> Result<Article, ContentError>;
    // ---- Public 只读查询：以下方法只返回已发布内容，供匿名端点使用 ----

    /// 公开文章列表：固定置顶排序与 Published 过滤，见 `PublicArticleQuery`。
    async fn list_public_articles(
        &self,
        query: PublicArticleQuery,
    ) -> Result<ArticlePage, ContentError>;
    /// 公开全文搜索：FTS（`simple` 分词）与 ILIKE 兜底取并集，只命中 Published；
    /// 按相关度排序（标题命中 > 摘要命中 > 正文命中，同档按发布时间倒序）。
    async fn search_public_articles(
        &self,
        keyword: &str,
        page: u32,
        page_size: u32,
    ) -> Result<SearchPage, ContentError>;
    /// 搜索建议：标题/摘要包含匹配，前缀优先，按发布时间倒序取前 `limit` 条，只命中 Published。
    async fn search_suggest(
        &self,
        keyword: &str,
        limit: u32,
    ) -> Result<Vec<SearchSuggestion>, ContentError>;
    /// 取同一公开排序下目标文章的上一篇/下一篇；目标必须已发布。
    async fn public_article_neighbors(
        &self,
        article_id: i64,
    ) -> Result<ArticleNeighbors, ContentError>;
    /// 文章的访问密码 Hash，仅用于公开解锁端点校验；不出现在任何响应中。
    async fn article_password_hash(&self, article_id: i64) -> Result<Option<String>, ContentError>;
    /// 浏览量 +1 并返回新值；去重由 HTTP 层滑动窗口负责。
    async fn increment_visit_count(&self, article_id: i64) -> Result<i64, ContentError>;
    /// 公开分类列表（kind = article），带实时 Published 文章数。
    async fn list_public_categories(&self) -> Result<Vec<CategorySummary>, ContentError>;
    /// 公开标签列表，带实时 Published 文章数。
    async fn list_public_tags(&self) -> Result<Vec<TagSummary>, ContentError>;
    /// 按 Slug 查找公开分类（未删除）；slug 仅在同一 kind 内唯一，必须带 kind。
    async fn find_category_by_slug(
        &self,
        kind: CategoryKind,
        slug: &str,
    ) -> Result<Option<Category>, ContentError>;
    /// 按 Slug 查找标签。
    async fn find_tag_by_slug(&self, slug: &str) -> Result<Option<Tag>, ContentError>;
    /// 按 ID 查找分类，供公开详情页投影分类信息。
    async fn find_category(&self, category_id: i64) -> Result<Option<Category>, ContentError>;
    /// 文章关联的标签列表，供公开详情页投影。
    async fn list_tags_of_article(&self, article_id: i64) -> Result<Vec<Tag>, ContentError>;
    /// 归档聚合：Published 文章按 year/month 分组，组内按发布时间倒序。
    async fn list_public_archives(&self) -> Result<Vec<ArchiveMonth>, ContentError>;

    async fn list_categories(&self, kind: CategoryKind) -> Result<Vec<Category>, ContentError>;
    async fn create_category(&self, category: NewCategory) -> Result<Category, ContentError>;
    async fn update_category(
        &self,
        category_id: i64,
        kind: CategoryKind,
        update: CategoryUpdate,
    ) -> Result<Category, ContentError>;
    async fn delete_category(
        &self,
        category_id: i64,
        kind: CategoryKind,
    ) -> Result<(), ContentError>;
    async fn list_tags(&self) -> Result<Vec<Tag>, ContentError>;
    async fn create_tag(&self, tag: NewTag) -> Result<Tag, ContentError>;
    async fn update_tag(&self, tag_id: i64, update: TagUpdate) -> Result<Tag, ContentError>;
    async fn delete_tag(&self, tag_id: i64) -> Result<(), ContentError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_normalization_is_stable_and_keeps_unicode_letters() {
        assert_eq!(
            normalize_slug("  Rust 与 Vue_3  "),
            Ok("rust-与-vue-3".to_owned())
        );
        assert_eq!(normalize_slug("---"), Err(ContentError::InvalidSlug));
    }

    #[test]
    fn article_state_machine_rejects_implicit_republish() {
        assert!(ArticleStatus::Draft.can_transition_to(ArticleStatus::Published));
        assert!(ArticleStatus::Published.can_transition_to(ArticleStatus::Recycled));
        assert!(ArticleStatus::Recycled.can_transition_to(ArticleStatus::Draft));
        assert!(!ArticleStatus::Recycled.can_transition_to(ArticleStatus::Published));
        assert!(!ArticleStatus::Published.can_transition_to(ArticleStatus::Draft));
    }

    #[test]
    fn article_state_machine_rejects_self_transitions() {
        // 草稿可以直接进回收站（不发布即废弃）。
        assert!(ArticleStatus::Draft.can_transition_to(ArticleStatus::Recycled));
        // 任何状态都不允许原地迁移，版本号语义只跟随真实变更。
        for status in [
            ArticleStatus::Draft,
            ArticleStatus::Published,
            ArticleStatus::Recycled,
        ] {
            assert!(!status.can_transition_to(status));
        }
    }

    #[test]
    fn article_status_round_trips_through_str() {
        for status in [
            ArticleStatus::Draft,
            ArticleStatus::Published,
            ArticleStatus::Recycled,
        ] {
            assert_eq!(status.as_str().parse::<ArticleStatus>(), Ok(status));
            assert_eq!(status.to_string(), status.as_str());
        }
        // 未知值与大小写差异一律拒绝（数据库 CHECK 之外的第一道防线）。
        assert_eq!(
            "deleted".parse::<ArticleStatus>(),
            Err(ContentError::InvalidStatus)
        );
        assert_eq!(
            "Published".parse::<ArticleStatus>(),
            Err(ContentError::InvalidStatus)
        );
    }

    #[test]
    fn category_kind_round_trips_through_str() {
        for kind in [
            CategoryKind::Article,
            CategoryKind::Link,
            CategoryKind::Gallery,
        ] {
            assert_eq!(kind.as_str().parse::<CategoryKind>(), Ok(kind));
            assert_eq!(kind.to_string(), kind.as_str());
        }
        assert_eq!(
            "tag".parse::<CategoryKind>(),
            Err(ContentError::InvalidCategoryKind)
        );
    }

    #[test]
    fn slug_normalization_collapses_separators_and_enforces_length() {
        // 连续分隔符合并为一个连字符，首尾分隔符不产生多余连字符。
        assert_eq!(
            normalize_slug(" - Hello__World 2026 - "),
            Ok("hello-world-2026".to_owned())
        );
        // 任意空白字符同样视为分隔符。
        assert_eq!(normalize_slug("a\tb\nc"), Ok("a-b-c".to_owned()));
        // 只有分隔符/空白 → 归一化为空 → 拒绝。
        assert_eq!(normalize_slug("   "), Err(ContentError::InvalidSlug));
        // 长度上限 160 字符，按字符数而非字节数计算。
        assert!(normalize_slug(&"a".repeat(160)).is_ok());
        assert_eq!(
            normalize_slug(&"a".repeat(161)),
            Err(ContentError::InvalidSlug)
        );
    }

    #[test]
    fn draft_validation_enforces_title_summary_and_slug_limits() {
        // 标题 200 字符通过、201 拒绝；按字符数而非字节数。
        assert!(validate_draft(&"题".repeat(200), "", "ok-slug").is_ok());
        assert_eq!(
            validate_draft(&"题".repeat(201), "", "ok-slug"),
            Err(ContentError::InvalidTitle)
        );
        // 摘要 500 字符通过、501 拒绝。
        assert!(validate_draft("t", &"摘".repeat(500), "ok-slug").is_ok());
        assert_eq!(
            validate_draft("t", &"摘".repeat(501), "ok-slug"),
            Err(ContentError::InvalidSummary)
        );
        // slug 校验失败原样透传。
        assert_eq!(
            validate_draft("t", "", "!!"),
            Err(ContentError::InvalidSlug)
        );
    }

    #[test]
    fn taxonomy_name_is_trimmed_and_length_limited() {
        // 首尾空白裁剪后返回。
        assert_eq!(validate_taxonomy_name("  后端  "), Ok("后端".to_owned()));
        // 裁剪后为空或超过 100 字符拒绝。
        assert_eq!(
            validate_taxonomy_name("   "),
            Err(ContentError::InvalidTaxonomyName)
        );
        assert!(validate_taxonomy_name(&"类".repeat(100)).is_ok());
        assert_eq!(
            validate_taxonomy_name(&"类".repeat(101)),
            Err(ContentError::InvalidTaxonomyName)
        );
    }

    #[test]
    fn publish_validation_requires_visible_content() {
        let now = OffsetDateTime::now_utc();
        let mut article = Article {
            id: 1,
            author_id: 1,
            category_id: None,
            status: ArticleStatus::Draft,
            slug: "first-post".to_owned(),
            title: "First post".to_owned(),
            summary: String::new(),
            ai_brief: None,
            cover_url: None,
            markdown_source: "# First post".to_owned(),
            rendered_html: "<h1>First post</h1>".to_owned(),
            seo_keywords: Vec::new(),
            tag_ids: Vec::new(),
            password_protected: false,
            allow_comments: true,
            is_pinned: false,
            version: 1,
            visit_count: 0,
            comment_count: 0,
            published_at: None,
            created_at: now,
            updated_at: now,
        };
        assert!(validate_publish(&article).is_ok());

        article.rendered_html.clear();
        assert_eq!(
            validate_publish(&article),
            Err(ContentError::PublishValidation)
        );
    }

    #[test]
    fn publish_validation_rejects_blank_fields_and_propagates_draft_errors() {
        let now = OffsetDateTime::now_utc();
        let base = Article {
            id: 1,
            author_id: 1,
            category_id: None,
            status: ArticleStatus::Draft,
            slug: "first-post".to_owned(),
            title: "First post".to_owned(),
            summary: String::new(),
            ai_brief: None,
            cover_url: None,
            markdown_source: "# First post".to_owned(),
            rendered_html: "<h1>First post</h1>".to_owned(),
            seo_keywords: Vec::new(),
            tag_ids: Vec::new(),
            password_protected: false,
            allow_comments: true,
            is_pinned: false,
            version: 1,
            visit_count: 0,
            comment_count: 0,
            published_at: None,
            created_at: now,
            updated_at: now,
        };

        // 标题或 Markdown 仅为空白字符时同样视为内容不完整。
        let mut article = base.clone();
        article.title = "   ".to_owned();
        assert_eq!(
            validate_publish(&article),
            Err(ContentError::PublishValidation)
        );
        let mut article = base.clone();
        article.markdown_source = "\n".to_owned();
        assert_eq!(
            validate_publish(&article),
            Err(ContentError::PublishValidation)
        );

        // 草稿校验失败优先于发布完整性校验：标题超长透传 InvalidTitle。
        let mut article = base;
        article.title = "题".repeat(201);
        assert_eq!(validate_publish(&article), Err(ContentError::InvalidTitle));
    }
}
