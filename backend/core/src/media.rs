use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaProvider {
    Local,
    S3,
    LegacyUrl,
}

impl MediaProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::S3 => "s3",
            Self::LegacyUrl => "legacy_url",
        }
    }
}

impl fmt::Display for MediaProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MediaProvider {
    type Err = MediaError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "local" => Ok(Self::Local),
            "s3" => Ok(Self::S3),
            "legacy_url" => Ok(Self::LegacyUrl),
            _ => Err(MediaError::InvalidProvider),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaStatus {
    Active,
    Deleted,
}

impl MediaStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deleted => "deleted",
        }
    }
}

impl FromStr for MediaStatus {
    type Err = MediaError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "deleted" => Ok(Self::Deleted),
            _ => Err(MediaError::StoreUnavailable),
        }
    }
}

/// Usage 目标类型固定枚举，数据库 Check Constraint 与之一一对应；
/// 不允许客户端传入任意字符串，避免 Usage 表被污染。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageTargetType {
    ArticleCover,
    ArticleContent,
    GalleryItem,
}

impl UsageTargetType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ArticleCover => "article_cover",
            Self::ArticleContent => "article_content",
            Self::GalleryItem => "gallery_item",
        }
    }
}

impl FromStr for UsageTargetType {
    type Err = MediaError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "article_cover" => Ok(Self::ArticleCover),
            "article_content" => Ok(Self::ArticleContent),
            "gallery_item" => Ok(Self::GalleryItem),
            _ => Err(MediaError::InvalidUsageTarget),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaAsset {
    pub id: i64,
    pub provider: MediaProvider,
    pub object_key: String,
    pub url: String,
    pub original_name: String,
    pub mime: String,
    pub size_bytes: i64,
    /// 图片尺寸探测失败不阻断上传，因此允许为 `None`。
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub sha256: String,
    pub alt: String,
    pub status: MediaStatus,
    pub uploaded_by: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone)]
pub struct NewMediaAsset {
    pub provider: MediaProvider,
    pub object_key: String,
    pub url: String,
    pub original_name: String,
    pub mime: String,
    pub size_bytes: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub sha256: String,
    pub alt: String,
    pub uploaded_by: i64,
}

/// 仅允许更新展示性字段；文件内容、Hash、URL 创建后不可变，
/// 保证已发布内容引用的 URL 语义稳定。
#[derive(Debug, Clone)]
pub struct MediaUpdate {
    pub alt: String,
    pub original_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct MediaListQuery {
    pub page: u32,
    pub page_size: u32,
    pub keyword: Option<String>,
    pub provider: Option<MediaProvider>,
    pub mime: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MediaPage {
    pub items: Vec<MediaAsset>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone)]
pub struct MediaUsage {
    pub id: i64,
    pub asset_id: i64,
    pub target_type: UsageTargetType,
    pub target_id: i64,
    pub created_at: OffsetDateTime,
}

/// 站点设置为单行配置，不存在列表语义，因此不需要独立 ID 之外的结构。
/// 站点评论策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentPolicy {
    Closed,
    Moderated,
    AutoApprove,
}

impl CommentPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Moderated => "moderated",
            Self::AutoApprove => "auto_approve",
        }
    }
}

impl fmt::Display for CommentPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CommentPolicy {
    type Err = MediaError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "closed" => Ok(Self::Closed),
            "moderated" => Ok(Self::Moderated),
            "auto_approve" => Ok(Self::AutoApprove),
            _ => Err(MediaError::InvalidCommentPolicy),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SiteSettings {
    pub site_name: String,
    pub site_description: String,
    pub site_url: String,
    pub logo_url: String,
    pub icp_text: String,
    pub default_cover_url: String,
    pub page_size_index: i32,
    pub page_size_archive: i32,
    pub page_size_search: i32,
    pub comment_policy: CommentPolicy,
    pub comments_per_page: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct SiteSettingsUpdate {
    pub site_name: String,
    pub site_description: String,
    pub site_url: String,
    pub logo_url: String,
    pub icp_text: String,
    pub default_cover_url: String,
    pub page_size_index: i32,
    pub page_size_archive: i32,
    pub page_size_search: i32,
    pub comment_policy: CommentPolicy,
    pub comments_per_page: i32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MediaError {
    #[error("invalid media provider")]
    InvalidProvider,
    #[error("invalid media usage target type")]
    InvalidUsageTarget,
    #[error("invalid comment policy")]
    InvalidCommentPolicy,
    #[error("media asset not found")]
    NotFound,
    #[error("resource conflict")]
    Conflict,
    /// 删除被引用的 Asset 时携带引用数，便于前端提示具体影响范围。
    #[error("media asset is still referenced")]
    Referenced { count: i64 },
    #[error("media store unavailable")]
    StoreUnavailable,
}

/// 批量删除的逐项结果：部分成功语义，被引用的资产单独归类，
/// 不让一个被引用的文件挡住同批其他废弃文件的清理。
#[derive(Debug, Clone, Default)]
pub struct MediaBatchDeleteResult {
    /// 成功软删除的 Asset ID。
    pub deleted: Vec<i64>,
    /// 仍被内容引用、保持 `active` 的 Asset ID。
    pub referenced: Vec<i64>,
    /// 不存在或已删除（列表之外）的 Asset ID。
    pub not_found: Vec<i64>,
}

#[async_trait]
pub trait MediaRepository: Send + Sync {
    async fn create(&self, asset: NewMediaAsset) -> Result<MediaAsset, MediaError>;
    async fn find(&self, asset_id: i64) -> Result<Option<MediaAsset>, MediaError>;
    /// Admin 列表只返回 `active` 资产；已删除资产由清理任务处理，不出现在列表中。
    async fn list(&self, query: MediaListQuery) -> Result<MediaPage, MediaError>;
    async fn update(&self, asset_id: i64, update: MediaUpdate) -> Result<MediaAsset, MediaError>;
    /// 软删除保留行与文件，物理删除交给后台清理任务，避免误删立即不可恢复。
    async fn soft_delete(&self, asset_id: i64) -> Result<MediaAsset, MediaError>;
    /// 批量软删除，部分成功语义：单事务内先探测引用再 UPDATE，
    /// 被引用项保持 `active` 并归入 `referenced`，其余成功删除。
    async fn batch_delete(&self, asset_ids: &[i64]) -> Result<MediaBatchDeleteResult, MediaError>;
    /// 按内容 Hash 查找活跃资产，用于上传时的重复提示。
    async fn find_by_hash(&self, sha256: &str) -> Result<Option<MediaAsset>, MediaError>;
    /// 按 Object Key 批量反查活跃资产，供文章引用解析使用。
    async fn find_by_object_keys(&self, keys: &[String]) -> Result<Vec<MediaAsset>, MediaError>;
    async fn usages_of(&self, asset_id: i64) -> Result<Vec<MediaUsage>, MediaError>;
    /// 全量重建某文章的 Cover 与正文引用：先删后插，保证与最终内容一致。
    async fn replace_article_usages(
        &self,
        article_id: i64,
        cover_asset_id: Option<i64>,
        content_asset_ids: &[i64],
    ) -> Result<(), MediaError>;
    async fn count_usages(&self, asset_id: i64) -> Result<i64, MediaError>;
    /// 供 metadata_probe 任务使用：找出缺少尺寸信息的活跃图片。
    async fn list_missing_dimensions(&self, limit: i64) -> Result<Vec<MediaAsset>, MediaError>;
    async fn update_dimensions(
        &self,
        asset_id: i64,
        width: i32,
        height: i32,
    ) -> Result<(), MediaError>;
    /// 供 media_cleanup 任务使用：已软删除且零引用的资产才允许物理清除。
    async fn list_purgeable(&self, limit: i64) -> Result<Vec<MediaAsset>, MediaError>;
    async fn purge(&self, asset_id: i64) -> Result<(), MediaError>;
}

/// 存储后端只关心字节读写与公开 URL 映射，不感知数据库。
#[async_trait]
pub trait MediaStorage: Send + Sync {
    /// 写入成功返回公开 URL；`object_key` 由调用方生成并保证唯一。
    async fn put(&self, object_key: &str, bytes: &[u8], mime: &str) -> Result<String, MediaError>;
    async fn get(&self, object_key: &str) -> Result<Option<Vec<u8>>, MediaError>;
    /// 删除不存在的 Key 视为成功，清理任务因此可以安全重试。
    async fn delete(&self, object_key: &str) -> Result<(), MediaError>;
    fn url_for(&self, object_key: &str) -> String;
    /// 本地存储由本站静态路由直接发文件；S3 等远端存储需要 Redirect 到公开 URL。
    fn serves_files_locally(&self) -> bool;
}

#[async_trait]
pub trait SiteSettingsRepository: Send + Sync {
    async fn get(&self) -> Result<SiteSettings, MediaError>;
    async fn update(&self, update: SiteSettingsUpdate) -> Result<SiteSettings, MediaError>;
}

/// 从 Markdown 正文与 Cover URL 中提取本站媒体引用的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaUsageRefs {
    pub cover: Option<String>,
    pub content: Vec<String>,
}

/// Object Key 允许的字符集合：Key 由服务端生成，只含路径安全的字符。
fn is_key_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '/' | '~')
}

/// 从 `base_url` 之后的 URL 片段截取 Object Key，丢弃 Query 与 Fragment。
fn extract_key_after_prefix(url: &str, base_url: &str) -> Option<String> {
    let prefix = format!("{base_url}/");
    let start = url.find(&prefix)? + prefix.len();
    let key: String = url[start..]
        .chars()
        .take_while(|character| is_key_character(*character))
        .collect();
    if key.is_empty() || key.contains("..") {
        return None;
    }
    Some(key)
}

/// 扫描 Markdown 正文与 Cover URL，找出所有指向本站媒体（`base_url` 前缀）的 Object Key。
/// 纯函数设计便于单元测试；匹配基于前缀而非完整 Markdown AST，
/// 因此 HTML `img` 标签与 Markdown 图片语法都能覆盖。
pub fn extract_media_usage_refs(
    markdown: &str,
    cover_url: Option<&str>,
    base_url: &str,
) -> MediaUsageRefs {
    let base_url = base_url.trim_end_matches('/');
    if base_url.is_empty() {
        return MediaUsageRefs::default();
    }

    let mut content = Vec::new();
    let prefix = format!("{base_url}/");
    let mut rest = markdown;
    while let Some(index) = rest.find(&prefix) {
        let candidate = &rest[index..];
        if let Some(key) = extract_key_after_prefix(candidate, base_url) {
            if !content.contains(&key) {
                content.push(key);
            }
        }
        rest = &candidate[prefix.len()..];
    }

    let cover = cover_url.and_then(|url| extract_key_after_prefix(url, base_url));
    MediaUsageRefs { cover, content }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_parser_extracts_content_and_cover_keys() {
        let markdown = "![a](/api/media/files/2026/08/a.png)\n\
            <img src=\"/api/media/files/2026/08/b.jpg?x=1\"> \
            [外链](https://example.com/x.png) \
            ![dup](/api/media/files/2026/08/a.png)";
        let refs = extract_media_usage_refs(
            markdown,
            Some("/api/media/files/2026/08/cover.webp"),
            "/api/media/files",
        );
        assert_eq!(refs.cover.as_deref(), Some("2026/08/cover.webp"));
        assert_eq!(
            refs.content,
            vec!["2026/08/a.png".to_owned(), "2026/08/b.jpg".to_owned()]
        );
    }

    #[test]
    fn usage_parser_ignores_foreign_and_traversal_urls() {
        let markdown = "![a](/api/media/files/../secret.png) ![b](https://cdn.example.com/x.png)";
        let refs = extract_media_usage_refs(markdown, None, "/api/media/files");
        assert!(refs.cover.is_none());
        assert!(refs.content.is_empty());
    }
}
