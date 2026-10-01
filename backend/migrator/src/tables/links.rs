//! `links` → `links`：默认 `active`；category_id 0/悬挂 → NULL（repaired）；
//! 非 http/https URL 不丢弃——原样入库并在 Report 列出（URL 协议为应用层校验，DB 无 CHECK）。

use std::collections::HashSet;

use anyhow::Context;
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use super::{
    ArchiveEntry, CommonParams, TransformOutcome, apply_outcome, enforce_length, enforce_nonempty,
    optional_time, required_time,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "links";

const MAX_TITLE_CHARS: usize = 100;
const MAX_URL_CHARS: usize = 2048;

#[derive(Debug, Clone, FromRow)]
pub struct LegacyLink {
    pub id: i64,
    pub category_id: i64,
    pub name: String,
    pub url: String,
    pub desc: String,
    pub icon: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewLink {
    pub id: i64,
    pub category_id: Option<i64>,
    pub title: String,
    pub url: String,
    pub icon_url: Option<String>,
    pub description: String,
    pub status: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyLink>> {
    sqlx::query_as::<_, LegacyLink>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(category_id AS SIGNED) AS category_id, \
         name, url, `desc`, icon, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM links ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy links")
}

pub fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

pub fn transform(
    row: &LegacyLink,
    params: &CommonParams,
    link_category_ids: &HashSet<i64>,
) -> anyhow::Result<TransformOutcome<NewLink>> {
    let mut archives = Vec::new();
    let mut repaired = 0i64;
    let mut notes = Vec::new();

    let (value, archive_a, fix_a) =
        enforce_nonempty(params, TABLE, row.id, "name", row.name.clone(), || {
            format!("link-{}", row.id)
        })?;
    let (title, archive_b, fix_b) =
        enforce_length(params, TABLE, row.id, "name", value, MAX_TITLE_CHARS)?;
    for entry in [archive_a, archive_b].into_iter().flatten() {
        archives.push(entry);
    }
    repaired += fix_a + fix_b;

    let (value, archive_a, fix_a) =
        enforce_nonempty(params, TABLE, row.id, "url", row.url.clone(), || {
            "about:blank".to_owned()
        })?;
    let (url, archive_b, fix_b) =
        enforce_length(params, TABLE, row.id, "url", value, MAX_URL_CHARS)?;
    for entry in [archive_a, archive_b].into_iter().flatten() {
        archives.push(entry);
    }
    repaired += fix_a + fix_b;

    if !is_http_url(&url) {
        notes.push(format!("#{}: non-http(s) url kept as-is: {url}", row.id));
    }

    let category_id = match row.category_id {
        0 => None,
        id if link_category_ids.contains(&id) => Some(id),
        dangling => {
            repaired += 1;
            archives.push(ArchiveEntry {
                id: Some(row.id),
                fields: serde_json::json!({ "category_id": dangling }),
                reason: "dangling or non-link category reference; reset to NULL".to_owned(),
                whole_row: false,
            });
            None
        }
    };

    let icon_url = if row.icon.trim().is_empty() {
        None
    } else {
        let (value, archive, fix) = enforce_length(
            params,
            TABLE,
            row.id,
            "icon",
            row.icon.clone(),
            MAX_URL_CHARS,
        )?;
        if let Some(entry) = archive {
            archives.push(entry);
        }
        repaired += fix;
        Some(value)
    };

    Ok(TransformOutcome {
        row: Some(NewLink {
            id: row.id,
            category_id,
            title,
            url,
            icon_url,
            description: row.desc.clone(),
            status: "active".to_owned(),
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
    // 友链分类为 kind='link'（旧 type=1）的分类。
    let link_category_ids = fetch_link_category_ids(&ctx.mysql).await?;

    let mut new_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let outcome = transform(row, &params, &link_category_ids)?;
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
                "INSERT INTO links (id, category_id, title, url, icon_url, description, status, \
                 created_at, updated_at, deleted_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(row.category_id)
            .bind(&row.title)
            .bind(&row.url)
            .bind(&row.icon_url)
            .bind(&row.description)
            .bind(&row.status)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert link #{}", row.id))?
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

async fn fetch_link_category_ids(mysql: &MySqlPool) -> anyhow::Result<HashSet<i64>> {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT CAST(id AS SIGNED) FROM categories WHERE `type` = 1")
            .fetch_all(mysql)
            .await
            .context("failed to fetch link category ids")?;
    Ok(ids.into_iter().collect())
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

    fn legacy(id: i64, category_id: i64) -> LegacyLink {
        LegacyLink {
            id,
            category_id,
            name: "示例".to_owned(),
            url: "https://example.com".to_owned(),
            desc: "描述".to_owned(),
            icon: String::new(),
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    #[test]
    fn zero_or_dangling_category_becomes_null() {
        let ids: HashSet<i64> = [5].into_iter().collect();
        let outcome = transform(&legacy(1, 0), &params(), &ids).unwrap();
        assert_eq!(outcome.row.unwrap().category_id, None);
        assert_eq!(outcome.repaired, 0);

        let outcome = transform(&legacy(1, 99), &params(), &ids).unwrap();
        assert_eq!(outcome.row.unwrap().category_id, None);
        assert_eq!(outcome.repaired, 1);

        let outcome = transform(&legacy(1, 5), &params(), &ids).unwrap();
        assert_eq!(outcome.row.unwrap().category_id, Some(5));
    }

    #[test]
    fn non_http_url_kept_with_note() {
        let ids: HashSet<i64> = HashSet::new();
        let mut row = legacy(1, 0);
        row.url = "ftp://example.com".to_owned();
        let outcome = transform(&row, &params(), &ids).unwrap();
        assert_eq!(outcome.row.unwrap().url, "ftp://example.com");
        assert_eq!(outcome.notes.len(), 1);
        assert!(!is_http_url("ftp://example.com"));
        assert!(is_http_url("https://example.com"));
    }

    #[test]
    fn defaults_active_status() {
        let ids: HashSet<i64> = HashSet::new();
        let outcome = transform(&legacy(1, 0), &params(), &ids).unwrap();
        assert_eq!(outcome.row.unwrap().status, "active");
    }
}
