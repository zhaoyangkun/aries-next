//! 导航菜单域：最多两级，目标类型 article/page/category/url，物理删除。
//! 两级限制与父节点存在性由 Repository/Server 保证；core 提供目标字段互斥校验。
//! 排序为一次原子批量操作（事务内更新 sort_order），不逐条移动。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

/// 导航目标类型：内部目标存 target_id，外部链接存 url。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NavigationTargetType {
    Article,
    Page,
    Category,
    Url,
}

impl NavigationTargetType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Article => "article",
            Self::Page => "page",
            Self::Category => "category",
            Self::Url => "url",
        }
    }
}

impl fmt::Display for NavigationTargetType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for NavigationTargetType {
    type Err = NavigationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "article" => Ok(Self::Article),
            "page" => Ok(Self::Page),
            "category" => Ok(Self::Category),
            "url" => Ok(Self::Url),
            _ => Err(NavigationError::InvalidTargetType),
        }
    }
}

/// 导航菜单项实体。
#[derive(Debug, Clone)]
pub struct NavigationItem {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub label: String,
    pub target_type: NavigationTargetType,
    pub target_id: Option<i64>,
    pub url: Option<String>,
    pub open_in_new_tab: bool,
    pub visible: bool,
    pub sort_order: i32,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 新建导航项入参。
#[derive(Debug, Clone)]
pub struct NewNavigationItem {
    pub parent_id: Option<i64>,
    pub label: String,
    pub target_type: NavigationTargetType,
    pub target_id: Option<i64>,
    pub url: Option<String>,
    pub open_in_new_tab: bool,
    pub visible: bool,
    pub sort_order: i32,
}

/// 导航项更新入参（全量字段）。
#[derive(Debug, Clone)]
pub struct NavigationItemUpdate {
    pub parent_id: Option<i64>,
    pub label: String,
    pub target_type: NavigationTargetType,
    pub target_id: Option<i64>,
    pub url: Option<String>,
    pub open_in_new_tab: bool,
    pub visible: bool,
    pub sort_order: i32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NavigationError {
    #[error("invalid navigation target type")]
    InvalidTargetType,
    /// url 目标必须提供 url，其余目标必须提供 target_id（与数据库 CHECK 一致）。
    #[error("navigation target fields do not match target type")]
    InvalidTarget,
    #[error("navigation url must use http or https scheme")]
    InvalidUrl,
    #[error("navigation label must contain 1 to 60 characters")]
    InvalidLabel,
    /// 父节点不存在或目标已超过两级。
    #[error("navigation hierarchy allows at most two levels")]
    InvalidHierarchy,
    #[error("navigation item not found")]
    NotFound,
    /// 存在子节点或排序冲突。
    #[error("navigation item conflict")]
    Conflict,
    #[error("navigation store unavailable")]
    StoreUnavailable,
}

/// 导航目标字段校验：target_type = url 时 url 必填且限 http/https，
/// 其余类型 target_id 必填；与数据库 CHECK 保持一致，提前给出可读错误。
pub fn validate_target(
    target_type: NavigationTargetType,
    target_id: Option<i64>,
    url: Option<&str>,
) -> Result<(), NavigationError> {
    match target_type {
        NavigationTargetType::Url => {
            let url = url.ok_or(NavigationError::InvalidTarget)?;
            let lower = url.to_ascii_lowercase();
            if url.chars().count() > 2048
                || !(lower.starts_with("https://") || lower.starts_with("http://"))
            {
                return Err(NavigationError::InvalidUrl);
            }
            Ok(())
        }
        _ => {
            if target_id.is_none() {
                return Err(NavigationError::InvalidTarget);
            }
            Ok(())
        }
    }
}

/// 导航标签校验。
pub fn validate_label(value: &str) -> Result<(), NavigationError> {
    let length = value.trim().chars().count();
    if !(1..=60).contains(&length) {
        return Err(NavigationError::InvalidLabel);
    }
    Ok(())
}

/// 导航 Repository 接口，由 infra 层实现。
#[async_trait]
pub trait NavigationRepository: Send + Sync {
    /// 创建导航项；父节点不存在或超过两级返回 `InvalidHierarchy`。
    async fn create(&self, item: NewNavigationItem) -> Result<NavigationItem, NavigationError>;

    /// 按 ID 查找。
    async fn find(&self, item_id: i64) -> Result<Option<NavigationItem>, NavigationError>;

    /// 全量列表（Admin 端，按 parent_id, sort_order, id 排序）。
    async fn list(&self) -> Result<Vec<NavigationItem>, NavigationError>;

    /// 可见项列表（Public 端，仅 visible = true）。
    async fn list_visible(&self) -> Result<Vec<NavigationItem>, NavigationError>;

    /// 更新导航项；层级校验同 create。
    async fn update(
        &self,
        item_id: i64,
        update: NavigationItemUpdate,
    ) -> Result<NavigationItem, NavigationError>;

    /// 物理删除导航项；仍有子节点时返回 `Conflict`。
    async fn delete(&self, item_id: i64) -> Result<(), NavigationError>;

    /// 原子批量排序：事务内按入参顺序重写 sort_order。
    async fn reorder(&self, ordered_item_ids: Vec<i64>) -> Result<(), NavigationError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_target_requires_safe_url() {
        assert!(
            validate_target(NavigationTargetType::Url, None, Some("https://example.com")).is_ok()
        );
        assert_eq!(
            validate_target(NavigationTargetType::Url, None, None),
            Err(NavigationError::InvalidTarget)
        );
        assert_eq!(
            validate_target(NavigationTargetType::Url, None, Some("javascript:alert(1)")),
            Err(NavigationError::InvalidUrl)
        );
    }

    #[test]
    fn internal_targets_require_target_id() {
        assert!(validate_target(NavigationTargetType::Article, Some(1), None).is_ok());
        assert_eq!(
            validate_target(NavigationTargetType::Page, None, None),
            Err(NavigationError::InvalidTarget)
        );
        assert_eq!(
            validate_target(NavigationTargetType::Category, None, Some("https://x.com")),
            Err(NavigationError::InvalidTarget)
        );
    }

    #[test]
    fn target_type_parses_only_known_values() {
        assert_eq!("url".parse(), Ok(NavigationTargetType::Url));
        assert_eq!(
            "external".parse::<NavigationTargetType>(),
            Err(NavigationError::InvalidTargetType)
        );
    }
}
