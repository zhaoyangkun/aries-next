//! 媒体存储实现：PostgresMediaRepository（引用维护、清理查询）与本地/S3 存储后端。
use std::{collections::HashSet, str::FromStr};

use crate::like::like_pattern;
use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use aries_core::media::{
    MediaAsset, MediaBatchDeleteResult, MediaError, MediaListQuery, MediaPage, MediaProvider,
    MediaRepository, MediaStatus, MediaUpdate, MediaUsage, NewMediaAsset, UsageTargetType,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresMediaRepository {
    pool: PgPool,
}

impl PostgresMediaRepository {
    /// 新建仓储实例，持有连接池。
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, FromRow)]
struct MediaAssetRow {
    id: i64,
    provider: String,
    object_key: String,
    url: String,
    original_name: String,
    mime: String,
    size_bytes: i64,
    width: Option<i32>,
    height: Option<i32>,
    sha256: String,
    alt: String,
    status: String,
    uploaded_by: i64,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    deleted_at: Option<OffsetDateTime>,
}

impl TryFrom<MediaAssetRow> for MediaAsset {
    type Error = MediaError;

    /// 查询行 → 领域对象：provider/status 字符串解析为枚举。
    fn try_from(row: MediaAssetRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            provider: MediaProvider::from_str(&row.provider)?,
            object_key: row.object_key,
            url: row.url,
            original_name: row.original_name,
            mime: row.mime,
            size_bytes: row.size_bytes,
            width: row.width,
            height: row.height,
            sha256: row.sha256,
            alt: row.alt,
            status: MediaStatus::from_str(&row.status)?,
            uploaded_by: row.uploaded_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
        })
    }
}

const ASSET_COLUMNS: &str = "id, provider, object_key, url, original_name, mime, size_bytes, \
    width, height, sha256, alt, status, uploaded_by, created_at, updated_at, deleted_at";

#[derive(Debug, FromRow)]
struct MediaUsageRow {
    id: i64,
    asset_id: i64,
    target_type: String,
    target_id: i64,
    created_at: OffsetDateTime,
}

impl TryFrom<MediaUsageRow> for MediaUsage {
    type Error = MediaError;

    /// 查询行 → 领域对象：target_type 字符串解析为枚举。
    fn try_from(row: MediaUsageRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            asset_id: row.asset_id,
            target_type: UsageTargetType::from_str(&row.target_type)?,
            target_id: row.target_id,
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl MediaRepository for PostgresMediaRepository {
    /// 登记新资产（元信息 + 存储 object_key）。
    async fn create(&self, asset: NewMediaAsset) -> Result<MediaAsset, MediaError> {
        let query = format!(
            "INSERT INTO media_assets (provider, object_key, url, original_name, mime, \
             size_bytes, width, height, sha256, alt, uploaded_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING {ASSET_COLUMNS}"
        );
        let row = logged_query_as::<MediaAssetRow>(&query)
            .bind(asset.provider.as_str())
            .bind(asset.object_key)
            .bind(asset.url)
            .bind(asset.original_name)
            .bind(asset.mime)
            .bind(asset.size_bytes)
            .bind(asset.width)
            .bind(asset.height)
            .bind(asset.sha256)
            .bind(asset.alt)
            .bind(asset.uploaded_by)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }

    /// 按 id 查询（含已软删，管理端详情/清理用）。
    async fn find(&self, asset_id: i64) -> Result<Option<MediaAsset>, MediaError> {
        let query = format!("SELECT {ASSET_COLUMNS} FROM media_assets WHERE id = $1");
        let row = logged_query_as::<MediaAssetRow>(&query)
            .bind(asset_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    /// 管理端分页列表：仅 active；count + 列表两条语句，created_at DESC, id DESC 稳定排序。
    async fn list(&self, query: MediaListQuery) -> Result<MediaPage, MediaError> {
        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, 100);
        let offset = i64::from(page - 1) * i64::from(page_size);
        let provider = query.provider.map(|value| value.as_str().to_owned());
        let keyword = query
            .keyword
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|value| like_pattern(&value));
        let mime = query
            .mime
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let total = logged_query_scalar::<i64>(
            "SELECT count(*) FROM media_assets WHERE status = 'active' \
             AND ($1::text IS NULL OR provider = $1) \
             AND ($2::text IS NULL OR mime = $2) \
             AND ($3::text IS NULL OR original_name ILIKE $3 ESCAPE '\\')",
        )
        .bind(provider.as_deref())
        .bind(mime.as_deref())
        .bind(keyword.as_deref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        // 稳定排序：created_at DESC 相同时按 id DESC，分页结果不会因并列抖动。
        let list_query = format!(
            "SELECT {ASSET_COLUMNS} FROM media_assets WHERE status = 'active' \
             AND ($1::text IS NULL OR provider = $1) \
             AND ($2::text IS NULL OR mime = $2) \
             AND ($3::text IS NULL OR original_name ILIKE $3 ESCAPE '\\') \
             ORDER BY created_at DESC, id DESC LIMIT $4 OFFSET $5"
        );
        let rows = logged_query_as::<MediaAssetRow>(&list_query)
            .bind(provider.as_deref())
            .bind(mime.as_deref())
            .bind(keyword.as_deref())
            .bind(i64::from(page_size))
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let items = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(MediaPage {
            items,
            total,
            page,
            page_size,
        })
    }

    /// 更新 alt/原始文件名；仅 active 行可更新。
    async fn update(&self, asset_id: i64, update: MediaUpdate) -> Result<MediaAsset, MediaError> {
        let query = format!(
            "UPDATE media_assets SET alt = $2, original_name = $3, updated_at = now() \
             WHERE id = $1 AND status = 'active' RETURNING {ASSET_COLUMNS}"
        );
        let row = logged_query_as::<MediaAssetRow>(&query)
            .bind(asset_id)
            .bind(update.alt)
            .bind(update.original_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(MediaError::NotFound)?;
        row.try_into()
    }

    /// 软删除（status + deleted_at）；仅 active 行可删。
    async fn soft_delete(&self, asset_id: i64) -> Result<MediaAsset, MediaError> {
        let query = format!(
            "UPDATE media_assets SET status = 'deleted', deleted_at = now(), updated_at = now() \
             WHERE id = $1 AND status = 'active' RETURNING {ASSET_COLUMNS}"
        );
        let row = logged_query_as::<MediaAssetRow>(&query)
            .bind(asset_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(MediaError::NotFound)?;
        row.try_into()
    }

    /// 批量软删除：单事务内完成引用探测与删除（同一快照，避免探测后并发写入引用导致误删）；被引用/不存在项归类返回。
    async fn batch_delete(&self, asset_ids: &[i64]) -> Result<MediaBatchDeleteResult, MediaError> {
        let mut result = MediaBatchDeleteResult::default();
        if asset_ids.is_empty() {
            return Ok(result);
        }
        // 入参去重：同一 ID 重复出现时结果列表里只归类一次。
        let mut seen = HashSet::new();
        let distinct: Vec<i64> = asset_ids
            .iter()
            .copied()
            .filter(|id| seen.insert(*id))
            .collect();
        // 单事务：引用探测与删除共用同一快照，避免探测后并发写入引用导致误删。
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        let existing: HashSet<i64> = logged_query_scalar::<i64>(
            "SELECT id FROM media_assets WHERE id = ANY($1) AND status = 'active'",
        )
        .bind(&distinct)
        .fetch_all(&mut *transaction)
        .await
        .map_err(map_sqlx)?
        .into_iter()
        .collect();
        let referenced: HashSet<i64> = logged_query_scalar::<i64>(
            "SELECT asset_id FROM media_usages WHERE asset_id = ANY($1)",
        )
        .bind(&distinct)
        .fetch_all(&mut *transaction)
        .await
        .map_err(map_sqlx)?
        .into_iter()
        .collect();
        for asset_id in &distinct {
            if !existing.contains(asset_id) {
                result.not_found.push(*asset_id);
            } else if referenced.contains(asset_id) {
                // 被引用项保持 active：物理清理任务只处理零引用的已删除资产，
                // 这里跳过即可保证内容引用不断链。
                result.referenced.push(*asset_id);
            } else {
                result.deleted.push(*asset_id);
            }
        }
        if !result.deleted.is_empty() {
            let updated = logged_query(
                "UPDATE media_assets SET status = 'deleted', deleted_at = now(), updated_at = now() \
                 WHERE id = ANY($1) AND status = 'active'",
            )
            .bind(&result.deleted)
            .execute(&mut *transaction)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
            // 探测已限定 active 行，影响行数不等说明探测后被并发修改；
            // 宁可整体回滚也不返回虚假的成功列表。
            if updated != result.deleted.len() as u64 {
                transaction.rollback().await.map_err(map_sqlx)?;
                return Err(MediaError::Conflict);
            }
        }
        result.deleted.sort_unstable();
        result.referenced.sort_unstable();
        result.not_found.sort_unstable();
        transaction.commit().await.map_err(map_sqlx)?;
        Ok(result)
    }

    /// 按 SHA-256 查 active 资产（重复上传检测；同 Hash 多条时取最早一条）。
    async fn find_by_hash(&self, sha256: &str) -> Result<Option<MediaAsset>, MediaError> {
        // 同 Hash 可能有多条，任取最早一条作为重复提示来源即可。
        let query = format!(
            "SELECT {ASSET_COLUMNS} FROM media_assets \
             WHERE sha256 = $1 AND status = 'active' ORDER BY id LIMIT 1"
        );
        let row = logged_query_as::<MediaAssetRow>(&query)
            .bind(sha256)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    /// 按 object_key 批量查 active 资产（导入冲突检测）。
    async fn find_by_object_keys(&self, keys: &[String]) -> Result<Vec<MediaAsset>, MediaError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let query = format!(
            "SELECT {ASSET_COLUMNS} FROM media_assets \
             WHERE object_key = ANY($1) AND status = 'active'"
        );
        let rows = logged_query_as::<MediaAssetRow>(&query)
            .bind(keys)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// 资产的全部引用列表。
    async fn usages_of(&self, asset_id: i64) -> Result<Vec<MediaUsage>, MediaError> {
        let rows = logged_query_as::<MediaUsageRow>(
            "SELECT id, asset_id, target_type, target_id, created_at FROM media_usages \
             WHERE asset_id = $1 ORDER BY target_type, target_id",
        )
        .bind(asset_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// 全量重建文章的媒体引用（先清 Cover/正文引用再按最终内容写入；同事务）。
    async fn replace_article_usages(
        &self,
        article_id: i64,
        cover_asset_id: Option<i64>,
        content_asset_ids: &[i64],
    ) -> Result<(), MediaError> {
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        // 全量重建：先清除该文章的 Cover 与正文引用，再按最终内容写入。
        logged_query(
            "DELETE FROM media_usages WHERE target_id = $1 \
             AND target_type IN ('article_cover', 'article_content')",
        )
        .bind(article_id)
        .execute(&mut *transaction)
        .await
        .map_err(map_sqlx)?;
        if let Some(asset_id) = cover_asset_id {
            logged_query(
                "INSERT INTO media_usages (asset_id, target_type, target_id) \
                 VALUES ($1, 'article_cover', $2) ON CONFLICT DO NOTHING",
            )
            .bind(asset_id)
            .bind(article_id)
            .execute(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        }
        let mut content_ids = content_asset_ids.to_vec();
        content_ids.sort_unstable();
        content_ids.dedup();
        if !content_ids.is_empty() {
            logged_query(
                "INSERT INTO media_usages (asset_id, target_type, target_id) \
                 SELECT asset_id, 'article_content', $2 FROM unnest($1::bigint[]) AS u(asset_id) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(&content_ids)
            .bind(article_id)
            .execute(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        }
        transaction.commit().await.map_err(map_sqlx)
    }

    /// 引用计数。
    async fn count_usages(&self, asset_id: i64) -> Result<i64, MediaError> {
        logged_query_scalar::<i64>("SELECT count(*) FROM media_usages WHERE asset_id = $1")
            .bind(asset_id)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)
    }

    /// 待补尺寸的图片来源（metadata_probe Worker 任务队列）。
    async fn list_missing_dimensions(&self, limit: i64) -> Result<Vec<MediaAsset>, MediaError> {
        let query = format!(
            "SELECT {ASSET_COLUMNS} FROM media_assets \
             WHERE status = 'active' AND mime LIKE 'image/%' \
             AND (width IS NULL OR height IS NULL) ORDER BY id LIMIT $1"
        );
        let rows = logged_query_as::<MediaAssetRow>(&query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// 回填宽高。
    async fn update_dimensions(
        &self,
        asset_id: i64,
        width: i32,
        height: i32,
    ) -> Result<(), MediaError> {
        logged_query(
            "UPDATE media_assets SET width = $2, height = $3, updated_at = now() WHERE id = $1",
        )
        .bind(asset_id)
        .bind(width)
        .bind(height)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    /// 零引用的已删除资产（物理清理候选；删除动作在 purge 内再次复核引用数）。
    async fn list_purgeable(&self, limit: i64) -> Result<Vec<MediaAsset>, MediaError> {
        // 零引用判定与删除分离在两个步骤，Worker 逐个 purge 时会再次校验引用数。
        let query = format!(
            "SELECT {ASSET_COLUMNS} FROM media_assets WHERE status = 'deleted' \
             AND NOT EXISTS (SELECT 1 FROM media_usages WHERE asset_id = media_assets.id) \
             ORDER BY id LIMIT $1"
        );
        let rows = logged_query_as::<MediaAssetRow>(&query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// 物理删除：事务内先复核引用数（防与并发引用写入竞争），仅 status = 'deleted' 可删。
    async fn purge(&self, asset_id: i64) -> Result<(), MediaError> {
        // 物理删除前在事务内复核引用数，防止与并发的引用写入竞争。
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        let references =
            logged_query_scalar::<i64>("SELECT count(*) FROM media_usages WHERE asset_id = $1")
                .bind(asset_id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(map_sqlx)?;
        if references > 0 {
            return Err(MediaError::Referenced { count: references });
        }
        let result = logged_query("DELETE FROM media_assets WHERE id = $1 AND status = 'deleted'")
            .bind(asset_id)
            .execute(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        transaction.commit().await.map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(MediaError::NotFound);
        }
        Ok(())
    }
}

/// SQLx 错误到 MediaError 的映射（存储故障记日志后归为 StoreUnavailable）。
fn map_sqlx(error: sqlx::Error) -> MediaError {
    if error
        .as_database_error()
        .is_some_and(|database_error| database_error.is_unique_violation())
    {
        MediaError::Conflict
    } else {
        tracing::error!(error = %error, "media repository operation failed");
        MediaError::StoreUnavailable
    }
}
