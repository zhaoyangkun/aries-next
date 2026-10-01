//! 图库两步映射：
//! - `categories` 中 type=2 的行 → `galleries`（相册实体，id 复用分类 id，status='published'）。
//! - 旧 `galleries` 每行一张图 → 先落 `media_assets`（复用 media.rs 的下载/外链策略，
//!   id 从 max(pictures.id)+1 起顺序分配）再落 `gallery_items`（id 复用旧行 id）。
//!   同图库内相同图片 URL 的重复行进 Archive。

use std::collections::HashSet;

use anyhow::Context;
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use super::{TransformOutcome, apply_outcome, media, optional_time, required_time};
use crate::context::MigrateCtx;

/// Report 中相册实体与条目分开统计。
pub const GALLERIES_TABLE: &str = "galleries";
pub const ITEMS_TABLE: &str = "gallery_items";

#[derive(Debug, Clone, FromRow)]
pub struct LegacyGalleryRow {
    pub id: i64,
    pub category_id: i64,
    pub url: String,
    pub name: String,
    pub desc: String,
    pub location: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewGallery {
    pub id: i64,
    pub category_id: i64,
    pub slug: String,
    pub title: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewGalleryItem {
    pub id: i64,
    pub gallery_id: i64,
    pub media_asset_id: i64,
    pub alt: String,
    pub location: String,
    pub created_at: OffsetDateTime,
}

pub async fn extract_rows(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyGalleryRow>> {
    sqlx::query_as::<_, LegacyGalleryRow>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(category_id AS SIGNED) AS category_id, \
         url, name, `desc`, location, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM galleries ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy galleries")
}

async fn extract_gallery_categories(
    mysql: &MySqlPool,
) -> anyhow::Result<Vec<super::categories::LegacyCategory>> {
    sqlx::query_as::<_, super::categories::LegacyCategory>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(parent_id AS SIGNED) AS parent_id, \
         CAST(`type` AS SIGNED) AS `type`, name, url, CAST(count AS SIGNED) AS count, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM categories WHERE `type` = 2 ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract gallery categories")
}

/// 图库分类 → 相册实体（纯函数）。
pub fn transform_gallery(
    category: &super::categories::LegacyCategory,
    offset: time::UtcOffset,
) -> anyhow::Result<TransformOutcome<NewGallery>> {
    let slug = if category.url.trim().is_empty() {
        format!("gallery-{}", category.id)
    } else {
        category.url.clone()
    };
    Ok(TransformOutcome::ok(NewGallery {
        id: category.id,
        category_id: category.id,
        slug,
        title: category.name.clone(),
        created_at: required_time(
            category.created_at.as_deref(),
            offset,
            GALLERIES_TABLE,
            category.id,
            "created_at",
        )?,
        updated_at: required_time(
            category.updated_at.as_deref(),
            offset,
            GALLERIES_TABLE,
            category.id,
            "updated_at",
        )?,
        deleted_at: optional_time(
            category.deleted_at.as_deref(),
            offset,
            GALLERIES_TABLE,
            category.id,
            "deleted_at",
        )?,
    }))
}

pub async fn migrate(ctx: &mut MigrateCtx) -> anyhow::Result<()> {
    let owner_id = ctx.owner_id().await?;

    // 第一步：图库分类 → galleries 相册实体。
    let categories = extract_gallery_categories(&ctx.mysql).await?;
    ctx.report.table(GALLERIES_TABLE).source_rows = categories.len() as i64;
    let gallery_category_ids: HashSet<i64> = categories.iter().map(|row| row.id).collect();

    let mut galleries = Vec::with_capacity(categories.len());
    for category in &categories {
        let outcome = transform_gallery(category, ctx.offset)?;
        if let Some(row) = apply_outcome(ctx, GALLERIES_TABLE, outcome)? {
            galleries.push(row);
        }
    }

    let mut migrated = 0i64;
    let mut skipped = 0i64;
    for chunk in galleries.chunks(ctx.batch_size.max(1)) {
        let mut tx = ctx.pg.begin().await?;
        for row in chunk {
            let affected = sqlx::query(
                "INSERT INTO galleries (id, category_id, slug, title, status, \
                 created_at, updated_at, deleted_at) \
                 VALUES ($1, $2, $3, $4, 'published', $5, $6, $7) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(row.category_id)
            .bind(&row.slug)
            .bind(&row.title)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert gallery #{}", row.id))?
            .rows_affected();
            if affected == 1 {
                migrated += 1;
            } else {
                skipped += 1;
            }
        }
        tx.commit().await?;
    }
    let report = ctx.report.table(GALLERIES_TABLE);
    report.migrated = migrated;
    report.skipped = skipped;

    // 第二步：旧 galleries 行 → media_assets + gallery_items。
    let rows = extract_rows(&ctx.mysql).await?;
    ctx.report.table(ITEMS_TABLE).source_rows = rows.len() as i64;

    // 媒体行 id 命名空间：max(pictures.id) 起顺序分配，避免与 pictures 迁移行冲突。
    let max_picture_id: i64 =
        sqlx::query_scalar("SELECT CAST(COALESCE(MAX(id), 0) AS SIGNED) FROM pictures")
            .fetch_one(&ctx.mysql)
            .await
            .context("failed to read max pictures id")?;
    let mut next_media_id = max_picture_id + 1;

    let mut seen_urls: HashSet<(i64, String)> = HashSet::new();
    let mut media_rows = Vec::new();
    let mut items = Vec::new();
    let mut media_notes = Vec::new();

    for row in &rows {
        // 分类不是图库分类（悬挂或 0）：整行归档。
        if !gallery_category_ids.contains(&row.category_id) {
            ctx.archive_row(
                ITEMS_TABLE,
                Some(row.id),
                serde_json::json!({
                    "category_id": row.category_id,
                    "url": row.url,
                    "name": row.name,
                }),
                "category is not a gallery category (dangling or missing)",
                true,
            )?;
            continue;
        }
        // 同图库相同 URL 去重（目标 UNIQUE (gallery_id, media_asset_id) 无法表达，按 URL 判重）。
        if !seen_urls.insert((row.category_id, row.url.clone())) {
            ctx.archive_row(
                ITEMS_TABLE,
                Some(row.id),
                serde_json::json!({
                    "category_id": row.category_id,
                    "url": row.url,
                    "name": row.name,
                }),
                "duplicate image url within the same gallery",
                true,
            )?;
            continue;
        }

        let created_at = required_time(
            row.created_at.as_deref(),
            ctx.offset,
            ITEMS_TABLE,
            row.id,
            "created_at",
        )?;
        let updated_at = required_time(
            row.updated_at.as_deref(),
            ctx.offset,
            ITEMS_TABLE,
            row.id,
            "updated_at",
        )?;
        let deleted_at = optional_time(
            row.deleted_at.as_deref(),
            ctx.offset,
            ITEMS_TABLE,
            row.id,
            "deleted_at",
        )?;

        let media_id = next_media_id;
        next_media_id += 1;
        let (asset, note) = media::build_media_asset(
            &ctx.media,
            media::MediaAssetInput {
                id: media_id,
                legacy_object_prefix: "galleries",
                source_url: &row.url,
                file_name: &row.name,
                size_kb: 0,
                legacy_hash: "",
                uploaded_by: owner_id,
                created_at,
                updated_at,
                deleted_at,
            },
        )
        .await;
        if let Some(note) = note {
            media_notes.push(format!("galleries row #{id}: {note}", id = row.id));
        }
        media_rows.push(asset);
        items.push(NewGalleryItem {
            id: row.id,
            gallery_id: row.category_id,
            media_asset_id: media_id,
            alt: row.desc.clone(),
            location: row.location.clone(),
            created_at,
        });
    }

    let (media_migrated, media_skipped) =
        media::insert_media_assets(&ctx.pg, &media_rows, ctx.batch_size).await?;
    // 图库媒体行并入 media_assets 表统计。
    let report = ctx.report.table(super::media::TABLE);
    report.source_rows += 0; // 来源是 galleries 表，不重复计 source_rows
    report.migrated += media_migrated;
    report.skipped += media_skipped;
    let legacy_count = media_rows
        .iter()
        .filter(|row| row.provider == "legacy_url")
        .count() as i64;
    report.repaired += legacy_count;
    for note in media_notes {
        report.note(note);
    }

    let mut migrated = 0i64;
    let mut skipped = 0i64;
    for chunk in items.chunks(ctx.batch_size.max(1)) {
        let mut tx = ctx.pg.begin().await?;
        for row in chunk {
            let affected = sqlx::query(
                "INSERT INTO gallery_items (id, gallery_id, media_asset_id, alt, location, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(row.gallery_id)
            .bind(row.media_asset_id)
            .bind(&row.alt)
            .bind(&row.location)
            .bind(row.created_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert gallery item #{}", row.id))?
            .rows_affected();
            if affected == 1 {
                migrated += 1;
            } else {
                skipped += 1;
            }
        }
        tx.commit().await?;
    }
    let report = ctx.report.table(ITEMS_TABLE);
    report.migrated = migrated;
    report.skipped = skipped;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::categories::LegacyCategory;
    use super::*;
    use time::UtcOffset;

    #[test]
    fn gallery_category_becomes_gallery_entity() {
        let category = LegacyCategory {
            id: 3,
            parent_id: 0,
            r#type: 2,
            name: "旅行相册".to_owned(),
            url: "travel".to_owned(),
            count: 0,
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        };
        let outcome = transform_gallery(&category, UtcOffset::UTC).unwrap();
        let row = outcome.row.unwrap();
        assert_eq!(row.id, 3);
        assert_eq!(row.category_id, 3);
        assert_eq!(row.slug, "travel");
        assert_eq!(row.title, "旅行相册");
    }

    #[test]
    fn empty_category_slug_generates_fallback() {
        let category = LegacyCategory {
            id: 4,
            parent_id: 0,
            r#type: 2,
            name: "相册".to_owned(),
            url: String::new(),
            count: 0,
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        };
        let outcome = transform_gallery(&category, UtcOffset::UTC).unwrap();
        assert_eq!(outcome.row.unwrap().slug, "gallery-4");
    }
}
