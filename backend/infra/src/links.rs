//! 友情链接 Repository 的 PostgreSQL 实现。
//! URL scheme 白名单校验在写入前执行，防止 javascript: 等危险输入落库。

use crate::like::like_pattern;
use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use crate::where_clause::WhereBuilder;
use aries_core::links::{
    Link, LinkError, LinkListQuery, LinkPage, LinkRepository, LinkUpdate, NewLink,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresLinkRepository {
    pool: PgPool,
}

impl PostgresLinkRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct LinkRow {
    id: i64,
    category_id: Option<i64>,
    title: String,
    url: String,
    icon_url: Option<String>,
    description: String,
    status: String,
    sort_order: i32,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<LinkRow> for Link {
    type Error = LinkError;

    fn try_from(row: LinkRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            category_id: row.category_id,
            title: row.title,
            url: row.url,
            icon_url: row.icon_url,
            description: row.description,
            status: row.status.parse().map_err(|_| LinkError::InvalidStatus)?,
            sort_order: row.sort_order,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const LINK_COLUMNS: &str = "id, category_id, title, url, icon_url, description, status, \
    sort_order, created_at, updated_at";

fn map_sqlx(error: sqlx::Error) -> LinkError {
    if let sqlx::Error::Database(db) = &error {
        // 23503 = foreign_key_violation：分类不存在。
        if db.code().as_deref() == Some("23503") {
            return LinkError::NotFound;
        }
    }
    tracing::error!(error = %error, "link repository operation failed");
    LinkError::StoreUnavailable
}

#[async_trait]
impl LinkRepository for PostgresLinkRepository {
    async fn create(&self, link: NewLink) -> Result<Link, LinkError> {
        aries_core::links::validate_title(&link.title)?;
        aries_core::links::validate_url(&link.url)?;
        let query = format!(
            "INSERT INTO links (category_id, title, url, icon_url, description, status, sort_order) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING {LINK_COLUMNS}"
        );
        let row = logged_query_as::<LinkRow>(&query)
            .bind(link.category_id)
            .bind(link.title.trim())
            .bind(&link.url)
            .bind(&link.icon_url)
            .bind(link.description.trim())
            .bind(link.status.as_str())
            .bind(link.sort_order)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }

    async fn find(&self, link_id: i64) -> Result<Option<Link>, LinkError> {
        let query =
            format!("SELECT {LINK_COLUMNS} FROM links WHERE id = $1 AND deleted_at IS NULL");
        let row = logged_query_as::<LinkRow>(&query)
            .bind(link_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn list(&self, query: LinkListQuery) -> Result<LinkPage, LinkError> {
        let offset = (query.page.saturating_sub(1)) * query.page_size;
        let limit = i64::from(query.page_size);

        // count 与 data 共享同一份条件构建，bind 序列从构造上保持一致。
        let mut where_ = WhereBuilder::new();
        where_.push_static("deleted_at IS NULL");
        if let Some(category_id) = query.category_id {
            where_.push("category_id = {}", category_id);
        }
        if let Some(ref status) = query.status {
            where_.push("status = {}", status.as_str());
        }
        if let Some(ref keyword) = query.keyword {
            where_.push(
                "(title ILIKE {} ESCAPE '\\' \
                 OR url ILIKE {} ESCAPE '\\')",
                like_pattern(keyword),
            );
        }
        let clause = where_.clause();

        let count_sql = format!("SELECT COUNT(*) FROM links {clause}");
        let total = where_
            .bind_to(logged_query_scalar::<i64>(&count_sql))
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;

        let next_index = where_.next_index();
        let data_sql = format!(
            "SELECT {LINK_COLUMNS} FROM links {clause} \
             ORDER BY sort_order, id LIMIT ${next_index} OFFSET ${}",
            next_index + 1
        );
        let rows = where_
            .bind_to(logged_query_as::<LinkRow>(&data_sql))
            .bind(limit)
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let items: Vec<Link> = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(LinkPage {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }

    async fn list_public(&self) -> Result<Vec<Link>, LinkError> {
        let query = format!(
            "SELECT {LINK_COLUMNS} FROM links \
             WHERE status = 'active' AND deleted_at IS NULL \
             ORDER BY sort_order, id"
        );
        let rows = logged_query_as::<LinkRow>(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()
    }

    async fn update(&self, link_id: i64, update: LinkUpdate) -> Result<Link, LinkError> {
        aries_core::links::validate_title(&update.title)?;
        aries_core::links::validate_url(&update.url)?;
        let query = format!(
            "UPDATE links SET category_id = $2, title = $3, url = $4, icon_url = $5, \
             description = $6, status = $7, sort_order = $8, updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL \
             RETURNING {LINK_COLUMNS}"
        );
        let row = logged_query_as::<LinkRow>(&query)
            .bind(link_id)
            .bind(update.category_id)
            .bind(update.title.trim())
            .bind(&update.url)
            .bind(&update.icon_url)
            .bind(update.description.trim())
            .bind(update.status.as_str())
            .bind(update.sort_order)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(LinkError::NotFound)?;
        row.try_into()
    }

    async fn delete(&self, link_id: i64) -> Result<(), LinkError> {
        let result = logged_query(
            "UPDATE links SET deleted_at = now(), updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(link_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(LinkError::NotFound);
        }
        Ok(())
    }
}
