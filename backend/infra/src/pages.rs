//! 页面 Repository 的 PostgreSQL 实现。
//! slug 唯一约束为部分索引（WHERE deleted_at IS NULL），冲突映射为 Conflict。

use crate::like::like_pattern;
use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use crate::where_clause::WhereBuilder;
use aries_core::pages::{
    NewPage, Page, PageError, PageListQuery, PagePage, PageRepository, PageUpdate,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresPageRepository {
    pool: PgPool,
}

impl PostgresPageRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct PageRow {
    id: i64,
    slug: String,
    title: String,
    content_markdown: String,
    content_html: String,
    status: String,
    sort_order: i32,
    created_by: Option<i64>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<PageRow> for Page {
    type Error = PageError;

    fn try_from(row: PageRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            slug: row.slug,
            title: row.title,
            content_markdown: row.content_markdown,
            content_html: row.content_html,
            status: row.status.parse().map_err(|_| PageError::InvalidStatus)?,
            sort_order: row.sort_order,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const PAGE_COLUMNS: &str = "id, slug::text AS slug, title, content_markdown, content_html, \
    status, sort_order, created_by, created_at, updated_at";

fn map_sqlx(error: sqlx::Error) -> PageError {
    // 23505 = unique_violation：slug 部分唯一索引冲突。
    if let sqlx::Error::Database(db) = &error {
        if db.code().as_deref() == Some("23505") {
            return PageError::Conflict;
        }
    }
    tracing::error!(error = %error, "page repository operation failed");
    PageError::StoreUnavailable
}

#[async_trait]
impl PageRepository for PostgresPageRepository {
    async fn create(&self, page: NewPage) -> Result<Page, PageError> {
        aries_core::pages::validate_slug(&page.slug)?;
        aries_core::pages::validate_title(&page.title)?;
        let query = format!(
            "INSERT INTO pages \
             (slug, title, content_markdown, content_html, status, sort_order, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING {PAGE_COLUMNS}"
        );
        let row = logged_query_as::<PageRow>(&query)
            .bind(&page.slug)
            .bind(&page.title)
            .bind(&page.content_markdown)
            .bind(&page.content_html)
            .bind(page.status.as_str())
            .bind(page.sort_order)
            .bind(page.created_by)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }

    async fn find(&self, page_id: i64) -> Result<Option<Page>, PageError> {
        let query =
            format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = $1 AND deleted_at IS NULL");
        let row = logged_query_as::<PageRow>(&query)
            .bind(page_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn find_published_by_slug(&self, slug: &str) -> Result<Option<Page>, PageError> {
        let query = format!(
            "SELECT {PAGE_COLUMNS} FROM pages \
             WHERE slug = $1::citext AND status = 'published' AND deleted_at IS NULL"
        );
        let row = logged_query_as::<PageRow>(&query)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn list(&self, query: PageListQuery) -> Result<PagePage, PageError> {
        let offset = (query.page.saturating_sub(1)) * query.page_size;
        let limit = i64::from(query.page_size);

        // count 与 data 共享同一份条件构建，bind 序列从构造上保持一致。
        let mut where_ = WhereBuilder::new();
        where_.push_static("deleted_at IS NULL");
        if let Some(ref status) = query.status {
            where_.push("status = {}", status.as_str());
        }
        if let Some(ref keyword) = query.keyword {
            where_.push(
                "(title ILIKE {} ESCAPE '\\' \
                 OR slug::text ILIKE {} ESCAPE '\\')",
                like_pattern(keyword),
            );
        }
        let clause = where_.clause();

        let count_sql = format!("SELECT COUNT(*) FROM pages {clause}");
        let total = where_
            .bind_to(logged_query_scalar::<i64>(&count_sql))
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;

        let next_index = where_.next_index();
        let data_sql = format!(
            "SELECT {PAGE_COLUMNS} FROM pages {clause} \
             ORDER BY sort_order, id LIMIT ${next_index} OFFSET ${}",
            next_index + 1
        );
        let rows = where_
            .bind_to(logged_query_as::<PageRow>(&data_sql))
            .bind(limit)
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let items: Vec<Page> = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(PagePage {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }

    async fn update(&self, page_id: i64, update: PageUpdate) -> Result<Page, PageError> {
        aries_core::pages::validate_slug(&update.slug)?;
        aries_core::pages::validate_title(&update.title)?;
        let query = format!(
            "UPDATE pages SET slug = $2, title = $3, content_markdown = $4, content_html = $5, \
             status = $6, sort_order = $7, updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL \
             RETURNING {PAGE_COLUMNS}"
        );
        let row = logged_query_as::<PageRow>(&query)
            .bind(page_id)
            .bind(&update.slug)
            .bind(&update.title)
            .bind(&update.content_markdown)
            .bind(&update.content_html)
            .bind(update.status.as_str())
            .bind(update.sort_order)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(PageError::NotFound)?;
        row.try_into()
    }

    async fn delete(&self, page_id: i64) -> Result<(), PageError> {
        let result = logged_query(
            "UPDATE pages SET deleted_at = now(), updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(page_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(PageError::NotFound);
        }
        Ok(())
    }
}
