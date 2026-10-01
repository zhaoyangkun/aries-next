//! 图库域：Gallery 与 Gallery Item，基于 Media Asset，draft/published 状态与软删除。
//! 条目排序由一次原子批量操作完成（事务内更新 sort_order）。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

/// 图库状态：草稿不对外可见。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GalleryStatus {
    Draft,
    Published,
}

impl GalleryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }
}

impl fmt::Display for GalleryStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for GalleryStatus {
    type Err = GalleryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "published" => Ok(Self::Published),
            _ => Err(GalleryError::InvalidStatus),
        }
    }
}

/// 图库实体。
#[derive(Debug, Clone)]
pub struct Gallery {
    pub id: i64,
    pub category_id: i64,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub cover_media_id: Option<i64>,
    pub status: GalleryStatus,
    pub sort_order: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 新建图库入参。
#[derive(Debug, Clone)]
pub struct NewGallery {
    pub category_id: i64,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub cover_media_id: Option<i64>,
    pub status: GalleryStatus,
    pub sort_order: i32,
}

/// 图库更新入参（全量字段）。
#[derive(Debug, Clone)]
pub struct GalleryUpdate {
    pub category_id: i64,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub cover_media_id: Option<i64>,
    pub status: GalleryStatus,
    pub sort_order: i32,
}

/// 图库列表查询参数。
#[derive(Debug, Clone, Default)]
pub struct GalleryListQuery {
    pub page: u32,
    pub page_size: u32,
    pub category_id: Option<i64>,
    pub status: Option<GalleryStatus>,
    pub keyword: Option<String>,
}

/// 分页图库列表。
#[derive(Debug, Clone)]
pub struct GalleryPage {
    pub items: Vec<Gallery>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// 图库条目实体。
#[derive(Debug, Clone)]
pub struct GalleryItem {
    pub id: i64,
    pub gallery_id: i64,
    pub media_asset_id: i64,
    pub alt: String,
    pub location: String,
    pub sort_order: i32,
    pub created_at: OffsetDateTime,
}

/// 媒体资产摘要：条目接口内联返回，消除前端逐条拉取媒体详情的 N+1。
/// 媒体被软删除时为 None（LEFT JOIN + deleted_at 过滤）。
#[derive(Debug, Clone)]
pub struct GalleryItemMedia {
    pub id: i64,
    pub url: String,
    pub alt: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// 带媒体摘要的图库条目：list/add/update 的返回模型。
#[derive(Debug, Clone)]
pub struct GalleryItemDetail {
    pub item: GalleryItem,
    pub media: Option<GalleryItemMedia>,
}

/// 新增图库条目入参。
#[derive(Debug, Clone)]
pub struct NewGalleryItem {
    pub gallery_id: i64,
    pub media_asset_id: i64,
    pub alt: String,
    pub location: String,
    pub sort_order: i32,
}

/// 公开照片墙条目：跨相册平铺所有 published 相册的照片（对齐旧版 xue 主题
/// 照片墙的 GetAll 语义）；已解析媒体 URL/尺寸与分类名，前端无需二次查询。
#[derive(Debug, Clone)]
pub struct PublicPhoto {
    pub url: String,
    pub alt: String,
    pub location: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub gallery_slug: String,
    pub gallery_title: String,
    pub category_name: Option<String>,
}

/// 图库条目更新入参（批量编辑 alt/location）。
#[derive(Debug, Clone)]
pub struct GalleryItemUpdate {
    pub alt: String,
    pub location: String,
    pub sort_order: i32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GalleryError {
    #[error("invalid gallery status")]
    InvalidStatus,
    #[error("gallery slug must be 1-160 lowercase letters, digits or hyphens")]
    InvalidSlug,
    #[error("gallery title must contain 1 to 200 characters")]
    InvalidTitle,
    #[error("gallery not found")]
    NotFound,
    #[error("gallery item not found")]
    ItemNotFound,
    /// slug 重复或同一媒体重复加入图库。
    #[error("gallery conflict: duplicate slug or media asset")]
    Conflict,
    #[error("gallery store unavailable")]
    StoreUnavailable,
}

/// 图库 slug 与页面同一规则：小写字母、数字、连字符。
pub fn validate_slug(value: &str) -> Result<(), GalleryError> {
    let length = value.chars().count();
    let valid = (1..=160).contains(&length)
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(GalleryError::InvalidSlug);
    }
    Ok(())
}

/// 图库标题校验。
pub fn validate_title(value: &str) -> Result<(), GalleryError> {
    let length = value.trim().chars().count();
    if !(1..=200).contains(&length) {
        return Err(GalleryError::InvalidTitle);
    }
    Ok(())
}

/// 图库 Repository 接口，由 infra 层实现。
#[async_trait]
pub trait GalleryRepository: Send + Sync {
    /// 创建图库；slug 冲突返回 `Conflict`。
    async fn create_gallery(&self, gallery: NewGallery) -> Result<Gallery, GalleryError>;

    /// 按 ID 查找（Admin 端，排除已软删除）。
    async fn find_gallery(&self, gallery_id: i64) -> Result<Option<Gallery>, GalleryError>;

    /// 按 slug 查找已发布图库（Public 端）。
    async fn find_published_by_slug(&self, slug: &str) -> Result<Option<Gallery>, GalleryError>;

    /// 分页列表查询（Admin 端）。
    async fn list_galleries(&self, query: GalleryListQuery) -> Result<GalleryPage, GalleryError>;

    /// 公开图库分页列表（Public 端，仅 published）。
    async fn list_public(&self, page: u32, page_size: u32) -> Result<GalleryPage, GalleryError>;

    /// 公开照片墙：跨相册平铺全部 published 相册的条目，不分页（个人博客量级）。
    /// 排序固定为 `galleries.sort_order, galleries.id, gallery_items.sort_order`。
    async fn list_public_photos(&self) -> Result<Vec<PublicPhoto>, GalleryError>;

    /// 更新图库；slug 冲突返回 `Conflict`。
    async fn update_gallery(
        &self,
        gallery_id: i64,
        update: GalleryUpdate,
    ) -> Result<Gallery, GalleryError>;

    /// 软删除图库（条目随 FK 级联规则处理）。
    async fn delete_gallery(&self, gallery_id: i64) -> Result<(), GalleryError>;

    /// 列出图库全部条目（按 sort_order, id 排序），内联媒体摘要。
    async fn list_items(&self, gallery_id: i64) -> Result<Vec<GalleryItemDetail>, GalleryError>;

    /// 添加条目；同一媒体重复加入返回 `Conflict`。
    async fn add_item(&self, item: NewGalleryItem) -> Result<GalleryItemDetail, GalleryError>;

    /// 更新条目 alt/location/排序。
    async fn update_item(
        &self,
        item_id: i64,
        update: GalleryItemUpdate,
    ) -> Result<GalleryItemDetail, GalleryError>;

    /// 移除条目（物理删除，条目本身无恢复价值）。
    async fn remove_item(&self, item_id: i64) -> Result<(), GalleryError>;

    /// 原子批量排序：事务内按入参顺序重写 sort_order。
    async fn reorder_items(
        &self,
        gallery_id: i64,
        ordered_item_ids: Vec<i64>,
    ) -> Result<(), GalleryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_slug_format_is_enforced() {
        assert!(validate_slug("travel-2024").is_ok());
        assert_eq!(validate_slug("Travel"), Err(GalleryError::InvalidSlug));
        assert_eq!(validate_slug(""), Err(GalleryError::InvalidSlug));
    }

    #[test]
    fn gallery_status_parses_only_known_values() {
        assert_eq!("draft".parse(), Ok(GalleryStatus::Draft));
        assert_eq!(
            "live".parse::<GalleryStatus>(),
            Err(GalleryError::InvalidStatus)
        );
    }
}
