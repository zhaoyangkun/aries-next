//! `journals` → `journals`：`is_secret` → visibility private/public；content 1–2000 字符。

use anyhow::Context;
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use aries_core::content::MarkdownRenderer;
use aries_infra::ComrakMarkdownRenderer;

use super::{
    CommonParams, TransformOutcome, apply_outcome, enforce_length, enforce_nonempty, optional_time,
    required_time, tinyint_to_bool,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "journals";

const MAX_CONTENT_CHARS: usize = 2000;

#[derive(Debug, Clone, FromRow)]
pub struct LegacyJournal {
    pub id: i64,
    pub is_secret: i64,
    pub content: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewJournal {
    pub id: i64,
    pub content_markdown: String,
    pub content_html: String,
    pub visibility: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyJournal>> {
    sqlx::query_as::<_, LegacyJournal>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(is_secret AS SIGNED) AS is_secret, content, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM journals ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy journals")
}

pub fn map_visibility(is_secret: bool) -> &'static str {
    if is_secret { "private" } else { "public" }
}

pub fn transform(
    row: &LegacyJournal,
    params: &CommonParams,
    renderer: &ComrakMarkdownRenderer,
) -> anyhow::Result<TransformOutcome<NewJournal>> {
    let mut archives = Vec::new();
    let mut repaired = 0i64;

    let (value, archive_a, fix_a) = enforce_nonempty(
        params,
        TABLE,
        row.id,
        "content",
        row.content.clone(),
        || " ".to_owned(),
    )?;
    let (content, archive_b, fix_b) =
        enforce_length(params, TABLE, row.id, "content", value, MAX_CONTENT_CHARS)?;
    for entry in [archive_a, archive_b].into_iter().flatten() {
        archives.push(entry);
    }
    repaired += fix_a + fix_b;

    Ok(TransformOutcome {
        row: Some(NewJournal {
            id: row.id,
            content_html: renderer.render(&content)?,
            content_markdown: content,
            visibility: map_visibility(tinyint_to_bool(row.is_secret, TABLE, row.id, "is_secret")?)
                .to_owned(),
            created_at: required_time(
                row.created_at.as_deref(),
                params.offset,
                TABLE,
                row.id,
                "created_at",
            )?,
            updated_at: required_time(
                row.updated_at.as_deref(),
                params.offset,
                TABLE,
                row.id,
                "updated_at",
            )?,
            deleted_at: optional_time(
                row.deleted_at.as_deref(),
                params.offset,
                TABLE,
                row.id,
                "deleted_at",
            )?,
        }),
        archives,
        repaired,
        notes: Vec::new(),
    })
}

pub async fn migrate(ctx: &mut MigrateCtx) -> anyhow::Result<()> {
    let rows = extract(&ctx.mysql).await?;
    let params = CommonParams {
        offset: ctx.offset,
        truncate_violations: ctx.truncate_violations,
    };
    ctx.report.table(TABLE).source_rows = rows.len() as i64;

    let mut new_rows = Vec::with_capacity(rows.len());
    let renderer = ctx.renderer;
    for row in &rows {
        let outcome = transform(row, &params, &renderer)?;
        if let Some(new_row) = apply_outcome(ctx, TABLE, outcome)? {
            new_rows.push(new_row);
        }
    }

    let mut migrated = 0i64;
    let mut skipped = 0i64;
    for chunk in new_rows.chunks(ctx.batch_size.max(1)) {
        let mut tx = ctx.pg.begin().await?;
        for row in chunk {
            let affected = sqlx::query(
                "INSERT INTO journals (id, content_markdown, content_html, visibility, \
                 created_at, updated_at, deleted_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(&row.content_markdown)
            .bind(&row.content_html)
            .bind(&row.visibility)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert journal #{}", row.id))?
            .rows_affected();
            if affected == 1 {
                migrated += 1;
            } else {
                skipped += 1;
            }
        }
        tx.commit().await?;
    }

    let report = ctx.report.table(TABLE);
    report.migrated = migrated;
    report.skipped = skipped;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::UtcOffset;

    fn params(truncate: bool) -> CommonParams {
        CommonParams {
            offset: UtcOffset::UTC,
            truncate_violations: truncate,
        }
    }

    fn legacy(id: i64, is_secret: i64) -> LegacyJournal {
        LegacyJournal {
            id,
            is_secret,
            content: "今天天气不错".to_owned(),
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    #[test]
    fn visibility_mapping() {
        assert_eq!(map_visibility(true), "private");
        assert_eq!(map_visibility(false), "public");
    }

    #[test]
    fn secret_journal_becomes_private() {
        let outcome = transform(&legacy(1, 1), &params(false), &ComrakMarkdownRenderer).unwrap();
        assert_eq!(outcome.row.unwrap().visibility, "private");
    }

    #[test]
    fn overlong_content_fails_then_truncates() {
        let mut row = legacy(1, 0);
        row.content = "长".repeat(2001);
        assert!(transform(&row, &params(false), &ComrakMarkdownRenderer).is_err());
        let outcome = transform(&row, &params(true), &ComrakMarkdownRenderer).unwrap();
        assert_eq!(outcome.row.unwrap().content_markdown.chars().count(), 2000);
        assert_eq!(outcome.repaired, 1);
    }
}
