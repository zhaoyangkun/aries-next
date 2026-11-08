//! 设置分组域：appearance / email / integrations / ai / social 五组结构化 JSONB 配置。
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
    Social,
}

impl SettingGroup {
    /// 返回分组的小写字符串标识，与数据库 `grp` 列取值一致。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Appearance => "appearance",
            Self::Email => "email",
            Self::Integrations => "integrations",
            Self::Ai => "ai",
            Self::Social => "social",
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

    /// 解析分组字符串；未知取值返回 `SettingError::InvalidGroup`。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "appearance" => Ok(Self::Appearance),
            "email" => Ok(Self::Email),
            "integrations" => Ok(Self::Integrations),
            "ai" => Ok(Self::Ai),
            "social" => Ok(Self::Social),
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

/// 社交链接条目：公开端 footer 展示的外部站点链接（Owner 配置，公开只读投影）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocialLink {
    /// 展示名，Trim 后 1–20 字符（如 `GitHub`）。
    pub name: String,
    /// 外部站点 http/https 绝对地址。
    pub url: String,
    /// 前端图标键；未知值由前端退化为通用链接图标。
    pub icon: Option<String>,
}

/// 社交链接设置：有序列表，展示顺序即数组顺序；全量覆盖更新。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SocialSettings {
    pub links: Vec<SocialLink>,
}

impl SocialSettings {
    pub const MAX_LINKS: usize = 10;
    pub const MAX_NAME_CHARS: usize = 20;
    pub const MAX_ICON_CHARS: usize = 20;

    /// 校验链接列表：数量上限、名称非空且限长、URL 必须是 http/https 绝对地址、按 URL 去重。
    pub fn validate(&self) -> Result<(), SettingError> {
        if self.links.len() > Self::MAX_LINKS {
            return Err(SettingError::Validation);
        }
        let mut seen = std::collections::HashSet::with_capacity(self.links.len());
        for link in &self.links {
            let name = link.name.trim();
            if name.is_empty() || name.chars().count() > Self::MAX_NAME_CHARS {
                return Err(SettingError::Validation);
            }
            if !is_absolute_http_url(&link.url) {
                return Err(SettingError::Validation);
            }
            let icon = link.icon.as_deref().map(str::trim).unwrap_or("");
            if icon.chars().count() > Self::MAX_ICON_CHARS {
                return Err(SettingError::Validation);
            }
            if !seen.insert(link.url.trim().to_owned()) {
                return Err(SettingError::Validation);
            }
        }
        Ok(())
    }
}

/// 保守的 http/https 绝对地址判断：显式 scheme + 非空 Host 段，
/// 拒绝协议相对（//host）与含空白字符的输入；足够挡住 javascript: 等非 http Scheme。
fn is_absolute_http_url(value: &str) -> bool {
    let value = value.trim();
    let host = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"));
    match host {
        Some(host) => {
            !host.is_empty()
                && !host.starts_with('/')
                && !host.contains(char::is_whitespace)
                && host.contains('.')
        }
        None => false,
    }
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

/// 设置分组领域错误。
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
        assert_eq!("social".parse(), Ok(SettingGroup::Social));
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

    #[test]
    fn social_settings_validate_links() {
        let valid = SocialSettings {
            links: vec![
                SocialLink {
                    name: "GitHub".to_owned(),
                    url: "https://github.com/example".to_owned(),
                    icon: Some("github".to_owned()),
                },
                SocialLink {
                    name: "微博".to_owned(),
                    url: "http://weibo.com/example".to_owned(),
                    icon: None,
                },
            ],
        };
        assert_eq!(valid.validate(), Ok(()));

        // 空名称 / 超长名称
        let mut invalid = valid.clone();
        invalid.links[0].name = "  ".to_owned();
        assert_eq!(invalid.validate(), Err(SettingError::Validation));
        let mut invalid = valid.clone();
        invalid.links[0].name = "长".repeat(SocialSettings::MAX_NAME_CHARS + 1);
        assert_eq!(invalid.validate(), Err(SettingError::Validation));

        // 非 http(s) 绝对地址：javascript Scheme、协议相对、站内路径
        for bad_url in [
            "javascript:alert(1)",
            "//evil.example.com",
            "/about",
            "https://",
        ] {
            let mut invalid = valid.clone();
            invalid.links[0].url = bad_url.to_owned();
            assert_eq!(
                invalid.validate(),
                Err(SettingError::Validation),
                "{bad_url}"
            );
        }

        // URL 去重
        let mut invalid = valid.clone();
        invalid.links[1].url = invalid.links[0].url.clone();
        assert_eq!(invalid.validate(), Err(SettingError::Validation));

        // 数量上限
        let too_many = SocialSettings {
            links: (0..=SocialSettings::MAX_LINKS)
                .map(|index| SocialLink {
                    name: format!("S{index}"),
                    url: format!("https://example.com/{index}"),
                    icon: None,
                })
                .collect(),
        };
        assert_eq!(too_many.validate(), Err(SettingError::Validation));
    }

    #[test]
    fn social_settings_tolerate_missing_links() {
        // jsonb 缺省 links 字段应反序列化为空列表，而不是报错。
        let settings: SocialSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, SocialSettings::default());
        assert_eq!(settings.validate(), Ok(()));
    }
}
