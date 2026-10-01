//! 自定义页面域：Markdown 页面，slug 定位，draft/published 状态与软删除。
//! 渲染由 HTTP 层完成（`content_html` 传入），Repository 不做渲染。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

/// 页面状态：草稿不对外可见，发布后可通过 Public API 按 slug 访问。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageStatus {
    Draft,
    Published,
}

impl PageStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }
}

impl fmt::Display for PageStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PageStatus {
    type Err = PageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "published" => Ok(Self::Published),
            _ => Err(PageError::InvalidStatus),
        }
    }
}

/// 页面实体。
#[derive(Debug, Clone)]
pub struct Page {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub content_markdown: String,
    pub content_html: String,
    pub status: PageStatus,
    pub sort_order: i32,
    pub created_by: Option<i64>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 新建页面入参；`content_html` 由 HTTP 层渲染后传入。
#[derive(Debug, Clone)]
pub struct NewPage {
    pub slug: String,
    pub title: String,
    pub content_markdown: String,
    pub content_html: String,
    pub status: PageStatus,
    pub sort_order: i32,
    pub created_by: i64,
}

/// 页面更新入参（全量字段）。
#[derive(Debug, Clone)]
pub struct PageUpdate {
    pub slug: String,
    pub title: String,
    pub content_markdown: String,
    pub content_html: String,
    pub status: PageStatus,
    pub sort_order: i32,
}

/// 页面列表查询参数。
#[derive(Debug, Clone, Default)]
pub struct PageListQuery {
    pub page: u32,
    pub page_size: u32,
    pub status: Option<PageStatus>,
    pub keyword: Option<String>,
}

/// 分页页面列表。
#[derive(Debug, Clone)]
pub struct PagePage {
    pub items: Vec<Page>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PageError {
    #[error("invalid page status")]
    InvalidStatus,
    #[error("page slug must be 1-160 lowercase letters, digits or hyphens")]
    InvalidSlug,
    #[error("page title must contain 1 to 200 characters")]
    InvalidTitle,
    #[error("page not found")]
    NotFound,
    /// 唯一约束冲突（slug 重复）。
    #[error("page slug already exists")]
    Conflict,
    #[error("page store unavailable")]
    StoreUnavailable,
}

/// 页面 slug 只允许小写字母、数字与连字符（出现在 URL 路径中，必须稳定且安全）。
pub fn validate_slug(value: &str) -> Result<(), PageError> {
    let length = value.chars().count();
    if !(1..=160).contains(&length) {
        return Err(PageError::InvalidSlug);
    }
    let valid = value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(PageError::InvalidSlug);
    }
    Ok(())
}

/// 页面标题校验。
pub fn validate_title(value: &str) -> Result<(), PageError> {
    let length = value.trim().chars().count();
    if !(1..=200).contains(&length) {
        return Err(PageError::InvalidTitle);
    }
    Ok(())
}

/// 页面 Repository 接口，由 infra 层实现。
#[async_trait]
pub trait PageRepository: Send + Sync {
    /// 创建页面；slug 冲突返回 `Conflict`。
    async fn create(&self, page: NewPage) -> Result<Page, PageError>;

    /// 按 ID 查找（Admin 端，排除已软删除）。
    async fn find(&self, page_id: i64) -> Result<Option<Page>, PageError>;

    /// 按 slug 查找已发布页面（Public 端）。
    async fn find_published_by_slug(&self, slug: &str) -> Result<Option<Page>, PageError>;

    /// 分页列表查询（Admin 端）。
    async fn list(&self, query: PageListQuery) -> Result<PagePage, PageError>;

    /// 更新页面；slug 冲突返回 `Conflict`，不存在返回 `NotFound`。
    async fn update(&self, page_id: i64, update: PageUpdate) -> Result<Page, PageError>;

    /// 软删除页面。
    async fn delete(&self, page_id: i64) -> Result<(), PageError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_slug_accepts_lowercase_alphanumeric_and_hyphens() {
        assert!(validate_slug("about").is_ok());
        assert!(validate_slug("about-me-2024").is_ok());
        assert!(validate_slug("").is_err());
        assert!(validate_slug("About").is_err());
        assert!(validate_slug("about me").is_err());
        assert!(validate_slug("关于").is_err());
        assert!(validate_slug(&"a".repeat(161)).is_err());
    }

    #[test]
    fn page_status_parses_only_known_values() {
        assert_eq!("draft".parse(), Ok(PageStatus::Draft));
        assert_eq!("published".parse(), Ok(PageStatus::Published));
        assert_eq!(
            "archived".parse::<PageStatus>(),
            Err(PageError::InvalidStatus)
        );
    }

    #[test]
    fn page_title_must_not_be_blank() {
        assert!(validate_title("关于我").is_ok());
        assert_eq!(validate_title("   "), Err(PageError::InvalidTitle));
    }
}
