//! 日志 Repository 的 PostgreSQL 实现。
//! 公开查询固定 visibility = 'public'，Private 日志不外泄。

use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use aries_core::journals::{
    Journal, JournalError, JournalListQuery, JournalPage, JournalRepository, JournalUpdate,
    JournalVisibility, NewJournal,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresJournalRepository {
    pool: PgPool,
}

impl PostgresJournalRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct JournalRow {
    id: i64,
    content_markdown: String,
    content_html: String,
    visibility: String,
    created_by: Option<i64>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<JournalRow> for Journal {
    type Error = JournalError;

    fn try_from(row: JournalRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            content_markdown: row.content_markdown,
            content_html: row.content_html,
            visibility: row
                .visibility
                .parse()
                .map_err(|_| JournalError::InvalidVisibility)?,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const JOURNAL_COLUMNS: &str =
    "id, content_markdown, content_html, visibility, created_by, created_at, updated_at";

fn map_sqlx(error: sqlx::Error) -> JournalError {
    tracing::error!(error = %error, "journal repository operation failed");
    JournalError::StoreUnavailable
}

#[async_trait]
impl JournalRepository for PostgresJournalRepository {
    async fn create(&self, journal: NewJournal) -> Result<Journal, JournalError> {
        aries_core::journals::validate_content(&journal.content_markdown)?;
        let query = format!(
            "INSERT INTO journals (content_markdown, content_html, visibility, created_by) \
             VALUES ($1, $2, $3, $4) \
             RETURNING {JOURNAL_COLUMNS}"
        );
        let row = logged_query_as::<JournalRow>(&query)
            .bind(&journal.content_markdown)
            .bind(&journal.content_html)
            .bind(journal.visibility.as_str())
            .bind(journal.created_by)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }

    async fn find(&self, journal_id: i64) -> Result<Option<Journal>, JournalError> {
        let query =
            format!("SELECT {JOURNAL_COLUMNS} FROM journals WHERE id = $1 AND deleted_at IS NULL");
        let row = logged_query_as::<JournalRow>(&query)
            .bind(journal_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn list(&self, query: JournalListQuery) -> Result<JournalPage, JournalError> {
        let offset = (query.page.saturating_sub(1)) * query.page_size;
        let limit = i64::from(query.page_size);

        let mut conditions = vec!["deleted_at IS NULL".to_string()];
        let mut bind_index = 1u32;
        if query.visibility.is_some() {
            conditions.push(format!("visibility = ${bind_index}"));
            bind_index += 1;
        }
        let where_clause = conditions.join(" AND ");

        let count_sql = format!("SELECT COUNT(*) FROM journals WHERE {where_clause}");
        let mut count_query = logged_query_scalar::<i64>(&count_sql);
        if let Some(visibility) = query.visibility {
            count_query = count_query.bind(visibility.as_str());
        }
        let total = count_query.fetch_one(&self.pool).await.map_err(map_sqlx)?;

        let data_sql = format!(
            "SELECT {JOURNAL_COLUMNS} FROM journals WHERE {where_clause} \
             ORDER BY created_at DESC, id DESC LIMIT ${bind_index} OFFSET ${}",
            bind_index + 1
        );
        let mut data_query = logged_query_as::<JournalRow>(&data_sql);
        if let Some(visibility) = query.visibility {
            data_query = data_query.bind(visibility.as_str());
        }
        data_query = data_query.bind(limit).bind(i64::from(offset));

        let rows = data_query.fetch_all(&self.pool).await.map_err(map_sqlx)?;
        let items: Vec<Journal> = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(JournalPage {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }

    async fn list_public(&self, page: u32, page_size: u32) -> Result<JournalPage, JournalError> {
        // 复用 list 的过滤逻辑，固定 visibility = public，保证 Private 不外泄。
        self.list(JournalListQuery {
            page,
            page_size,
            visibility: Some(JournalVisibility::Public),
        })
        .await
    }

    async fn update(
        &self,
        journal_id: i64,
        update: JournalUpdate,
    ) -> Result<Journal, JournalError> {
        aries_core::journals::validate_content(&update.content_markdown)?;
        let query = format!(
            "UPDATE journals SET content_markdown = $2, content_html = $3, visibility = $4, \
             updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL \
             RETURNING {JOURNAL_COLUMNS}"
        );
        let row = logged_query_as::<JournalRow>(&query)
            .bind(journal_id)
            .bind(&update.content_markdown)
            .bind(&update.content_html)
            .bind(update.visibility.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(JournalError::NotFound)?;
        row.try_into()
    }

    async fn delete(&self, journal_id: i64) -> Result<(), JournalError> {
        let result = logged_query(
            "UPDATE journals SET deleted_at = now(), updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(journal_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(JournalError::NotFound);
        }
        Ok(())
    }
}
