//! 日志（短内容 Timeline）域：公开/私密可见性，软删除。
//! Private 日志不得出现在任何 Public API、Sitemap 或 RSS 中（Phase 06 §5）。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

/// 日志可见性：private 仅限管理端。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalVisibility {
    Public,
    Private,
}

impl JournalVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

impl fmt::Display for JournalVisibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for JournalVisibility {
    type Err = JournalError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            _ => Err(JournalError::InvalidVisibility),
        }
    }
}

/// 日志实体。
#[derive(Debug, Clone)]
pub struct Journal {
    pub id: i64,
    pub content_markdown: String,
    pub content_html: String,
    pub visibility: JournalVisibility,
    pub created_by: Option<i64>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 新建日志入参；`content_html` 由 HTTP 层渲染后传入。
#[derive(Debug, Clone)]
pub struct NewJournal {
    pub content_markdown: String,
    pub content_html: String,
    pub visibility: JournalVisibility,
    pub created_by: i64,
}

/// 日志更新入参（全量字段）。
#[derive(Debug, Clone)]
pub struct JournalUpdate {
    pub content_markdown: String,
    pub content_html: String,
    pub visibility: JournalVisibility,
}

/// 日志列表查询参数。
#[derive(Debug, Clone, Default)]
pub struct JournalListQuery {
    pub page: u32,
    pub page_size: u32,
    pub visibility: Option<JournalVisibility>,
}

/// 分页日志列表。
#[derive(Debug, Clone)]
pub struct JournalPage {
    pub items: Vec<Journal>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum JournalError {
    #[error("invalid journal visibility")]
    InvalidVisibility,
    #[error("journal content must contain 1 to 2000 characters")]
    ContentLength,
    #[error("journal not found")]
    NotFound,
    #[error("journal conflict")]
    Conflict,
    #[error("journal store unavailable")]
    StoreUnavailable,
}

/// 日志内容长度校验（与数据库 CHECK 一致）。
pub fn validate_content(value: &str) -> Result<(), JournalError> {
    let length = value.trim().chars().count();
    if !(1..=2000).contains(&length) {
        return Err(JournalError::ContentLength);
    }
    Ok(())
}

/// 日志 Repository 接口，由 infra 层实现。
#[async_trait]
pub trait JournalRepository: Send + Sync {
    /// 创建日志。
    async fn create(&self, journal: NewJournal) -> Result<Journal, JournalError>;

    /// 按 ID 查找（Admin 端，排除已软删除）。
    async fn find(&self, journal_id: i64) -> Result<Option<Journal>, JournalError>;

    /// 分页列表查询（Admin 端，按创建时间倒序）。
    async fn list(&self, query: JournalListQuery) -> Result<JournalPage, JournalError>;

    /// 公开日志分页列表（Public 端，仅 visibility = public）。
    async fn list_public(&self, page: u32, page_size: u32) -> Result<JournalPage, JournalError>;

    /// 更新日志。
    async fn update(&self, journal_id: i64, update: JournalUpdate)
    -> Result<Journal, JournalError>;

    /// 软删除日志。
    async fn delete(&self, journal_id: i64) -> Result<(), JournalError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_visibility_parses_only_known_values() {
        assert_eq!("public".parse(), Ok(JournalVisibility::Public));
        assert_eq!("private".parse(), Ok(JournalVisibility::Private));
        assert_eq!(
            "hidden".parse::<JournalVisibility>(),
            Err(JournalError::InvalidVisibility)
        );
    }

    #[test]
    fn journal_content_length_is_bounded() {
        assert!(validate_content("今天天气不错").is_ok());
        assert_eq!(validate_content("  "), Err(JournalError::ContentLength));
        assert_eq!(
            validate_content(&"a".repeat(2001)),
            Err(JournalError::ContentLength)
        );
    }
}
