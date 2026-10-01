//! 设置分组域：appearance / email / integrations 三组结构化 JSONB 配置。
//! Secret 字段（如 smtp_password）为 write-only：infra 只负责存取 payload，
//! 读取时由 HTTP 层转换为 `*_set: bool`，不回向客户端暴露明文（Phase 06 §3）。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

/// 设置分组标识，与 `setting_groups.grp` 的 CHECK 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingGroup {
    Appearance,
    Email,
    Integrations,
    Ai,
}

impl SettingGroup {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Email => "email",
            Self::Integrations => "integrations",
            Self::Ai => "ai",
        }
    }
}

impl fmt::Display for SettingGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SettingGroup {
    type Err = SettingError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "appearance" => Ok(Self::Appearance),
            "email" => Ok(Self::Email),
            "integrations" => Ok(Self::Integrations),
            "ai" => Ok(Self::Ai),
            _ => Err(SettingError::InvalidGroup),
        }
    }
}

/// 颜色偏好：跟随系统 / 固定亮色 / 固定暗色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSchemePreference {
    System,
    Light,
    Dark,
}

/// 文章列表密度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListDensity {
    Comfortable,
    Compact,
}

/// 外观设置：只存结构化字段，禁止任意 Script/head_content 注入（Phase 06 §6）。
/// `#[serde(default)]` 允许 jsonb 中缺省字段，便于增量演进。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub color_scheme: Option<ColorSchemePreference>,
    pub list_density: Option<ListDensity>,
}

/// 邮件设置：`smtp_password` 为 write-only secret——
/// PUT 省略/null 表示保持不变，空字符串表示清除；GET 永不回传明文。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailSettings {
    pub enabled: bool,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
}

/// 第三方集成设置：受控 Integration Slot，仅结构化字段。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationSettings {
    pub analytics_id: Option<String>,
    pub site_verification_token: Option<String>,
}

/// 设置分组记录：payload 为原始 jsonb，结构化类型由 HTTP 层反序列化校验。
#[derive(Debug, Clone)]
pub struct SettingGroupRecord {
    pub group: SettingGroup,
    pub payload: serde_json::Value,
    pub version: i32,
    pub updated_by: Option<i64>,
    pub updated_at: OffsetDateTime,
}

/// 设置更新入参：`expected_version` 用于乐观锁，过期提交返回 `Conflict`。
#[derive(Debug, Clone)]
pub struct SettingGroupUpdate {
    pub payload: serde_json::Value,
    pub expected_version: i32,
    pub updated_by: i64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SettingError {
    #[error("invalid setting group")]
    InvalidGroup,
    #[error("setting group not found")]
    NotFound,
    /// 乐观锁冲突：提交基于过期版本。
    #[error("setting version conflict")]
    Conflict,
    #[error("setting payload is invalid")]
    Validation,
    #[error("setting store unavailable")]
    StoreUnavailable,
}

/// 设置 Repository 接口，由 infra 层实现。
#[async_trait]
pub trait SettingRepository: Send + Sync {
    /// 读取分组设置；三组默认行由 Migration 预插，正常不会 miss。
    async fn get_group(&self, group: SettingGroup) -> Result<SettingGroupRecord, SettingError>;

    /// 更新分组设置（乐观锁）：版本不匹配返回 `Conflict`，成功后 version + 1。
    async fn update_group(
        &self,
        group: SettingGroup,
        update: SettingGroupUpdate,
    ) -> Result<SettingGroupRecord, SettingError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_group_parses_only_known_values() {
        assert_eq!("appearance".parse(), Ok(SettingGroup::Appearance));
        assert_eq!("email".parse(), Ok(SettingGroup::Email));
        assert_eq!("integrations".parse(), Ok(SettingGroup::Integrations));
        assert_eq!("ai".parse(), Ok(SettingGroup::Ai));
        assert_eq!(
            "site".parse::<SettingGroup>(),
            Err(SettingError::InvalidGroup)
        );
    }

    #[test]
    fn appearance_settings_tolerate_missing_fields() {
        // jsonb 中缺省字段应反序列化为 None，而不是报错。
        let settings: AppearanceSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, AppearanceSettings::default());

        let settings: AppearanceSettings =
            serde_json::from_str(r#"{"color_scheme": "dark", "list_density": "compact"}"#).unwrap();
        assert_eq!(settings.color_scheme, Some(ColorSchemePreference::Dark));
        assert_eq!(settings.list_density, Some(ListDensity::Compact));
        assert_eq!(settings.logo_url, None);
    }

    #[test]
    fn email_settings_secret_roundtrips_through_jsonb() {
        let settings = EmailSettings {
            enabled: true,
            smtp_host: Some("smtp.example.com".to_owned()),
            smtp_port: Some(465),
            smtp_password: Some("secret".to_owned()),
            ..EmailSettings::default()
        };
        let json = serde_json::to_value(&settings).unwrap();
        let parsed: EmailSettings = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, settings);
    }
}
