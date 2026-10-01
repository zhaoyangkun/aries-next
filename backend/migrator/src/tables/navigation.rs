//! `navs` → `navigation_items`：全部映射为 `target_type='url'`；`parent_nav_id=0`/悬挂 → NULL；
//! `icon` 进 Archive；目标无 `deleted_at` 列，旧库软删除行整行归档不迁入。

use std::collections::HashSet;

use anyhow::Context;
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use super::{
    ArchiveEntry, CommonParams, TransformOutcome, apply_outcome, archive_unmapped_field,
    enforce_length, enforce_nonempty, required_time, tinyint_to_bool,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "navigation_items";
/// 源表名（Report 中 source_rows 以此为准）。
const SOURCE_TABLE: &str = "navs";

const MAX_LABEL_CHARS: usize = 60;
const MAX_URL_CHARS: usize = 2048;

#[derive(Debug, Clone, FromRow)]
pub struct LegacyNav {
    pub id: i64,
    pub parent_nav_id: i64,
    pub order_id: i64,
    pub open_type: i64,
    pub name: String,
    pub url: String,
    pub icon: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewNavItem {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub label: String,
    pub url: String,
    pub open_in_new_tab: bool,
    pub visible: bool,
    pub sort_order: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyNav>> {
    sqlx::query_as::<_, LegacyNav>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(parent_nav_id AS SIGNED) AS parent_nav_id, \
         CAST(order_id AS SIGNED) AS order_id, CAST(open_type AS SIGNED) AS open_type, \
         name, url, icon, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM navs ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy navs")
}

pub fn transform(
    row: &LegacyNav,
    params: &CommonParams,
    valid_ids: &HashSet<i64>,
) -> anyhow::Result<TransformOutcome<NewNavItem>> {
    // 目标 navigation_items 无 deleted_at 列；旧库软删除行整行归档。
    if row.deleted_at.is_some() {
        return Ok(TransformOutcome::archive_row(
            Some(row.id),
            serde_json::json!({
                "name": row.name,
                "url": row.url,
                "deleted_at": row.deleted_at,
            }),
            "target navigation_items table has no deleted_at column",
        ));
    }

    let mut archives = Vec::new();
    let mut repaired = 0i64;

    archive_unmapped_field(
        &mut archives,
        row.id,
        "icon",
        &row.icon,
        "target navigation_items table has no icon column",
    );

    let (value, archive_a, fix_a) = enforce_nonempty(
        params,
        SOURCE_TABLE,
        row.id,
        "name",
        row.name.clone(),
        || format!("nav-{}", row.id),
    )?;
    let (label, archive_b, fix_b) =
        enforce_length(params, SOURCE_TABLE, row.id, "name", value, MAX_LABEL_CHARS)?;
    for entry in [archive_a, archive_b].into_iter().flatten() {
        archives.push(entry);
    }
    repaired += fix_a + fix_b;

    let (url, archive, fix) = enforce_length(
        params,
        SOURCE_TABLE,
        row.id,
        "url",
        row.url.clone(),
        MAX_URL_CHARS,
    )?;
    if let Some(entry) = archive {
        archives.push(entry);
    }
    repaired += fix;

    let parent_id = match row.parent_nav_id {
        0 => None,
        id if valid_ids.contains(&id) && id != row.id => Some(id),
        dangling => {
            repaired += 1;
            archives.push(ArchiveEntry {
                id: Some(row.id),
                fields: serde_json::json!({ "parent_nav_id": dangling }),
                reason: "dangling or self parent reference; reset to top level".to_owned(),
                whole_row: false,
            });
            None
        }
    };

    Ok(TransformOutcome {
        row: Some(NewNavItem {
            id: row.id,
            parent_id,
            label,
            // CHECK 要求 url 类型必须有 url；空串满足 NOT NULL，语义问题由 Preflight/Report 暴露。
            url,
            open_in_new_tab: tinyint_to_bool(row.open_type, SOURCE_TABLE, row.id, "open_type")?,
            visible: true,
            sort_order: i32::try_from(row.order_id)
                .context("navs: order_id out of integer range")?,
            created_at: required_time(
                row.created_at.as_deref(),
                params.offset,
                SOURCE_TABLE,
                row.id,
                "created_at",
            )?,
            updated_at: required_time(
                row.updated_at.as_deref(),
                params.offset,
                SOURCE_TABLE,
                row.id,
                "updated_at",
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
                "INSERT INTO navigation_items (id, parent_id, label, target_type, url, \
                 open_in_new_tab, visible, sort_order, created_at, updated_at) \
                 VALUES ($1, $2, $3, 'url', $4, $5, $6, $7, $8, $9) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(row.parent_id)
            .bind(&row.label)
            .bind(&row.url)
            .bind(row.open_in_new_tab)
            .bind(row.visible)
            .bind(row.sort_order)
            .bind(row.created_at)
            .bind(row.updated_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert navigation item #{}", row.id))?
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

    fn legacy(id: i64, parent: i64) -> LegacyNav {
        LegacyNav {
            id,
            parent_nav_id: parent,
            order_id: 0,
            open_type: 1,
            name: "首页".to_owned(),
            url: "/".to_owned(),
            icon: "home".to_owned(),
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    #[test]
    fn maps_to_url_target_with_new_tab_flag() {
        let ids: HashSet<i64> = [1].into_iter().collect();
        let outcome = transform(&legacy(1, 0), &params(), &ids).unwrap();
        let row = outcome.row.unwrap();
        assert!(row.open_in_new_tab);
        assert!(row.visible);
        assert_eq!(row.parent_id, None);
        // icon 进 Archive。
        assert!(
            outcome
                .archives
                .iter()
                .any(|a| a.fields.get("icon").is_some())
        );
    }

    #[test]
    fn dangling_parent_repaired_to_top_level() {
        let ids: HashSet<i64> = [1].into_iter().collect();
        let outcome = transform(&legacy(1, 99), &params(), &ids).unwrap();
        assert_eq!(outcome.row.unwrap().parent_id, None);
        assert_eq!(outcome.repaired, 1);
    }

    #[test]
    fn soft_deleted_nav_archived_not_migrated() {
        let ids: HashSet<i64> = [1].into_iter().collect();
        let mut row = legacy(1, 0);
        row.deleted_at = Some("2021-01-01 00:00:00".to_owned());
        let outcome = transform(&row, &params(), &ids).unwrap();
        assert!(outcome.row.is_none());
        assert_eq!(outcome.archives.len(), 1);
    }
}
