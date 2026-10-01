//! `tag_article` → `article_tags`：去重；悬挂关联（文章或标签不存在）进 Archive + Report。

use std::collections::HashSet;

use anyhow::Context;
use sqlx::{FromRow, MySqlPool};

use super::{ArchiveEntry, TransformOutcome, apply_outcome, fetch_id_set};
use crate::context::MigrateCtx;

pub const TABLE: &str = "article_tags";

#[derive(Debug, Clone, Copy, FromRow, PartialEq, Eq, Hash)]
pub struct LegacyTagArticle {
    pub article_id: i64,
    pub tag_id: i64,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyTagArticle>> {
    sqlx::query_as::<_, LegacyTagArticle>(
        "SELECT CAST(article_id AS SIGNED) AS article_id, CAST(tag_id AS SIGNED) AS tag_id \
         FROM tag_article ORDER BY article_id, tag_id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy tag_article")
}

/// 纯函数：去重 + 悬挂检测。返回（有效关联，归档条目，重复条数）。
pub fn transform(
    rows: &[LegacyTagArticle],
    article_ids: &HashSet<i64>,
    tag_ids: &HashSet<i64>,
) -> (Vec<LegacyTagArticle>, Vec<ArchiveEntry>, i64) {
    let mut seen = HashSet::new();
    let mut valid = Vec::new();
    let mut archives = Vec::new();
    let mut duplicates = 0i64;
    for row in rows {
        if !article_ids.contains(&row.article_id) || !tag_ids.contains(&row.tag_id) {
            archives.push(ArchiveEntry {
                id: None,
                fields: serde_json::json!({
                    "article_id": row.article_id,
                    "tag_id": row.tag_id,
                }),
                reason: "dangling article or tag reference".to_owned(),
                whole_row: true,
            });
            continue;
        }
        if !seen.insert((row.article_id, row.tag_id)) {
            duplicates += 1;
            continue;
        }
        valid.push(*row);
    }
    (valid, archives, duplicates)
}

pub async fn migrate(ctx: &mut MigrateCtx) -> anyhow::Result<()> {
    let rows = extract(&ctx.mysql).await?;
    ctx.report.table(TABLE).source_rows = rows.len() as i64;
    let article_ids = fetch_id_set(&ctx.mysql, "articles").await?;
    let tag_ids = fetch_id_set(&ctx.mysql, "tags").await?;

    let (valid, archives, duplicates) = transform(&rows, &article_ids, &tag_ids);
    let outcome: TransformOutcome<()> = TransformOutcome {
        row: None,
        archives,
        repaired: 0,
        notes: if duplicates > 0 {
            vec![format!("{duplicates} duplicate associations dropped")]
        } else {
            Vec::new()
        },
    };
    apply_outcome(ctx, TABLE, outcome)?;

    let mut migrated = 0i64;
    let mut skipped = 0i64;
    for chunk in valid.chunks(ctx.batch_size.max(1)) {
        let mut tx = ctx.pg.begin().await?;
        for row in chunk {
            let affected = sqlx::query(
                "INSERT INTO article_tags (article_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(row.article_id)
            .bind(row.tag_id)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!(
                    "failed to insert article_tags ({}, {})",
                    row.article_id, row.tag_id
                )
            })?
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
    report.skipped = skipped + duplicates;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupes_and_archives_dangling_pairs() {
        let rows = vec![
            LegacyTagArticle {
                article_id: 1,
                tag_id: 1,
            },
            LegacyTagArticle {
                article_id: 1,
                tag_id: 1,
            }, // 重复
            LegacyTagArticle {
                article_id: 1,
                tag_id: 99,
            }, // 悬挂 tag
            LegacyTagArticle {
                article_id: 99,
                tag_id: 1,
            }, // 悬挂 article
            LegacyTagArticle {
                article_id: 1,
                tag_id: 2,
            },
        ];
        let article_ids: HashSet<i64> = [1].into_iter().collect();
        let tag_ids: HashSet<i64> = [1, 2].into_iter().collect();
        let (valid, archives, duplicates) = transform(&rows, &article_ids, &tag_ids);
        assert_eq!(valid.len(), 2);
        assert_eq!(duplicates, 1);
        assert_eq!(archives.len(), 2);
    }
}
