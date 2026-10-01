//! 友情链接域：分类可空，URL scheme 白名单（仅 http/https），active/inactive 状态与软删除。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

/// 友链状态：inactive 不对外展示但保留数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    Active,
    Inactive,
}

impl LinkStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
        }
    }
}

impl fmt::Display for LinkStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for LinkStatus {
    type Err = LinkError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "inactive" => Ok(Self::Inactive),
            _ => Err(LinkError::InvalidStatus),
        }
    }
}

/// 友链实体。
#[derive(Debug, Clone)]
pub struct Link {
    pub id: i64,
    pub category_id: Option<i64>,
    pub title: String,
    pub url: String,
    pub icon_url: Option<String>,
    pub description: String,
    pub status: LinkStatus,
    pub sort_order: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 新建友链入参。
#[derive(Debug, Clone)]
pub struct NewLink {
    pub category_id: Option<i64>,
    pub title: String,
    pub url: String,
    pub icon_url: Option<String>,
    pub description: String,
    pub status: LinkStatus,
    pub sort_order: i32,
}

/// 友链更新入参（全量字段）。
#[derive(Debug, Clone)]
pub struct LinkUpdate {
    pub category_id: Option<i64>,
    pub title: String,
    pub url: String,
    pub icon_url: Option<String>,
    pub description: String,
    pub status: LinkStatus,
    pub sort_order: i32,
}

/// 友链列表查询参数。
#[derive(Debug, Clone, Default)]
pub struct LinkListQuery {
    pub page: u32,
    pub page_size: u32,
    pub category_id: Option<i64>,
    pub status: Option<LinkStatus>,
    pub keyword: Option<String>,
}

/// 分页友链列表。
#[derive(Debug, Clone)]
pub struct LinkPage {
    pub items: Vec<Link>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LinkError {
    #[error("invalid link status")]
    InvalidStatus,
    #[error("link url must use http or https scheme")]
    InvalidUrl,
    #[error("link title must contain 1 to 100 characters")]
    InvalidTitle,
    #[error("link not found")]
    NotFound,
    #[error("link conflict")]
    Conflict,
    #[error("link store unavailable")]
    StoreUnavailable,
}

/// 友链 URL 校验：只允许 http/https，防止 javascript: 等危险 scheme 注入。
pub fn validate_url(value: &str) -> Result<(), LinkError> {
    let length = value.chars().count();
    if !(1..=2048).contains(&length) {
        return Err(LinkError::InvalidUrl);
    }
    let lower = value.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(LinkError::InvalidUrl);
    }
    Ok(())
}

/// 友链标题校验。
pub fn validate_title(value: &str) -> Result<(), LinkError> {
    let length = value.trim().chars().count();
    if !(1..=100).contains(&length) {
        return Err(LinkError::InvalidTitle);
    }
    Ok(())
}

/// 友链 Repository 接口，由 infra 层实现。
#[async_trait]
pub trait LinkRepository: Send + Sync {
    /// 创建友链。
    async fn create(&self, link: NewLink) -> Result<Link, LinkError>;

    /// 按 ID 查找（Admin 端，排除已软删除）。
    async fn find(&self, link_id: i64) -> Result<Option<Link>, LinkError>;

    /// 分页列表查询（Admin 端）。
    async fn list(&self, query: LinkListQuery) -> Result<LinkPage, LinkError>;

    /// 公开友链列表（Public 端，仅 active，按 sort_order, id 排序）。
    async fn list_public(&self) -> Result<Vec<Link>, LinkError>;

    /// 更新友链。
    async fn update(&self, link_id: i64, update: LinkUpdate) -> Result<Link, LinkError>;

    /// 软删除友链。
    async fn delete(&self, link_id: i64) -> Result<(), LinkError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_url_only_allows_http_schemes() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/path?q=1").is_ok());
        assert!(validate_url("HTTPS://EXAMPLE.COM").is_ok());
        assert_eq!(
            validate_url("javascript:alert(1)"),
            Err(LinkError::InvalidUrl)
        );
        assert_eq!(
            validate_url("ftp://example.com"),
            Err(LinkError::InvalidUrl)
        );
        assert_eq!(validate_url("//example.com"), Err(LinkError::InvalidUrl));
        assert_eq!(validate_url(""), Err(LinkError::InvalidUrl));
    }

    #[test]
    fn link_status_parses_only_known_values() {
        assert_eq!("active".parse(), Ok(LinkStatus::Active));
        assert_eq!("inactive".parse(), Ok(LinkStatus::Inactive));
        assert_eq!(
            "enabled".parse::<LinkStatus>(),
            Err(LinkError::InvalidStatus)
        );
    }
}
