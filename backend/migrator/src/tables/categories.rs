//! `categories` → `categories`：`type` 0/1/2 → kind article/link/gallery；
//! `parent_id=0` 或悬挂 → NULL（记 repaired）；`count` 为冗余派生数据，全行进 Archive。

use std::collections::HashSet;

use anyhow::{Context, bail};
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use super::{
    ArchiveEntry, CommonParams, TransformOutcome, apply_outcome, optional_time, required_time,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "categories";

#[derive(Debug, Clone, FromRow)]
pub struct LegacyCategory {
    pub id: i64,
    pub parent_id: i64,
    pub r#type: i64,
    pub name: String,
    pub url: String,
    pub count: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewCategory {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub kind: String,
    pub name: String,
    pub slug: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyCategory>> {
    sqlx::query_as::<_, LegacyCategory>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(parent_id AS SIGNED) AS parent_id, \
         CAST(`type` AS SIGNED) AS `type`, name, url, CAST(count AS SIGNED) AS count, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM categories ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy categories")
}

pub fn map_kind(legacy_type: i64, id: i64) -> anyhow::Result<&'static str> {
    match legacy_type {
        0 => Ok("article"),
        1 => Ok("link"),
        2 => Ok("gallery"),
        other => bail!("categories #{id}: unknown type {other} (expected 0/1/2)"),
    }
}

pub fn transform(
    row: &LegacyCategory,
    params: &CommonParams,
    valid_ids: &HashSet<i64>,
) -> anyhow::Result<TransformOutcome<NewCategory>> {
    let mut archives = Vec::new();
    // count 是可由关联行数派生的冗余计数，目标无此字段；全量归档以保留追溯。
    archives.push(ArchiveEntry {
        id: Some(row.id),
        fields: serde_json::json!({ "count": row.count }),
        reason: "redundant denormalized counter; no target column".to_owned(),
        whole_row: false,
    });

    let mut repaired = 0;
    let parent_id = match row.parent_id {
        0 => None,
        id if valid_ids.contains(&id) && id != row.id => Some(id),
        dangling => {
            repaired += 1;
            archives.push(ArchiveEntry {
                id: Some(row.id),
                fields: serde_json::json!({ "parent_id": dangling }),
                reason: "dangling or self parent reference; reset to top level".to_owned(),
                whole_row: false,
            });
            None
        }
    };

    let mut notes = Vec::new();
    let slug = if row.url.trim().is_empty() {
        repaired += 1;
        notes.push(format!(
            "#{}: empty slug replaced with generated value",
            row.id
        ));
        format!("category-{}", row.id)
    } else {
        row.url.clone()
    };

    Ok(TransformOutcome {
        row: Some(NewCategory {
            id: row.id,
            parent_id,
            kind: map_kind(row.r#type, row.id)?.to_owned(),
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
    let valid_ids: HashSet<i64> = rows.iter().map(|row| row.id).collect();

    let mut new_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let outcome = transform(row, &params, &valid_ids)?;
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
                "INSERT INTO categories (id, parent_id, kind, name, slug, created_at, updated_at, deleted_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(row.parent_id)
            .bind(&row.kind)
            .bind(&row.name)
            .bind(&row.slug)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert category #{}", row.id))?
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

    fn legacy(id: i64, parent_id: i64, legacy_type: i64) -> LegacyCategory {
        LegacyCategory {
            id,
            parent_id,
            r#type: legacy_type,
            name: "默认分类".to_owned(),
            url: "default".to_owned(),
            count: 3,
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    #[test]
    fn maps_all_kind_branches() {
        assert_eq!(map_kind(0, 1).unwrap(), "article");
        assert_eq!(map_kind(1, 1).unwrap(), "link");
        assert_eq!(map_kind(2, 1).unwrap(), "gallery");
        assert!(map_kind(3, 1).is_err());
    }

    #[test]
    fn zero_parent_becomes_null_and_count_is_archived() {
        let ids: HashSet<i64> = [1].into_iter().collect();
        let outcome = transform(&legacy(1, 0, 0), &params(), &ids).unwrap();
        let row = outcome.row.unwrap();
        assert_eq!(row.parent_id, None);
        assert_eq!(row.kind, "article");
        assert!(
            outcome
                .archives
                .iter()
                .any(|a| a.fields.get("count").is_some())
        );
        assert_eq!(outcome.repaired, 0);
    }

    #[test]
    fn dangling_or_self_parent_is_repaired_to_null() {
        let ids: HashSet<i64> = [1, 2].into_iter().collect();
        let outcome = transform(&legacy(2, 99, 0), &params(), &ids).unwrap();
        assert_eq!(outcome.row.unwrap().parent_id, None);
        assert_eq!(outcome.repaired, 1);

        let outcome = transform(&legacy(2, 2, 0), &params(), &ids).unwrap();
        assert_eq!(outcome.row.unwrap().parent_id, None);
        assert_eq!(outcome.repaired, 1);
    }

    #[test]
    fn valid_parent_kept_and_empty_slug_generated() {
        let ids: HashSet<i64> = [1, 2].into_iter().collect();
        let outcome = transform(&legacy(2, 1, 2), &params(), &ids).unwrap();
        assert_eq!(outcome.row.as_ref().unwrap().parent_id, Some(1));
        assert_eq!(outcome.row.as_ref().unwrap().kind, "gallery");

        let mut row = legacy(3, 0, 1);
        row.url = "  ".to_owned();
        let outcome = transform(&row, &params(), &[3].into_iter().collect()).unwrap();
        assert_eq!(outcome.row.as_ref().unwrap().slug, "category-3");
        assert_eq!(outcome.repaired, 1);
    }
}
