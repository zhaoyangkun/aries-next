//! `pages` → `pages`：旧版无状态字段，默认 `published`；`html` 是 Markdown 源文，
//! `content_html` 重新渲染，旧 `md_html` 进 Archive。

use anyhow::Context;
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use aries_core::content::MarkdownRenderer;
use aries_infra::ComrakMarkdownRenderer;

use super::{
    CommonParams, TransformOutcome, apply_outcome, enforce_length, enforce_nonempty, optional_time,
    required_time,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "pages";

const MAX_TITLE_CHARS: usize = 200;

#[derive(Debug, Clone, FromRow)]
pub struct LegacyPage {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub html: String,
    pub md_html: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewPage {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub content_markdown: String,
    pub content_html: String,
    pub status: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyPage>> {
    sqlx::query_as::<_, LegacyPage>(
        "SELECT CAST(id AS SIGNED) AS id, title, url, html, md_html, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM pages ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy pages")
}

pub fn transform(
    row: &LegacyPage,
    params: &CommonParams,
    renderer: &ComrakMarkdownRenderer,
) -> anyhow::Result<TransformOutcome<NewPage>> {
    let mut archives = Vec::new();
    let mut repaired = 0i64;
    let mut notes = Vec::new();

    let (value, archive_a, fix_a) =
        enforce_nonempty(params, TABLE, row.id, "title", row.title.clone(), || {
            format!("page-{}", row.id)
        })?;
    let (title, archive_b, fix_b) =
        enforce_length(params, TABLE, row.id, "title", value, MAX_TITLE_CHARS)?;
    for entry in [archive_a, archive_b].into_iter().flatten() {
        archives.push(entry);
    }
    repaired += fix_a + fix_b;

    let slug = if row.url.trim().is_empty() {
        repaired += 1;
        notes.push(format!(
            "#{}: empty slug replaced with generated value",
            row.id
        ));
        format!("page-{}", row.id)
    } else {
        row.url.clone()
    };

    super::archive_unmapped_field(
        &mut archives,
        row.id,
        "md_html",
        &row.md_html,
        "legacy rendered HTML replaced by fresh comrak render",
    );

    Ok(TransformOutcome {
        row: Some(NewPage {
            id: row.id,
            slug,
            title,
            content_html: renderer.render(&row.html)?,
            content_markdown: row.html.clone(),
            status: "published".to_owned(),
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
        notes,
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
                "INSERT INTO pages (id, slug, title, content_markdown, content_html, status, \
                 created_at, updated_at, deleted_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(&row.slug)
            .bind(&row.title)
            .bind(&row.content_markdown)
            .bind(&row.content_html)
            .bind(&row.status)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert page #{}", row.id))?
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

    fn legacy(id: i64) -> LegacyPage {
        LegacyPage {
            id,
            title: "关于".to_owned(),
            url: "about".to_owned(),
            html: "# 关于我".to_owned(),
            md_html: "<h1>关于我</h1>".to_owned(),
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    #[test]
    fn defaults_to_published_and_rerenders() {
        let outcome = transform(&legacy(1), &params(false), &ComrakMarkdownRenderer).unwrap();
        let row = outcome.row.unwrap();
        assert_eq!(row.status, "published");
        assert_eq!(row.slug, "about");
        assert!(row.content_html.contains("<h1>关于我</h1>"));
        assert!(
            outcome
                .archives
                .iter()
                .any(|a| a.fields.get("md_html").is_some())
        );
    }

    #[test]
    fn empty_title_hard_error_then_placeholder_with_flag() {
        let mut row = legacy(1);
        row.title = String::new();
        assert!(transform(&row, &params(false), &ComrakMarkdownRenderer).is_err());
        let outcome = transform(&row, &params(true), &ComrakMarkdownRenderer).unwrap();
        assert_eq!(outcome.row.unwrap().title, "page-1");
        assert_eq!(outcome.repaired, 1);
    }

    #[test]
    fn overlong_title_truncates_with_flag() {
        let mut row = legacy(1);
        row.title = "标".repeat(201);
        assert!(transform(&row, &params(false), &ComrakMarkdownRenderer).is_err());
        let outcome = transform(&row, &params(true), &ComrakMarkdownRenderer).unwrap();
        assert_eq!(outcome.row.unwrap().title.chars().count(), 200);
    }
}
