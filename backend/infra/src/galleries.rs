//! 图库 Repository 的 PostgreSQL 实现。
//! 条目排序为事务内原子批量更新；媒体资产删除受限（FK RESTRICT）。

use crate::like::like_pattern;
use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use crate::where_clause::WhereBuilder;
use aries_core::galleries::{
    Gallery, GalleryError, GalleryItem, GalleryItemDetail, GalleryItemMedia, GalleryItemUpdate,
    GalleryListQuery, GalleryPage, GalleryRepository, GalleryUpdate, NewGallery, NewGalleryItem,
    PublicPhoto,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresGalleryRepository {
    pool: PgPool,
}

impl PostgresGalleryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct GalleryRow {
    id: i64,
    category_id: i64,
    slug: String,
    title: String,
    description: String,
    cover_media_id: Option<i64>,
    status: String,
    sort_order: i32,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<GalleryRow> for Gallery {
    type Error = GalleryError;

    fn try_from(row: GalleryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            category_id: row.category_id,
            slug: row.slug,
            title: row.title,
            description: row.description,
            cover_media_id: row.cover_media_id,
            status: row
                .status
                .parse()
                .map_err(|_| GalleryError::InvalidStatus)?,
            sort_order: row.sort_order,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// 条目 + 内联媒体摘要的行映射：LEFT JOIN media_assets（软删除资产媒体列为 NULL）。
#[derive(Debug, FromRow)]
struct GalleryItemRow {
    id: i64,
    gallery_id: i64,
    media_asset_id: i64,
    alt: String,
    location: String,
    sort_order: i32,
    created_at: OffsetDateTime,
    media_id: Option<i64>,
    media_url: Option<String>,
    media_alt: Option<String>,
    media_width: Option<i32>,
    media_height: Option<i32>,
}

impl From<GalleryItemRow> for GalleryItemDetail {
    fn from(row: GalleryItemRow) -> Self {
        let media = row.media_id.map(|id| GalleryItemMedia {
            id,
            url: row.media_url.unwrap_or_default(),
            alt: row.media_alt.unwrap_or_default(),
            width: row.media_width,
            height: row.media_height,
        });
        Self {
            item: GalleryItem {
                id: row.id,
                gallery_id: row.gallery_id,
                media_asset_id: row.media_asset_id,
                alt: row.alt,
                location: row.location,
                sort_order: row.sort_order,
                created_at: row.created_at,
            },
            media,
        }
    }
}

/// 照片墙查询的行映射：一条 JOIN 同时取条目、相册与媒体字段。
#[derive(Debug, FromRow)]
struct PublicPhotoRow {
    url: String,
    alt: String,
    location: String,
    width: Option<i32>,
    height: Option<i32>,
    gallery_slug: String,
    gallery_title: String,
    category_name: Option<String>,
}

impl From<PublicPhotoRow> for PublicPhoto {
    fn from(row: PublicPhotoRow) -> Self {
        Self {
            url: row.url,
            alt: row.alt,
            location: row.location,
            width: row.width,
            height: row.height,
            gallery_slug: row.gallery_slug,
            gallery_title: row.gallery_title,
            category_name: row.category_name,
        }
    }
}

const GALLERY_COLUMNS: &str = "id, category_id, slug::text AS slug, title, description, \
    cover_media_id, status, sort_order, created_at, updated_at";

/// 条目 + 内联媒体摘要的列清单（i 为 gallery_items，m 为 media_assets）。
const GALLERY_ITEM_DETAIL_COLUMNS: &str = "i.id, i.gallery_id, i.media_asset_id, i.alt, \
    i.location, i.sort_order, i.created_at, \
    m.id AS media_id, m.url AS media_url, m.alt AS media_alt, \
    m.width AS media_width, m.height AS media_height";

fn map_sqlx(error: sqlx::Error) -> GalleryError {
    if let sqlx::Error::Database(db) = &error {
        match db.code().as_deref() {
            // 23505 = unique_violation：slug 或 (gallery_id, media_asset_id) 冲突。
            Some("23505") => return GalleryError::Conflict,
            // 23503 = foreign_key_violation：分类/媒体不存在。
            Some("23503") => return GalleryError::NotFound,
            _ => {}
        }
    }
    tracing::error!(error = %error, "gallery repository operation failed");
    GalleryError::StoreUnavailable
}

#[async_trait]
impl GalleryRepository for PostgresGalleryRepository {
    async fn create_gallery(&self, gallery: NewGallery) -> Result<Gallery, GalleryError> {
        aries_core::galleries::validate_slug(&gallery.slug)?;
        aries_core::galleries::validate_title(&gallery.title)?;
        let query = format!(
            "INSERT INTO galleries \
             (category_id, slug, title, description, cover_media_id, status, sort_order) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             RETURNING {GALLERY_COLUMNS}"
        );
        let row = logged_query_as::<GalleryRow>(&query)
            .bind(gallery.category_id)
            .bind(&gallery.slug)
            .bind(&gallery.title)
            .bind(gallery.description.trim())
            .bind(gallery.cover_media_id)
            .bind(gallery.status.as_str())
            .bind(gallery.sort_order)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }

    async fn find_gallery(&self, gallery_id: i64) -> Result<Option<Gallery>, GalleryError> {
        let query =
            format!("SELECT {GALLERY_COLUMNS} FROM galleries WHERE id = $1 AND deleted_at IS NULL");
        let row = logged_query_as::<GalleryRow>(&query)
            .bind(gallery_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn find_published_by_slug(&self, slug: &str) -> Result<Option<Gallery>, GalleryError> {
        let query = format!(
            "SELECT {GALLERY_COLUMNS} FROM galleries \
             WHERE slug = $1::citext AND status = 'published' AND deleted_at IS NULL"
        );
        let row = logged_query_as::<GalleryRow>(&query)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn list_galleries(&self, query: GalleryListQuery) -> Result<GalleryPage, GalleryError> {
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
                 OR slug::text ILIKE {} ESCAPE '\\')",
                like_pattern(keyword),
            );
        }
        let clause = where_.clause();

        let count_sql = format!("SELECT COUNT(*) FROM galleries {clause}");
        let total = where_
            .bind_to(logged_query_scalar::<i64>(&count_sql))
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;

        let next_index = where_.next_index();
        let data_sql = format!(
            "SELECT {GALLERY_COLUMNS} FROM galleries {clause} \
             ORDER BY sort_order, id LIMIT ${next_index} OFFSET ${}",
            next_index + 1
        );
        let rows = where_
            .bind_to(logged_query_as::<GalleryRow>(&data_sql))
            .bind(limit)
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let items: Vec<Gallery> = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(GalleryPage {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }

    async fn list_public(&self, page: u32, page_size: u32) -> Result<GalleryPage, GalleryError> {
        // 复用 list 的过滤逻辑，固定 status = published。
        self.list_galleries(GalleryListQuery {
            page,
            page_size,
            category_id: None,
            status: Some(aries_core::galleries::GalleryStatus::Published),
            keyword: None,
        })
        .await
    }

    async fn list_public_photos(&self) -> Result<Vec<PublicPhoto>, GalleryError> {
        // 照片墙：只取 published 且未软删相册的条目；媒体 JOIN 过滤软删除资产
        // （已删除媒体的 URL 不可再展示）；
        // 分类 LEFT JOIN，相册分类被删时 category_name 为 NULL。
        let rows = logged_query_as::<PublicPhotoRow>(
            "SELECT m.url, i.alt, i.location, m.width, m.height, \
                    g.slug::text AS gallery_slug, g.title AS gallery_title, \
                    c.name AS category_name \
             FROM gallery_items i \
             JOIN galleries g ON g.id = i.gallery_id \
                AND g.status = 'published' AND g.deleted_at IS NULL \
             JOIN media_assets m ON m.id = i.media_asset_id AND m.deleted_at IS NULL \
             LEFT JOIN categories c ON c.id = g.category_id AND c.deleted_at IS NULL \
             ORDER BY g.sort_order, g.id, i.sort_order, i.id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_gallery(
        &self,
        gallery_id: i64,
        update: GalleryUpdate,
    ) -> Result<Gallery, GalleryError> {
        aries_core::galleries::validate_slug(&update.slug)?;
        aries_core::galleries::validate_title(&update.title)?;
        let query = format!(
            "UPDATE galleries SET category_id = $2, slug = $3, title = $4, description = $5, \
             cover_media_id = $6, status = $7, sort_order = $8, updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL \
             RETURNING {GALLERY_COLUMNS}"
        );
        let row = logged_query_as::<GalleryRow>(&query)
            .bind(gallery_id)
            .bind(update.category_id)
            .bind(&update.slug)
            .bind(&update.title)
            .bind(update.description.trim())
            .bind(update.cover_media_id)
            .bind(update.status.as_str())
            .bind(update.sort_order)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(GalleryError::NotFound)?;
        row.try_into()
    }

    async fn delete_gallery(&self, gallery_id: i64) -> Result<(), GalleryError> {
        // 软删除图库本身；条目保留（图库恢复时仍然可用）。
        let result = logged_query(
            "UPDATE galleries SET deleted_at = now(), updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(gallery_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(GalleryError::NotFound);
        }
        Ok(())
    }

    async fn list_items(&self, gallery_id: i64) -> Result<Vec<GalleryItemDetail>, GalleryError> {
        // LEFT JOIN 一次查全媒体摘要，消除逐条查询的 N+1；软删除资产媒体列为 NULL。
        let query = format!(
            "SELECT {GALLERY_ITEM_DETAIL_COLUMNS} FROM gallery_items i \
             LEFT JOIN media_assets m ON m.id = i.media_asset_id AND m.deleted_at IS NULL \
             WHERE i.gallery_id = $1 ORDER BY i.sort_order, i.id"
        );
        let rows = logged_query_as::<GalleryItemRow>(&query)
            .bind(gallery_id)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn add_item(&self, item: NewGalleryItem) -> Result<GalleryItemDetail, GalleryError> {
        // CTE：插入与媒体摘要一次 round trip 返回。
        let query = format!(
            "WITH inserted AS ( \
                INSERT INTO gallery_items (gallery_id, media_asset_id, alt, location, sort_order) \
                VALUES ($1, $2, $3, $4, $5) \
                RETURNING * \
             ) \
             SELECT {GALLERY_ITEM_DETAIL_COLUMNS} FROM inserted i \
             LEFT JOIN media_assets m ON m.id = i.media_asset_id AND m.deleted_at IS NULL"
        );
        let row = logged_query_as::<GalleryItemRow>(&query)
            .bind(item.gallery_id)
            .bind(item.media_asset_id)
            .bind(item.alt.trim())
            .bind(item.location.trim())
            .bind(item.sort_order)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(row.into())
    }

    async fn update_item(
        &self,
        item_id: i64,
        update: GalleryItemUpdate,
    ) -> Result<GalleryItemDetail, GalleryError> {
        let query = format!(
            "WITH updated AS ( \
                UPDATE gallery_items SET alt = $2, location = $3, sort_order = $4 \
                WHERE id = $1 \
                RETURNING * \
             ) \
             SELECT {GALLERY_ITEM_DETAIL_COLUMNS} FROM updated i \
             LEFT JOIN media_assets m ON m.id = i.media_asset_id AND m.deleted_at IS NULL"
        );
        let row = logged_query_as::<GalleryItemRow>(&query)
            .bind(item_id)
            .bind(update.alt.trim())
            .bind(update.location.trim())
            .bind(update.sort_order)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(GalleryError::ItemNotFound)?;
        Ok(row.into())
    }

    async fn remove_item(&self, item_id: i64) -> Result<(), GalleryError> {
        let result = logged_query("DELETE FROM gallery_items WHERE id = $1")
            .bind(item_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(GalleryError::ItemNotFound);
        }
        Ok(())
    }

    async fn reorder_items(
        &self,
        gallery_id: i64,
        ordered_item_ids: Vec<i64>,
    ) -> Result<(), GalleryError> {
        // 事务内按入参顺序重写 sort_order，保证批量排序原子生效。
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        if ordered_item_ids.is_empty() {
            tx.commit().await.map_err(map_sqlx)?;
            return Ok(());
        }
        let sort_orders: Vec<i32> = (0..ordered_item_ids.len())
            .map(|index| i32::try_from(index).unwrap_or(i32::MAX))
            .collect();
        let result = logged_query(
            "UPDATE gallery_items SET sort_order = data.sort_order \
             FROM (SELECT unnest($1::bigint[]) AS id, unnest($2::int[]) AS sort_order) AS data \
             WHERE gallery_items.id = data.id AND gallery_items.gallery_id = $3",
        )
        .bind(&ordered_item_ids)
        .bind(&sort_orders)
        .bind(gallery_id)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() != ordered_item_ids.len() as u64 {
            return Err(GalleryError::ItemNotFound);
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, ensure};
    use aries_core::galleries::GalleryStatus;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn postgresql_repository_reorders_items_with_aligned_unnest_arrays() -> anyhow::Result<()>
    {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let _ = dotenvy::dotenv();
        let base_config = crate::PostgresConfig::from_env()?;
        let admin_pool = crate::connect_postgres(&base_config).await?;
        let test_schema = format!("aries_test_{}", Uuid::now_v7().simple());

        // 随机 Schema 隔离业务数据，测试无论成功或失败都会执行清理。
        logged_query(&format!("CREATE SCHEMA \"{test_schema}\""))
            .execute(&admin_pool)
            .await
            .context("failed to create isolated gallery test schema")?;
        let test_config = crate::PostgresConfig {
            options: base_config.options.clone(),
            schema: format!("{test_schema},{}", base_config.schema()),
            slow_query_ms: 0,
        };
        let test_pool = crate::connect_postgres(&test_config).await?;
        let scenario_result = async {
            crate::run_migrations(&test_pool).await?;
            run_reorder_scenario(&test_pool).await
        }
        .await;

        test_pool.close().await;
        let cleanup_result = logged_query(&format!("DROP SCHEMA \"{test_schema}\" CASCADE"))
            .execute(&admin_pool)
            .await
            .context("failed to remove isolated gallery test schema");
        admin_pool.close().await;

        scenario_result?;
        cleanup_result?;
        Ok(())
    }

    async fn run_reorder_scenario(pool: &PgPool) -> anyhow::Result<()> {
        let user_id = logged_query_scalar::<i64>(
            "INSERT INTO users \
             (username, email, password_hash, display_name, role, status) \
             VALUES ('uploader', 'uploader@example.com', 'hash', 'Uploader', 'editor', 'active') \
             RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        let category_id = logged_query_scalar::<i64>(
            "INSERT INTO categories (kind, name, slug) \
             VALUES ('gallery', 'Travel', 'travel') RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        let repository = PostgresGalleryRepository::new(pool.clone());
        let gallery = repository
            .create_gallery(NewGallery {
                category_id,
                slug: "reorder-gallery".to_owned(),
                title: "Reorder".to_owned(),
                description: String::new(),
                cover_media_id: None,
                status: GalleryStatus::Published,
                sort_order: 0,
            })
            .await?;

        let mut item_ids = Vec::new();
        for index in 0..3 {
            let object_key = format!("reorder-{}-{index}", gallery.id);
            let asset_id = logged_query_scalar::<i64>(
                "INSERT INTO media_assets \
                 (provider, object_key, url, original_name, mime, size_bytes, sha256, uploaded_by) \
                 VALUES ('local', $1, 'https://example.com/a.jpg', 'a.jpg', 'image/jpeg', 1, $1, $2) \
                 RETURNING id",
            )
            .bind(&object_key)
            .bind(user_id)
            .fetch_one(pool)
            .await?;
            let item = repository
                .add_item(NewGalleryItem {
                    gallery_id: gallery.id,
                    media_asset_id: asset_id,
                    alt: String::new(),
                    location: String::new(),
                    sort_order: index,
                })
                .await?;
            item_ids.push(item.item.id);
        }

        // 乱序批量重排：验证 unnest 并行数组按位置对齐，而非对 id 排序。
        let mut shuffled = item_ids.clone();
        shuffled.reverse();
        repository
            .reorder_items(gallery.id, shuffled.clone())
            .await?;
        let reordered = repository.list_items(gallery.id).await?;
        let ordered_ids: Vec<i64> = reordered.iter().map(|detail| detail.item.id).collect();
        ensure!(ordered_ids == shuffled);
        ensure!(
            reordered
                .iter()
                .enumerate()
                .all(|(index, detail)| detail.item.sort_order
                    == i32::try_from(index).unwrap_or(i32::MAX))
        );

        // 混入不存在条目时保持 NotFound 语义，且不产生部分写入。
        let mut bogus = shuffled.clone();
        bogus[1] = i64::MAX;
        ensure!(matches!(
            repository.reorder_items(gallery.id, bogus).await,
            Err(GalleryError::ItemNotFound)
        ));
        let after_failed: Vec<i64> = repository
            .list_items(gallery.id)
            .await?
            .into_iter()
            .map(|detail| detail.item.id)
            .collect();
        ensure!(after_failed == shuffled);

        // 空入参为无操作，不发出批量 UPDATE。
        repository.reorder_items(gallery.id, Vec::new()).await?;
        Ok(())
    }
}
