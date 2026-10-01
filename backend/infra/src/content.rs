use std::str::FromStr;

use crate::like::{like_pattern, prefix_pattern};
use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use aries_core::content::{
    ArchiveArticle, ArchiveMonth, Article, ArticleListQuery, ArticleNeighbor, ArticleNeighbors,
    ArticlePage, ArticleRevision, ArticleSort, ArticleStatus, ArticleUpdate, Category,
    CategoryKind, CategorySummary, CategoryUpdate, ContentError, ContentRepository, NewArticle,
    NewCategory, NewTag, PublicArticleQuery, RevisionMetadata, RevisionRestore, SearchHit,
    SearchPage, SearchSuggestion, SortOrder, Tag, TagSummary, TagUpdate, normalize_slug,
    validate_draft, validate_publish, validate_taxonomy_name,
};
use aries_core::search;
use async_trait::async_trait;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresContentRepository {
    pool: PgPool,
}

impl PostgresContentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, FromRow)]
struct ArticleRow {
    id: i64,
    author_id: i64,
    category_id: Option<i64>,
    status: String,
    slug: String,
    title: String,
    summary: String,
    ai_brief: Option<String>,
    cover_url: Option<String>,
    markdown_source: String,
    rendered_html: String,
    seo_keywords: Vec<String>,
    tag_ids: Vec<i64>,
    password_protected: bool,
    allow_comments: bool,
    is_pinned: bool,
    version: i64,
    visit_count: i64,
    comment_count: i64,
    published_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<ArticleRow> for Article {
    type Error = ContentError;

    fn try_from(row: ArticleRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            author_id: row.author_id,
            category_id: row.category_id,
            status: ArticleStatus::from_str(&row.status)?,
            slug: row.slug,
            title: row.title,
            summary: row.summary,
            ai_brief: row.ai_brief,
            cover_url: row.cover_url,
            markdown_source: row.markdown_source,
            rendered_html: row.rendered_html,
            seo_keywords: row.seo_keywords,
            tag_ids: row.tag_ids,
            password_protected: row.password_protected,
            allow_comments: row.allow_comments,
            is_pinned: row.is_pinned,
            version: row.version,
            visit_count: row.visit_count,
            comment_count: row.comment_count,
            published_at: row.published_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const ARTICLE_COLUMNS: &str = "id, author_id, category_id, status, slug::text AS slug, title, \
    summary, ai_brief, cover_url, markdown_source, rendered_html, seo_keywords, \
    COALESCE((SELECT array_agg(tag_id ORDER BY tag_id) FROM article_tags \
        WHERE article_id = articles.id), ARRAY[]::bigint[]) AS tag_ids, \
    access_password_hash IS NOT NULL AS password_protected, allow_comments, is_pinned, version, \
    visit_count, comment_count, published_at, created_at, updated_at";

#[derive(Debug, FromRow)]
struct CategoryRow {
    id: i64,
    parent_id: Option<i64>,
    kind: String,
    name: String,
    slug: String,
    description: String,
}

impl TryFrom<CategoryRow> for Category {
    type Error = ContentError;

    fn try_from(row: CategoryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            parent_id: row.parent_id,
            kind: row.kind.parse()?,
            name: row.name,
            slug: row.slug,
            description: row.description,
        })
    }
}

#[derive(Debug, FromRow)]
struct TagRow {
    id: i64,
    name: String,
    slug: String,
}

impl From<TagRow> for Tag {
    fn from(row: TagRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            slug: row.slug,
        }
    }
}

#[derive(Debug, FromRow)]
struct RevisionRow {
    article_id: i64,
    revision_no: i64,
    markdown_source: String,
    metadata_snapshot: serde_json::Value,
    operator_id: i64,
    created_at: OffsetDateTime,
}

impl TryFrom<RevisionRow> for ArticleRevision {
    type Error = ContentError;

    fn try_from(row: RevisionRow) -> Result<Self, Self::Error> {
        let metadata: RevisionMetadata = serde_json::from_value(row.metadata_snapshot)
            .map_err(|_| ContentError::InvalidRevisionSnapshot)?;
        Ok(Self {
            article_id: row.article_id,
            revision_no: row.revision_no,
            markdown_source: row.markdown_source,
            metadata,
            operator_id: row.operator_id,
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl ContentRepository for PostgresContentRepository {
    async fn create_article(&self, mut article: NewArticle) -> Result<Article, ContentError> {
        article.slug = normalize_slug(&article.slug)?;
        validate_draft(&article.title, &article.summary, &article.slug)?;
        normalize_tag_ids(&mut article.tag_ids);

        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        validate_category(&mut transaction, article.category_id).await?;
        let query = format!(
            "INSERT INTO articles (author_id, category_id, slug, title, summary, ai_brief, cover_url, \
             markdown_source, rendered_html, seo_keywords, access_password_hash, allow_comments, \
             is_pinned) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             RETURNING {ARTICLE_COLUMNS}"
        );
        let row = logged_query_as::<ArticleRow>(&query)
            .bind(article.author_id)
            .bind(article.category_id)
            .bind(article.slug)
            .bind(article.title)
            .bind(article.summary)
            .bind(article.ai_brief)
            .bind(article.cover_url)
            .bind(article.markdown_source)
            .bind(article.rendered_html)
            .bind(article.seo_keywords)
            .bind(article.access_password_hash)
            .bind(article.allow_comments)
            .bind(article.is_pinned)
            .fetch_one(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        sync_tags(&mut transaction, row.id, &article.tag_ids).await?;
        let article = lock_article(&mut transaction, row.id).await?;
        transaction.commit().await.map_err(map_sqlx)?;
        Ok(article)
    }

    async fn find_article(&self, article_id: i64) -> Result<Option<Article>, ContentError> {
        let query =
            format!("SELECT {ARTICLE_COLUMNS} FROM articles WHERE id = $1 AND deleted_at IS NULL");
        let row = logged_query_as::<ArticleRow>(&query)
            .bind(article_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn find_published_article_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<Article>, ContentError> {
        // 绑定参数按 text 传参，显式 cast 回 citext 以保留大小写不敏感语义并命中索引。
        let query = format!(
            "SELECT {ARTICLE_COLUMNS} FROM articles \
             WHERE slug = $1::citext AND status = 'published' AND deleted_at IS NULL"
        );
        let row = logged_query_as::<ArticleRow>(&query)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn find_article_by_slug(&self, slug: &str) -> Result<Option<Article>, ContentError> {
        // 与 find_published_article_by_slug 的 cast 一致，保留 citext 大小写不敏感语义。
        let query = format!(
            "SELECT {ARTICLE_COLUMNS} FROM articles \
             WHERE slug = $1::citext AND deleted_at IS NULL"
        );
        let row = logged_query_as::<ArticleRow>(&query)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn list_articles(&self, query: ArticleListQuery) -> Result<ArticlePage, ContentError> {
        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, 100);
        let offset = i64::from(page - 1) * i64::from(page_size);
        let status = query.status.map(|value| value.as_str().to_owned());
        let keyword = query
            .keyword
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|value| like_pattern(&value));

        let total = logged_query_scalar::<i64>(
            "SELECT count(*) FROM articles WHERE deleted_at IS NULL \
             AND ($1::text IS NULL OR status = $1) \
             AND ($2::bigint IS NULL OR category_id = $2) \
             AND ($3::bigint IS NULL OR EXISTS (SELECT 1 FROM article_tags \
                 WHERE article_id = articles.id AND tag_id = $3)) \
             AND ($4::text IS NULL OR title ILIKE $4 ESCAPE '\\' \
                 OR slug::text ILIKE $4 ESCAPE '\\')",
        )
        .bind(status.as_deref())
        .bind(query.category_id)
        .bind(query.tag_id)
        .bind(keyword.as_deref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        // 排序字段只从白名单枚举映射到固定 SQL 片段，客户端输入绝不直接拼接。
        let sort_column = match query.sort {
            ArticleSort::UpdatedAt => "updated_at",
            ArticleSort::CreatedAt => "created_at",
            ArticleSort::PublishedAt => "published_at",
            ArticleSort::Title => "title",
        };
        let sort_direction = match query.order {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };
        let list_query = format!(
            "SELECT {ARTICLE_COLUMNS} FROM articles WHERE deleted_at IS NULL \
             AND ($1::text IS NULL OR status = $1) \
             AND ($2::bigint IS NULL OR category_id = $2) \
             AND ($3::bigint IS NULL OR EXISTS (SELECT 1 FROM article_tags \
                 WHERE article_id = articles.id AND tag_id = $3)) \
             AND ($4::text IS NULL OR title ILIKE $4 ESCAPE '\\' \
                 OR slug::text ILIKE $4 ESCAPE '\\') \
             ORDER BY {sort_column} {sort_direction}, id DESC LIMIT $5 OFFSET $6"
        );
        let rows = logged_query_as::<ArticleRow>(&list_query)
            .bind(status.as_deref())
            .bind(query.category_id)
            .bind(query.tag_id)
            .bind(keyword.as_deref())
            .bind(i64::from(page_size))
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let items = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ArticlePage {
            items,
            total,
            page,
            page_size,
        })
    }

    /// 公开列表固定排序：置顶优先，其次手工排序值，再按发布时间倒序；`id DESC` 兜底稳定。
    /// 过滤条件同时支持 ID 与 Slug 两种形态，Slug 走 citext 索引保持大小写不敏感。
    async fn list_public_articles(
        &self,
        query: PublicArticleQuery,
    ) -> Result<ArticlePage, ContentError> {
        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, 100);
        let offset = i64::from(page - 1) * i64::from(page_size);
        let keyword = query
            .keyword
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|value| like_pattern(&value));

        const FILTERS: &str = "FROM articles WHERE status = 'published' AND deleted_at IS NULL \
             AND ($1::bigint IS NULL OR category_id = $1) \
             AND ($2::bigint IS NULL OR EXISTS (SELECT 1 FROM article_tags \
                 WHERE article_id = articles.id AND tag_id = $2)) \
             AND ($3::text IS NULL OR title ILIKE $3 ESCAPE '\\') \
             AND ($4::citext IS NULL OR category_id = (SELECT id FROM categories \
                 WHERE slug = $4::citext AND kind = 'article' AND deleted_at IS NULL)) \
             AND ($5::citext IS NULL OR EXISTS (SELECT 1 FROM article_tags \
                 JOIN tags ON tags.id = article_tags.tag_id \
                 WHERE article_tags.article_id = articles.id AND tags.slug = $5::citext))";
        let total = logged_query_scalar::<i64>(&format!("SELECT count(*) {FILTERS}"))
            .bind(query.category_id)
            .bind(query.tag_id)
            .bind(keyword.as_deref())
            .bind(query.category_slug.as_deref())
            .bind(query.tag_slug.as_deref())
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let rows = logged_query_as::<ArticleRow>(&format!(
            "SELECT {ARTICLE_COLUMNS} {FILTERS} \
             ORDER BY is_pinned DESC, sort_order ASC, published_at DESC, id DESC \
             LIMIT $6 OFFSET $7"
        ))
        .bind(query.category_id)
        .bind(query.tag_id)
        .bind(keyword.as_deref())
        .bind(query.category_slug.as_deref())
        .bind(query.tag_slug.as_deref())
        .bind(i64::from(page_size))
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let items = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ArticlePage {
            items,
            total,
            page,
            page_size,
        })
    }

    async fn search_public_articles(
        &self,
        keyword: &str,
        page: u32,
        page_size: u32,
    ) -> Result<SearchPage, ContentError> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 100);
        let offset = i64::from(page - 1) * i64::from(page_size);
        // FTS 用 `simple` 分词；中文场景分词退化为整串匹配，因此 OR ILIKE 兜底标题与摘要。
        // FTS 保留原始 keyword（plainto_tsquery 自带语法）；ILIKE 侧绑定转义后的 like_pattern。
        const FILTERS: &str = "FROM articles WHERE status = 'published' AND deleted_at IS NULL \
             AND (to_tsvector('simple', coalesce(title, '') || ' ' || coalesce(summary, '') \
                 || ' ' || coalesce(markdown_source, '')) @@ plainto_tsquery('simple', $1) \
             OR title ILIKE $2 ESCAPE '\\' OR summary ILIKE $2 ESCAPE '\\')";
        // 相关度排序：标题命中 > 摘要命中 > 仅正文命中，同档按发布时间倒序
        const ORDERING: &str = "ORDER BY CASE WHEN title ILIKE $2 ESCAPE '\\' THEN 0 \
             WHEN summary ILIKE $2 ESCAPE '\\' THEN 1 ELSE 2 END, \
             published_at DESC, id DESC";
        let total = logged_query_scalar::<i64>(&format!("SELECT count(*) {FILTERS}"))
            .bind(keyword)
            .bind(like_pattern(keyword))
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let rows = logged_query_as::<ArticleRow>(&format!(
            "SELECT {ARTICLE_COLUMNS} {FILTERS} {ORDERING} LIMIT $3 OFFSET $4"
        ))
        .bind(keyword)
        .bind(like_pattern(keyword))
        .bind(i64::from(page_size))
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let items = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<Article>, _>>()?
            .into_iter()
            .map(|article| {
                let snippet =
                    search::build_snippet(keyword, &article.summary, &article.markdown_source);
                SearchHit { article, snippet }
            })
            .collect();
        Ok(SearchPage {
            items,
            total,
            page,
            page_size,
        })
    }

    async fn search_suggest(
        &self,
        keyword: &str,
        limit: u32,
    ) -> Result<Vec<SearchSuggestion>, ContentError> {
        let limit = i64::from(limit.clamp(1, 20));
        // 前缀命中优先于包含命中，同档按发布时间倒序
        let rows = logged_query_as::<(String, String)>(
            "SELECT slug::text, title FROM articles \
             WHERE status = 'published' AND deleted_at IS NULL \
                 AND (title ILIKE $1 ESCAPE '\\' OR summary ILIKE $1 ESCAPE '\\') \
             ORDER BY CASE WHEN title ILIKE $2 ESCAPE '\\' THEN 0 ELSE 1 END, \
                 published_at DESC, id DESC LIMIT $3",
        )
        .bind(like_pattern(keyword))
        .bind(prefix_pattern(keyword))
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(rows
            .into_iter()
            .map(|(slug, title)| SearchSuggestion { slug, title })
            .collect())
    }

    async fn public_article_neighbors(
        &self,
        article_id: i64,
    ) -> Result<ArticleNeighbors, ContentError> {
        // 窗口函数在与列表完全一致的排序上取前后邻居，保证详情页导航与列表顺序一致。
        #[derive(FromRow)]
        struct NeighborRow {
            prev_slug: Option<String>,
            prev_title: Option<String>,
            next_slug: Option<String>,
            next_title: Option<String>,
        }
        const ORDER: &str = "ORDER BY is_pinned DESC, sort_order ASC, published_at DESC, id DESC";
        let row = logged_query_as::<NeighborRow>(&format!(
            "SELECT prev_slug, prev_title, next_slug, next_title FROM ( \
                 SELECT id, LAG(slug::text) OVER ({ORDER}) AS prev_slug, \
                     LAG(title) OVER ({ORDER}) AS prev_title, \
                     LEAD(slug::text) OVER ({ORDER}) AS next_slug, \
                     LEAD(title) OVER ({ORDER}) AS next_title \
                 FROM articles WHERE status = 'published' AND deleted_at IS NULL \
             ) neighbors WHERE id = $1"
        ))
        .bind(article_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let neighbor = |slug: Option<String>, title: Option<String>| {
            slug.zip(title)
                .map(|(slug, title)| ArticleNeighbor { slug, title })
        };
        Ok(match row {
            Some(row) => ArticleNeighbors {
                previous: neighbor(row.prev_slug, row.prev_title),
                next: neighbor(row.next_slug, row.next_title),
            },
            None => ArticleNeighbors::default(),
        })
    }

    async fn article_password_hash(&self, article_id: i64) -> Result<Option<String>, ContentError> {
        let hash = logged_query_scalar::<Option<String>>(
            "SELECT access_password_hash FROM articles WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(article_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(ContentError::NotFound)?;
        Ok(hash)
    }

    async fn increment_visit_count(&self, article_id: i64) -> Result<i64, ContentError> {
        // 只允许给已发布文章计数，避免通过 ID 探测草稿存在性。
        let count = logged_query_scalar::<i64>(
            "UPDATE articles SET visit_count = visit_count + 1 \
             WHERE id = $1 AND status = 'published' AND deleted_at IS NULL RETURNING visit_count",
        )
        .bind(article_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(ContentError::NotFound)?;
        Ok(count)
    }

    async fn list_public_categories(&self) -> Result<Vec<CategorySummary>, ContentError> {
        #[derive(FromRow)]
        struct SummaryRow {
            id: i64,
            name: String,
            slug: String,
            description: String,
            article_count: i64,
        }
        // 文章数实时 Count：手工维护的计数字段在并发与回滚场景下不可靠。
        let rows = logged_query_as::<SummaryRow>(
            "SELECT c.id, c.name::text AS name, c.slug::text AS slug, c.description, \
                 (SELECT count(*) FROM articles a WHERE a.category_id = c.id \
                     AND a.status = 'published' AND a.deleted_at IS NULL) AS article_count \
             FROM categories c WHERE c.kind = 'article' AND c.deleted_at IS NULL \
             ORDER BY c.sort_order ASC, c.id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(rows
            .into_iter()
            .map(|row| CategorySummary {
                id: row.id,
                name: row.name,
                slug: row.slug,
                description: row.description,
                article_count: row.article_count,
            })
            .collect())
    }

    async fn list_public_tags(&self) -> Result<Vec<TagSummary>, ContentError> {
        #[derive(FromRow)]
        struct SummaryRow {
            id: i64,
            name: String,
            slug: String,
            article_count: i64,
        }
        let rows = logged_query_as::<SummaryRow>(
            "SELECT t.id, t.name::text AS name, t.slug::text AS slug, \
                 (SELECT count(*) FROM article_tags JOIN articles a ON a.id = article_tags.article_id \
                     WHERE article_tags.tag_id = t.id \
                     AND a.status = 'published' AND a.deleted_at IS NULL) AS article_count \
             FROM tags t ORDER BY t.name, t.id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(rows
            .into_iter()
            .map(|row| TagSummary {
                id: row.id,
                name: row.name,
                slug: row.slug,
                article_count: row.article_count,
            })
            .collect())
    }

    async fn find_category_by_slug(
        &self,
        kind: CategoryKind,
        slug: &str,
    ) -> Result<Option<Category>, ContentError> {
        let row = logged_query_as::<CategoryRow>(
            "SELECT id, parent_id, kind, name::text AS name, slug::text AS slug, description \
             FROM categories WHERE slug = $1::citext AND kind = $2 AND deleted_at IS NULL",
        )
        .bind(slug)
        .bind(kind.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn find_tag_by_slug(&self, slug: &str) -> Result<Option<Tag>, ContentError> {
        let row = logged_query_as::<TagRow>(
            "SELECT id, name::text AS name, slug::text AS slug FROM tags WHERE slug = $1::citext",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(row.map(Into::into))
    }

    async fn find_category(&self, category_id: i64) -> Result<Option<Category>, ContentError> {
        // id 全局唯一，不按 kind 过滤，links/galleries 模块复用此方法解析各自分类。
        let row = logged_query_as::<CategoryRow>(
            "SELECT id, parent_id, kind, name::text AS name, slug::text AS slug, description \
             FROM categories WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(category_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn list_tags_of_article(&self, article_id: i64) -> Result<Vec<Tag>, ContentError> {
        let rows = logged_query_as::<TagRow>(
            "SELECT t.id, t.name::text AS name, t.slug::text AS slug FROM tags t \
             JOIN article_tags ON article_tags.tag_id = t.id \
             WHERE article_tags.article_id = $1 ORDER BY t.name, t.id",
        )
        .bind(article_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_public_archives(&self) -> Result<Vec<ArchiveMonth>, ContentError> {
        #[derive(FromRow)]
        struct ArchiveRow {
            slug: String,
            title: String,
            published_at: OffsetDateTime,
            year: i32,
            month: i32,
        }
        let rows = logged_query_as::<ArchiveRow>(
            "SELECT slug::text AS slug, title, published_at, \
                 EXTRACT(YEAR FROM published_at)::int AS year, \
                 EXTRACT(MONTH FROM published_at)::int AS month \
             FROM articles WHERE status = 'published' AND deleted_at IS NULL \
             ORDER BY year DESC, month DESC, published_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        // 按 (year, month) 保序分组：SQL 已按组排序，顺序扫描即可。
        let mut months: Vec<ArchiveMonth> = Vec::new();
        for row in rows {
            let entry = ArchiveArticle {
                slug: row.slug,
                title: row.title,
                published_at: row.published_at,
            };
            match months.last_mut() {
                Some(month) if month.year == row.year && month.month == row.month => {
                    month.count += 1;
                    month.articles.push(entry);
                }
                _ => months.push(ArchiveMonth {
                    year: row.year,
                    month: row.month,
                    count: 1,
                    articles: vec![entry],
                }),
            }
        }
        Ok(months)
    }

    async fn update_article(
        &self,
        article_id: i64,
        mut update: ArticleUpdate,
    ) -> Result<Article, ContentError> {
        update.slug = normalize_slug(&update.slug)?;
        validate_draft(&update.title, &update.summary, &update.slug)?;
        normalize_tag_ids(&mut update.tag_ids);

        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        validate_category(&mut transaction, update.category_id).await?;
        let mut current = lock_article(&mut transaction, article_id).await?;
        if current.version != update.expected_version {
            return Err(ContentError::Conflict);
        }
        if current.status == ArticleStatus::Published {
            current.slug.clone_from(&update.slug);
            current.title.clone_from(&update.title);
            current.summary.clone_from(&update.summary);
            current.markdown_source.clone_from(&update.markdown_source);
            current.rendered_html.clone_from(&update.rendered_html);
            validate_publish(&current)?;
        }
        record_revision(&mut transaction, article_id, update.operator_id).await?;

        // `None` 表示不改动密码列，SQL 中直接省略该赋值，避免把现有 hash 覆盖成 NULL。
        let set_password = update.access_password_hash.is_some();
        // PostgreSQL 预处理语句的占位符必须从 $1 连续编号，省略密码子句时后续参数前移一位。
        let (password_clause, comments_param, pinned_param) = if set_password {
            ("access_password_hash = $11, ", "$12", "$13")
        } else {
            ("", "$11", "$12")
        };
        let query = format!(
            "UPDATE articles SET category_id = $2, slug = $3, title = $4, summary = $5, \
             ai_brief = $10, cover_url = $6, markdown_source = $7, rendered_html = $8, \
             seo_keywords = $9, {password_clause}allow_comments = {comments_param}, \
             is_pinned = {pinned_param}, version = version + 1, updated_at = now() \
             WHERE id = $1 RETURNING {ARTICLE_COLUMNS}"
        );
        let statement = logged_query_as::<ArticleRow>(&query)
            .bind(article_id)
            .bind(update.category_id)
            .bind(update.slug)
            .bind(update.title)
            .bind(update.summary)
            .bind(update.ai_brief)
            .bind(update.cover_url)
            .bind(update.markdown_source)
            .bind(update.rendered_html)
            .bind(update.seo_keywords);
        // 占位符按绑定顺序编号，密码子句存在时才绑定 $11。
        let statement = if set_password {
            statement.bind(update.access_password_hash.flatten())
        } else {
            statement
        };
        let row = statement
            .bind(update.allow_comments)
            .bind(update.is_pinned)
            .fetch_one(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        sync_tags(&mut transaction, article_id, &update.tag_ids).await?;
        let article = lock_article(&mut transaction, row.id).await?;
        transaction.commit().await.map_err(map_sqlx)?;
        Ok(article)
    }

    async fn transition_article(
        &self,
        article_id: i64,
        expected_version: i64,
        target: ArticleStatus,
        operator_id: i64,
    ) -> Result<Article, ContentError> {
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        let current = lock_article(&mut transaction, article_id).await?;
        if current.version != expected_version {
            return Err(ContentError::Conflict);
        }
        if !current.status.can_transition_to(target) {
            return Err(ContentError::InvalidTransition);
        }
        if target == ArticleStatus::Published {
            validate_publish(&current)?;
        }
        record_revision(&mut transaction, article_id, operator_id).await?;

        let query = format!(
            "UPDATE articles SET status = $2, \
             published_at = CASE WHEN $2 = 'published' THEN COALESCE(published_at, now()) \
                 WHEN $2 = 'draft' THEN NULL ELSE published_at END, \
             version = version + 1, updated_at = now() \
             WHERE id = $1 RETURNING {ARTICLE_COLUMNS}"
        );
        let row = logged_query_as::<ArticleRow>(&query)
            .bind(article_id)
            .bind(target.as_str())
            .fetch_one(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        transaction.commit().await.map_err(map_sqlx)?;
        row.try_into()
    }

    async fn delete_article(&self, article_id: i64) -> Result<(), ContentError> {
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        let current = lock_article(&mut transaction, article_id).await?;
        // 只允许物理删除已回收的文章，防止误删可见内容；其他状态必须先走回收流程。
        if current.status != ArticleStatus::Recycled {
            return Err(ContentError::NotRecycled);
        }
        // 外键虽有 ON DELETE CASCADE，这里显式删除关联数据，保证删除范围清晰可查。
        for statement in [
            "DELETE FROM article_tags WHERE article_id = $1",
            "DELETE FROM article_revisions WHERE article_id = $1",
            "DELETE FROM article_chunks WHERE article_id = $1",
            "DELETE FROM articles WHERE id = $1",
        ] {
            logged_query(statement)
                .bind(article_id)
                .execute(&mut *transaction)
                .await
                .map_err(map_sqlx)?;
        }
        transaction.commit().await.map_err(map_sqlx)
    }

    async fn list_revisions(&self, article_id: i64) -> Result<Vec<ArticleRevision>, ContentError> {
        let rows = logged_query_as::<RevisionRow>(
            "SELECT article_id, revision_no, markdown_source, metadata_snapshot, operator_id, \
             created_at FROM article_revisions WHERE article_id = $1 ORDER BY revision_no DESC",
        )
        .bind(article_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn find_revision(
        &self,
        article_id: i64,
        revision_no: i64,
    ) -> Result<Option<ArticleRevision>, ContentError> {
        let row = logged_query_as::<RevisionRow>(
            "SELECT article_id, revision_no, markdown_source, metadata_snapshot, operator_id, \
             created_at FROM article_revisions WHERE article_id = $1 AND revision_no = $2",
        )
        .bind(article_id)
        .bind(revision_no)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn restore_revision(
        &self,
        article_id: i64,
        restore: RevisionRestore,
    ) -> Result<Article, ContentError> {
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        let mut current = lock_article(&mut transaction, article_id).await?;
        if current.version != restore.expected_version {
            return Err(ContentError::Conflict);
        }
        let revision = logged_query_as::<RevisionRow>(
            "SELECT article_id, revision_no, markdown_source, metadata_snapshot, operator_id, \
             created_at FROM article_revisions WHERE article_id = $1 AND revision_no = $2",
        )
        .bind(article_id)
        .bind(restore.revision_no)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(map_sqlx)?
        .ok_or(ContentError::RevisionNotFound)?;
        let revision = ArticleRevision::try_from(revision)?;
        let metadata = revision.metadata;

        validate_draft(&metadata.title, &metadata.summary, &metadata.slug)?;
        // 已发布文章恢复后仍必须满足发布校验，否则会违反 articles_published_content_check 约束。
        if current.status == ArticleStatus::Published {
            current.slug.clone_from(&metadata.slug);
            current.title.clone_from(&metadata.title);
            current.summary.clone_from(&metadata.summary);
            current
                .markdown_source
                .clone_from(&revision.markdown_source);
            current.rendered_html.clone_from(&restore.rendered_html);
            validate_publish(&current)?;
        }
        // 恢复前先为当前版本留档，保证任何一次写操作都可回溯。
        record_revision(&mut transaction, article_id, restore.operator_id).await?;

        let query = format!(
            "UPDATE articles SET category_id = $2, slug = $3, title = $4, summary = $5, \
             ai_brief = $13, cover_url = $6, markdown_source = $7, rendered_html = $8, \
             seo_keywords = $9, access_password_hash = $10, allow_comments = $11, is_pinned = $12, \
             version = version + 1, updated_at = now() \
             WHERE id = $1 RETURNING {ARTICLE_COLUMNS}"
        );
        let row = logged_query_as::<ArticleRow>(&query)
            .bind(article_id)
            .bind(metadata.category_id)
            .bind(metadata.slug)
            .bind(metadata.title)
            .bind(metadata.summary)
            .bind(metadata.ai_brief)
            .bind(metadata.cover_url)
            .bind(revision.markdown_source)
            .bind(restore.rendered_html)
            .bind(metadata.seo_keywords)
            .bind(metadata.access_password_hash)
            .bind(metadata.allow_comments)
            .bind(metadata.is_pinned)
            .fetch_one(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        sync_tags(&mut transaction, article_id, &metadata.tag_ids).await?;
        let article = lock_article(&mut transaction, row.id).await?;
        transaction.commit().await.map_err(map_sqlx)?;
        Ok(article)
    }

    async fn list_categories(&self, kind: CategoryKind) -> Result<Vec<Category>, ContentError> {
        let rows = logged_query_as::<CategoryRow>(
            "SELECT id, parent_id, kind, name, slug::text AS slug, description FROM categories \
             WHERE kind = $1 AND deleted_at IS NULL \
             ORDER BY sort_order, name, id",
        )
        .bind(kind.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn create_category(&self, mut category: NewCategory) -> Result<Category, ContentError> {
        category.name = validate_taxonomy_name(&category.name)?;
        category.slug = normalize_slug(&category.slug)?;
        let row = logged_query_as::<CategoryRow>(
            "INSERT INTO categories (parent_id, kind, name, slug, description) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id, parent_id, kind, name, slug::text AS slug, description",
        )
        .bind(category.parent_id)
        .bind(category.kind.as_str())
        .bind(category.name)
        .bind(category.slug)
        .bind(category.description.trim())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.try_into()
    }

    async fn update_category(
        &self,
        category_id: i64,
        kind: CategoryKind,
        mut update: CategoryUpdate,
    ) -> Result<Category, ContentError> {
        update.name = validate_taxonomy_name(&update.name)?;
        update.slug = normalize_slug(&update.slug)?;
        let row = logged_query_as::<CategoryRow>(
            "UPDATE categories SET name = $3, slug = $4, updated_at = now() \
             WHERE id = $1 AND kind = $2 AND deleted_at IS NULL \
             RETURNING id, parent_id, kind, name, slug::text AS slug, description",
        )
        .bind(category_id)
        .bind(kind.as_str())
        .bind(update.name)
        .bind(update.slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(ContentError::NotFound)?;
        row.try_into()
    }

    async fn delete_category(
        &self,
        category_id: i64,
        kind: CategoryKind,
    ) -> Result<(), ContentError> {
        // 引用保护：仍有内容使用时拒绝删除并返回引用数，避免产生悬空分类。
        // 引用表随 kind 而变：article → articles，gallery → galleries，link → links。
        let references_sql = match kind {
            CategoryKind::Article => {
                "SELECT count(*) FROM articles WHERE category_id = $1 AND deleted_at IS NULL"
            }
            CategoryKind::Gallery => {
                "SELECT count(*) FROM galleries WHERE category_id = $1 AND deleted_at IS NULL"
            }
            CategoryKind::Link => {
                "SELECT count(*) FROM links WHERE category_id = $1 AND deleted_at IS NULL"
            }
        };
        let references = logged_query_scalar::<i64>(references_sql)
            .bind(category_id)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        if references > 0 {
            return Err(ContentError::Referenced { count: references });
        }
        // 物理删除：UNIQUE (kind, slug) 不区分已删除行，软删除会导致同名 slug 无法重建。
        let result = logged_query(
            "DELETE FROM categories WHERE id = $1 AND kind = $2 AND deleted_at IS NULL",
        )
        .bind(category_id)
        .bind(kind.as_str())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(ContentError::NotFound);
        }
        Ok(())
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, ContentError> {
        let rows = logged_query_as::<TagRow>(
            "SELECT id, name::text AS name, slug::text AS slug FROM tags ORDER BY name, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn create_tag(&self, mut tag: NewTag) -> Result<Tag, ContentError> {
        tag.name = validate_taxonomy_name(&tag.name)?;
        tag.slug = normalize_slug(&tag.slug)?;
        let row = logged_query_as::<TagRow>(
            "INSERT INTO tags (name, slug) VALUES ($1, $2) \
             RETURNING id, name::text AS name, slug::text AS slug",
        )
        .bind(tag.name)
        .bind(tag.slug)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update_tag(&self, tag_id: i64, mut update: TagUpdate) -> Result<Tag, ContentError> {
        update.name = validate_taxonomy_name(&update.name)?;
        update.slug = normalize_slug(&update.slug)?;
        let row = logged_query_as::<TagRow>(
            "UPDATE tags SET name = $2, slug = $3, updated_at = now() WHERE id = $1 \
             RETURNING id, name::text AS name, slug::text AS slug",
        )
        .bind(tag_id)
        .bind(update.name)
        .bind(update.slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(ContentError::NotFound)?;
        Ok(row.into())
    }

    async fn delete_tag(&self, tag_id: i64) -> Result<(), ContentError> {
        // 引用保护：仍被文章关联时拒绝删除并返回引用数。
        let references = logged_query_scalar::<i64>(
            "SELECT count(*) FROM article_tags \
             WHERE tag_id = $1 AND EXISTS (SELECT 1 FROM articles \
                 WHERE id = article_tags.article_id AND deleted_at IS NULL)",
        )
        .bind(tag_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if references > 0 {
            return Err(ContentError::Referenced { count: references });
        }
        let result = logged_query("DELETE FROM tags WHERE id = $1")
            .bind(tag_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(ContentError::NotFound);
        }
        Ok(())
    }
}

async fn validate_category(
    transaction: &mut Transaction<'_, Postgres>,
    category_id: Option<i64>,
) -> Result<(), ContentError> {
    let Some(category_id) = category_id else {
        return Ok(());
    };
    let exists = logged_query_scalar::<bool>(
        "SELECT EXISTS(SELECT 1 FROM categories \
         WHERE id = $1 AND kind = 'article' AND deleted_at IS NULL)",
    )
    .bind(category_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(map_sqlx)?;
    if !exists {
        return Err(ContentError::NotFound);
    }
    Ok(())
}

async fn lock_article(
    transaction: &mut Transaction<'_, Postgres>,
    article_id: i64,
) -> Result<Article, ContentError> {
    let query = format!(
        "SELECT {ARTICLE_COLUMNS} FROM articles \
         WHERE id = $1 AND deleted_at IS NULL FOR UPDATE"
    );
    let row = logged_query_as::<ArticleRow>(&query)
        .bind(article_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(map_sqlx)?
        .ok_or(ContentError::NotFound)?;
    row.try_into()
}

async fn record_revision(
    transaction: &mut Transaction<'_, Postgres>,
    article_id: i64,
    operator_id: i64,
) -> Result<(), ContentError> {
    logged_query(
        "INSERT INTO article_revisions \
         (article_id, revision_no, markdown_source, metadata_snapshot, operator_id) \
         SELECT id, version, markdown_source, jsonb_build_object( \
             'title', title, 'slug', slug::text, 'summary', summary, 'ai_brief', ai_brief, \
             'category_id', category_id, \
             'cover_url', cover_url, 'seo_keywords', seo_keywords, \
             'access_password_hash', access_password_hash, 'allow_comments', allow_comments, \
             'is_pinned', is_pinned, 'status', status, 'published_at', published_at, \
             'tag_ids', COALESCE((SELECT jsonb_agg(tag_id ORDER BY tag_id) FROM article_tags \
                 WHERE article_id = articles.id), '[]'::jsonb)), $2 \
         FROM articles WHERE id = $1",
    )
    .bind(article_id)
    .bind(operator_id)
    .execute(&mut **transaction)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

async fn sync_tags(
    transaction: &mut Transaction<'_, Postgres>,
    article_id: i64,
    tag_ids: &[i64],
) -> Result<(), ContentError> {
    if !tag_ids.is_empty() {
        let found = logged_query_scalar::<i64>("SELECT count(*) FROM tags WHERE id = ANY($1)")
            .bind(tag_ids)
            .fetch_one(&mut **transaction)
            .await
            .map_err(map_sqlx)?;
        if found != i64::try_from(tag_ids.len()).map_err(|_| ContentError::StoreUnavailable)? {
            return Err(ContentError::NotFound);
        }
    }

    logged_query("DELETE FROM article_tags WHERE article_id = $1")
        .bind(article_id)
        .execute(&mut **transaction)
        .await
        .map_err(map_sqlx)?;
    if !tag_ids.is_empty() {
        logged_query(
            "INSERT INTO article_tags (article_id, tag_id) \
             SELECT $1, tag_id FROM unnest($2::bigint[]) AS u(tag_id)",
        )
        .bind(article_id)
        .bind(tag_ids)
        .execute(&mut **transaction)
        .await
        .map_err(map_sqlx)?;
    }
    Ok(())
}

fn normalize_tag_ids(tag_ids: &mut Vec<i64>) {
    tag_ids.sort_unstable();
    tag_ids.dedup();
}

fn map_sqlx(error: sqlx::Error) -> ContentError {
    if error
        .as_database_error()
        .is_some_and(|database_error| database_error.is_unique_violation())
    {
        ContentError::Conflict
    } else if error
        .as_database_error()
        .is_some_and(|database_error| database_error.is_foreign_key_violation())
    {
        ContentError::NotFound
    } else {
        tracing::error!(error = %error, "content repository operation failed");
        ContentError::StoreUnavailable
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, ensure};
    use aries_core::content::MarkdownRenderer;
    use uuid::Uuid;

    use super::*;
    use crate::ComrakMarkdownRenderer;

    #[tokio::test]
    async fn postgresql_repository_covers_article_publish_lifecycle() -> anyhow::Result<()> {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let _ = dotenvy::dotenv();
        let base_config = crate::PostgresConfig::from_env()?;
        let admin_pool = crate::connect_postgres(&base_config).await?;
        let test_schema = format!("aries_test_{}", Uuid::now_v7().simple());

        // 随机 Schema 隔离业务数据，测试无论成功或失败都会执行清理。
        logged_query(&format!("CREATE SCHEMA \"{test_schema}\""))
            .execute(&admin_pool)
            .await
            .context("failed to create isolated content test schema")?;
        let test_config = crate::PostgresConfig {
            options: base_config.options.clone(),
            schema: format!("{test_schema},{}", base_config.schema()),
            slow_query_ms: 0,
        };
        let test_pool = crate::connect_postgres(&test_config).await?;
        let scenario_result = async {
            crate::run_migrations(&test_pool).await?;
            run_repository_scenario(&test_pool).await
        }
        .await;

        test_pool.close().await;
        let cleanup_result = logged_query(&format!("DROP SCHEMA \"{test_schema}\" CASCADE"))
            .execute(&admin_pool)
            .await
            .context("failed to remove isolated content test schema");
        admin_pool.close().await;

        scenario_result?;
        cleanup_result?;
        Ok(())
    }

    async fn run_repository_scenario(pool: &PgPool) -> anyhow::Result<()> {
        let user_id = logged_query_scalar::<i64>(
            "INSERT INTO users \
             (username, email, password_hash, display_name, role, status) \
             VALUES ('editor', 'editor@example.com', 'hash', 'Editor', 'editor', 'active') \
             RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        let repository = PostgresContentRepository::new(pool.clone());
        let category = repository
            .create_category(NewCategory {
                parent_id: None,
                kind: CategoryKind::Article,
                name: "Backend".to_owned(),
                slug: "Backend".to_owned(),
                description: String::new(),
            })
            .await?;
        let tag = repository
            .create_tag(NewTag {
                name: "Rust".to_owned(),
                slug: "Rust".to_owned(),
            })
            .await?;
        let tag_id = tag.id;
        ensure!(repository.list_categories(CategoryKind::Article).await? == vec![category.clone()]);
        ensure!(repository.list_tags().await? == vec![tag]);
        ensure!(matches!(
            repository
                .create_tag(NewTag {
                    name: "Rust".to_owned(),
                    slug: "rust-duplicate".to_owned(),
                })
                .await,
            Err(ContentError::Conflict)
        ));

        let rendered_html = ComrakMarkdownRenderer.render("# First post")?;
        let article = repository
            .create_article(NewArticle {
                author_id: user_id,
                category_id: Some(category.id),
                slug: " First_Post ".to_owned(),
                title: "First post".to_owned(),
                summary: "Repository integration test".to_owned(),
                ai_brief: None,
                cover_url: None,
                markdown_source: "# First post".to_owned(),
                rendered_html,
                seo_keywords: vec!["rust".to_owned()],
                access_password_hash: None,
                allow_comments: true,
                is_pinned: false,
                tag_ids: vec![tag_id, tag_id],
            })
            .await?;
        ensure!(article.slug == "first-post");
        ensure!(article.version == 1);
        ensure!(article.category_id == Some(category.id));
        ensure!(article.tag_ids == vec![tag_id]);
        // Draft 对 Public 站不可见，按 slug 查询必须返回 None。
        ensure!(
            repository
                .find_published_article_by_slug("first-post")
                .await?
                .is_none()
        );

        let page = repository
            .list_articles(ArticleListQuery {
                page: 1,
                page_size: 20,
                keyword: Some("First".to_owned()),
                status: Some(ArticleStatus::Draft),
                category_id: None,
                tag_id: Some(tag_id),
                sort: ArticleSort::default(),
                order: SortOrder::default(),
            })
            .await?;
        ensure!(page.total == 1 && page.items.len() == 1);

        let updated = repository
            .update_article(
                article.id,
                ArticleUpdate {
                    category_id: Some(category.id),
                    slug: article.slug.clone(),
                    title: "First post, revised".to_owned(),
                    summary: article.summary.clone(),
                    ai_brief: Some("AI 导读草稿".to_owned()),
                    cover_url: None,
                    markdown_source: "# First post\n\nRevised.".to_owned(),
                    rendered_html: ComrakMarkdownRenderer.render("# First post\n\nRevised.")?,
                    seo_keywords: article.seo_keywords.clone(),
                    access_password_hash: None,
                    allow_comments: true,
                    is_pinned: false,
                    tag_ids: vec![tag_id],
                    expected_version: article.version,
                    operator_id: user_id,
                },
            )
            .await?;
        ensure!(updated.version == 2);
        ensure!(updated.tag_ids == vec![tag_id]);
        // ai_brief 与 summary 同语义：全量赋值，随 ARTICLE_COLUMNS 读回。
        ensure!(updated.ai_brief.as_deref() == Some("AI 导读草稿"));

        let published = repository
            .transition_article(
                article.id,
                updated.version,
                ArticleStatus::Published,
                user_id,
            )
            .await?;
        ensure!(published.status == ArticleStatus::Published);
        ensure!(published.published_at.is_some());
        ensure!(published.version == 3);
        let found_by_slug = repository
            .find_published_article_by_slug("first-post")
            .await?
            .context("published article must be visible by slug")?;
        ensure!(found_by_slug.id == article.id);
        ensure!(found_by_slug.tag_ids == vec![tag_id]);
        // slug 列为 citext，大小写不敏感命中同一条记录。
        ensure!(
            repository
                .find_published_article_by_slug("FIRST-POST")
                .await?
                .is_some()
        );
        ensure!(matches!(
            repository
                .update_article(
                    article.id,
                    ArticleUpdate {
                        category_id: None,
                        slug: published.slug.clone(),
                        title: published.title.clone(),
                        summary: published.summary.clone(),
                        ai_brief: None,
                        cover_url: None,
                        markdown_source: String::new(),
                        rendered_html: String::new(),
                        seo_keywords: published.seo_keywords.clone(),
                        access_password_hash: None,
                        allow_comments: true,
                        is_pinned: false,
                        tag_ids: vec![tag_id],
                        expected_version: published.version,
                        operator_id: user_id,
                    },
                )
                .await,
            Err(ContentError::PublishValidation)
        ));
        ensure!(matches!(
            repository
                .transition_article(article.id, 2, ArticleStatus::Recycled, user_id)
                .await,
            Err(ContentError::Conflict)
        ));

        let revision_count = logged_query_scalar::<i64>(
            "SELECT count(*) FROM article_revisions WHERE article_id = $1",
        )
        .bind(article.id)
        .fetch_one(pool)
        .await?;
        ensure!(revision_count == 2);

        // Revision 列表按 revision_no DESC，恢复后内容、标签与元数据与快照一致并产生新 Revision。
        let revisions = repository.list_revisions(article.id).await?;
        ensure!(revisions.len() == 2);
        ensure!(revisions[0].revision_no == 2 && revisions[1].revision_no == 1);
        let oldest = repository
            .find_revision(article.id, 1)
            .await?
            .context("missing oldest revision")?;
        ensure!(oldest.markdown_source == "# First post");
        ensure!(oldest.metadata.title == "First post");
        ensure!(oldest.metadata.tag_ids == vec![tag_id]);

        let restored = repository
            .restore_revision(
                article.id,
                RevisionRestore {
                    revision_no: 1,
                    rendered_html: ComrakMarkdownRenderer.render(&oldest.markdown_source)?,
                    expected_version: published.version,
                    operator_id: user_id,
                },
            )
            .await?;
        ensure!(restored.title == "First post");
        ensure!(restored.markdown_source == "# First post");
        ensure!(restored.tag_ids == vec![tag_id]);
        ensure!(restored.status == ArticleStatus::Published);
        ensure!(restored.version == published.version + 1);
        ensure!(repository.list_revisions(article.id).await?.len() == 3);
        ensure!(matches!(
            repository
                .restore_revision(
                    article.id,
                    RevisionRestore {
                        revision_no: 1,
                        rendered_html: String::new(),
                        expected_version: 1,
                        operator_id: user_id,
                    },
                )
                .await,
            Err(ContentError::Conflict)
        ));
        ensure!(matches!(
            repository
                .restore_revision(
                    article.id,
                    RevisionRestore {
                        revision_no: 99,
                        rendered_html: String::new(),
                        expected_version: restored.version,
                        operator_id: user_id,
                    },
                )
                .await,
            Err(ContentError::RevisionNotFound)
        ));

        // 访问密码三层语义：Some(Some) 设置、None 保持不变、Some(None) 清除。
        let mut current = restored;
        for (password, expected) in [
            (Some(Some("argon2-hash".to_owned())), true),
            (None, true),
            (Some(None), false),
        ] {
            current = repository
                .update_article(
                    article.id,
                    ArticleUpdate {
                        category_id: current.category_id,
                        slug: current.slug.clone(),
                        title: current.title.clone(),
                        summary: current.summary.clone(),
                        ai_brief: None,
                        cover_url: None,
                        markdown_source: current.markdown_source.clone(),
                        rendered_html: ComrakMarkdownRenderer.render(&current.markdown_source)?,
                        seo_keywords: current.seo_keywords.clone(),
                        access_password_hash: password,
                        allow_comments: true,
                        is_pinned: false,
                        tag_ids: vec![tag_id],
                        expected_version: current.version,
                        operator_id: user_id,
                    },
                )
                .await?;
            ensure!(current.password_protected == expected);
        }

        // Taxonomy 写操作与引用保护。
        let renamed_category = repository
            .update_category(
                category.id,
                CategoryKind::Article,
                CategoryUpdate {
                    name: "Backend Core".to_owned(),
                    slug: "Backend Core".to_owned(),
                },
            )
            .await?;
        ensure!(renamed_category.slug == "backend-core");
        let renamed_tag = repository
            .update_tag(
                tag_id,
                TagUpdate {
                    name: "Rust Lang".to_owned(),
                    slug: "rust-lang".to_owned(),
                },
            )
            .await?;
        ensure!(renamed_tag.name == "Rust Lang");
        ensure!(matches!(
            repository
                .delete_category(category.id, CategoryKind::Article)
                .await,
            Err(ContentError::Referenced { count: 1 })
        ));
        ensure!(matches!(
            repository.delete_tag(tag_id).await,
            Err(ContentError::Referenced { count: 1 })
        ));

        let invalid_result = repository
            .create_article(NewArticle {
                author_id: user_id,
                category_id: None,
                slug: "invalid-tag".to_owned(),
                title: "Invalid tag".to_owned(),
                summary: String::new(),
                ai_brief: None,
                cover_url: None,
                markdown_source: String::new(),
                rendered_html: String::new(),
                seo_keywords: Vec::new(),
                access_password_hash: None,
                allow_comments: true,
                is_pinned: false,
                tag_ids: vec![i64::MAX],
            })
            .await;
        ensure!(matches!(invalid_result, Err(ContentError::NotFound)));
        ensure!(
            logged_query_scalar::<i64>("SELECT count(*) FROM articles")
                .fetch_one(pool)
                .await?
                == 1
        );

        // 仅 recycled 状态允许物理删除；删除后 Revision 与关联同步清除。
        ensure!(matches!(
            repository.delete_article(article.id).await,
            Err(ContentError::NotRecycled)
        ));
        let recycled = repository
            .transition_article(
                article.id,
                current.version,
                ArticleStatus::Recycled,
                user_id,
            )
            .await?;
        ensure!(recycled.status == ArticleStatus::Recycled);
        // Recycled 对 Public 站不可见。
        ensure!(
            repository
                .find_published_article_by_slug("first-post")
                .await?
                .is_none()
        );
        repository.delete_article(article.id).await?;
        ensure!(repository.find_article(article.id).await?.is_none());
        ensure!(
            logged_query_scalar::<i64>(
                "SELECT count(*) FROM article_revisions WHERE article_id = $1",
            )
            .bind(article.id)
            .fetch_one(pool)
            .await?
                == 0
        );
        // 引用解除后 Category 与 Tag 可以删除。
        repository
            .delete_category(category.id, CategoryKind::Article)
            .await?;
        repository.delete_tag(tag_id).await?;
        Ok(())
    }
}
