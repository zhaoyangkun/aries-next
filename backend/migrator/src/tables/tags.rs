//! `tags` → `tags`：旧版只有 `name`，目标 `slug` 为 citext UNIQUE，迁移时生成并解决冲突；
//! `count` 为冗余计数，全行进 Archive。

use std::collections::HashSet;

use anyhow::Context;
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use super::{
    ArchiveEntry, CommonParams, TransformOutcome, apply_outcome, dedupe_slug, required_time,
    slugify,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "tags";

#[derive(Debug, Clone, FromRow)]
pub struct LegacyTag {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTag {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyTag>> {
    sqlx::query_as::<_, LegacyTag>(
        "SELECT CAST(id AS SIGNED) AS id, name, CAST(count AS SIGNED) AS count, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM tags ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy tags")
}

/// `used_slugs` 存放已占用 slug 的小写形式（citext 语义），跨行累计。
pub fn transform(
    row: &LegacyTag,
    params: &CommonParams,
    used_slugs: &mut HashSet<String>,
) -> anyhow::Result<TransformOutcome<NewTag>> {
    let mut archives = vec![ArchiveEntry {
        id: Some(row.id),
        fields: serde_json::json!({ "count": row.count }),
        reason: "redundant denormalized counter; no target column".to_owned(),
        whole_row: false,
    }];

    let base = slugify(&row.name, "tag", row.id);
    let (slug, conflict_repaired) = dedupe_slug(base, row.id, used_slugs);
    used_slugs.insert(slug.to_lowercase());
    let repaired = i64::from(conflict_repaired);
    let mut notes = Vec::new();
    if conflict_repaired {
        notes.push(format!("#{}: slug conflict resolved as `{}`", row.id, slug));
    }
    if row.deleted_at.is_some() {
        // 目标 tags 表无 deleted_at 列；旧库业务上恒 NULL，若非 NULL 整行归档不迁入。
        return Ok(TransformOutcome {
            row: None,
            archives: {
                archives.push(ArchiveEntry {
                    id: Some(row.id),
                    fields: serde_json::json!({ "name": row.name, "deleted_at": row.deleted_at }),
                    reason: "target tags table has no deleted_at column".to_owned(),
                    whole_row: true,
                });
                archives
            },
            repaired,
            notes,
        });
    }

    Ok(TransformOutcome {
        row: Some(NewTag {
            id: row.id,
            name: row.name.clone(),
            slug,
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

    let mut used_slugs = HashSet::new();
    let mut new_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let outcome = transform(row, &params, &mut used_slugs)?;
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
                "INSERT INTO tags (id, name, slug, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(&row.name)
            .bind(&row.slug)
            .bind(row.created_at)
            .bind(row.updated_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert tag #{}", row.id))?
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

    fn params() -> CommonParams {
        CommonParams {
            offset: UtcOffset::UTC,
            truncate_violations: false,
        }
    }

    fn legacy(id: i64, name: &str) -> LegacyTag {
        LegacyTag {
            id,
            name: name.to_owned(),
            count: 0,
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    #[test]
    fn slug_generated_from_name() {
        let mut used = HashSet::new();
        let outcome = transform(&legacy(1, "Hello World"), &params(), &mut used).unwrap();
        assert_eq!(outcome.row.unwrap().slug, "hello-world");
        assert_eq!(outcome.repaired, 0);
    }

    #[test]
    fn empty_after_normalize_falls_back_to_tag_id() {
        let mut used = HashSet::new();
        let outcome = transform(&legacy(9, "中文标签"), &params(), &mut used).unwrap();
        assert_eq!(outcome.row.unwrap().slug, "tag-9");
    }

    #[test]
    fn conflicting_slug_appends_id_and_counts_repaired() {
        let mut used = HashSet::new();
        let first = transform(&legacy(1, "rust"), &params(), &mut used).unwrap();
        assert_eq!(first.row.unwrap().slug, "rust");
        let second = transform(&legacy(2, "Rust"), &params(), &mut used).unwrap();
        // slugify 先小写化，冲突追加 -{id}。
        assert_eq!(second.row.unwrap().slug, "rust-2");
        assert_eq!(second.repaired, 1);
    }

    #[test]
    fn soft_deleted_tag_is_archived_not_migrated() {
        let mut used = HashSet::new();
        let mut row = legacy(1, "x");
        row.deleted_at = Some("2021-01-01 00:00:00".to_owned());
        let outcome = transform(&row, &params(), &mut used).unwrap();
        assert!(outcome.row.is_none());
        assert!(
            outcome
                .archives
                .iter()
                .any(|a| a.reason.contains("deleted_at"))
        );
    }
}
