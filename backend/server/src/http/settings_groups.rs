//! Admin 设置分组管理：appearance / email / integrations 三组的读取与更新。
//! email / ai 组为部分合并语义：Patch 全部字段可选，省略或显式 null 的字段保持不变，
//! 只更新提交的字段；appearance / integrations 组为全量覆盖。
//! Secret 字段（smtp_password / api_key）为 write-only：GET 只回 `*_set: bool`；
//! PATCH 中省略/null 表示保持不变，空字符串表示清除，非空表示更新。

use std::str::FromStr;

use aries_core::{
    ai::AiSettings,
    auth::{AuditEvent, Permission},
    settings::{
        AppearanceSettings, EmailSettings, IntegrationSettings, SettingError, SettingGroup,
        SettingGroupRecord, SettingGroupUpdate,
    },
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::state::AppState;

use super::{auth::CurrentUser, error::ApiError};

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/settings/{group}",
        get(get_setting_group).put(update_setting_group),
    )
}

#[derive(Debug, Serialize)]
struct SettingGroupResponse {
    group: String,
    version: i32,
    updated_at: OffsetDateTime,
    settings: serde_json::Value,
}

/// 邮件设置的公开视图：secret 不返回值，只回是否已设置。
#[derive(Debug, Serialize)]
struct EmailSettingsView {
    enabled: bool,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_username: Option<String>,
    smtp_password_set: bool,
    from_address: Option<String>,
    from_name: Option<String>,
}

impl From<EmailSettings> for EmailSettingsView {
    fn from(settings: EmailSettings) -> Self {
        Self {
            enabled: settings.enabled,
            smtp_host: settings.smtp_host,
            smtp_port: settings.smtp_port,
            smtp_username: settings.smtp_username,
            smtp_password_set: settings
                .smtp_password
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
            from_address: settings.from_address,
            from_name: settings.from_name,
        }
    }
}

/// AI 设置的公开视图：api_key 不回明文，只回是否已设置。
#[derive(Debug, Serialize)]
struct AiSettingsView {
    enabled: bool,
    protocol: aries_core::ai::AiProtocol,
    base_url: Option<String>,
    model: Option<String>,
    api_key_set: bool,
    features: aries_core::ai::AiFeatureToggles,
}

impl From<AiSettings> for AiSettingsView {
    fn from(settings: AiSettings) -> Self {
        Self {
            enabled: settings.enabled,
            protocol: settings.protocol,
            base_url: settings.base_url,
            model: settings.model,
            api_key_set: settings
                .api_key
                .as_deref()
                .is_some_and(|value| !value.is_empty()),
            features: settings.features,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SettingUpdateRequest {
    expected_version: i32,
    settings: serde_json::Value,
}

/// AI 设置更新入参：全部字段可选——`None`（字段缺省或显式 null）表示保持不变，
/// 只合并 `Some` 字段，部分 PATCH 不会重置未提交字段。
/// `api_key` 双层 Option 三态语义同 email 的 smtp_password。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AiSettingsPatch {
    enabled: Option<bool>,
    protocol: Option<aries_core::ai::AiProtocol>,
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<Option<String>>,
    features: Option<aries_core::ai::AiFeatureToggles>,
}

/// 邮件设置更新入参：全部字段可选——`None`（字段缺省或显式 null）表示保持不变，
/// 只合并 `Some` 字段，部分 PATCH 不会重置未提交字段。
/// `smtp_password` 用双层 Option 表达三种语义——
/// 字段缺省/null（`None`/`Some(None)`）保持不变，空字符串清除，非空更新。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EmailSettingsPatch {
    enabled: Option<bool>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_username: Option<String>,
    smtp_password: Option<Option<String>>,
    from_address: Option<String>,
    from_name: Option<String>,
}

fn parse_payload<T: serde::de::DeserializeOwned>(
    payload: serde_json::Value,
) -> Result<T, ApiError> {
    serde_json::from_value(payload).map_err(|_| SettingError::Validation.into())
}

/// 分组设置的公开视图：email 组屏蔽 secret 明文。
fn settings_view(record: &SettingGroupRecord) -> Result<serde_json::Value, ApiError> {
    match record.group {
        SettingGroup::Appearance => {
            let parsed: AppearanceSettings = parse_payload(record.payload.clone())?;
            serde_json::to_value(parsed).map_err(|_| SettingError::Validation.into())
        }
        SettingGroup::Email => {
            let parsed: EmailSettings = parse_payload(record.payload.clone())?;
            serde_json::to_value(EmailSettingsView::from(parsed))
                .map_err(|_| SettingError::Validation.into())
        }
        SettingGroup::Integrations => {
            let parsed: IntegrationSettings = parse_payload(record.payload.clone())?;
            serde_json::to_value(parsed).map_err(|_| SettingError::Validation.into())
        }
        SettingGroup::Ai => {
            let parsed: AiSettings = parse_payload(record.payload.clone())?;
            serde_json::to_value(AiSettingsView::from(parsed))
                .map_err(|_| SettingError::Validation.into())
        }
    }
}

fn to_response(record: SettingGroupRecord) -> Result<SettingGroupResponse, ApiError> {
    Ok(SettingGroupResponse {
        group: record.group.as_str().to_owned(),
        version: record.version,
        updated_at: record.updated_at,
        settings: settings_view(&record)?,
    })
}

async fn get_setting_group(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(group): Path<String>,
) -> Result<Json<SettingGroupResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let group = SettingGroup::from_str(&group)?;
    let record = state.settings.get_group(group).await?;
    Ok(Json(to_response(record)?))
}

async fn update_setting_group(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(group): Path<String>,
    Json(request): Json<SettingUpdateRequest>,
) -> Result<Json<SettingGroupResponse>, ApiError> {
    current.require(Permission::ManageSettings)?;
    let group = SettingGroup::from_str(&group)?;

    // email / ai 组的合并需要读取当前值：只覆盖 Patch 中提交的字段，
    // secret 三态——null/缺省保持，空串清除，非空覆盖。
    let payload = match group {
        SettingGroup::Appearance => {
            let parsed: AppearanceSettings = parse_payload(request.settings)?;
            serde_json::to_value(parsed).map_err(|_| ApiError::internal())?
        }
        SettingGroup::Email => {
            let patch: EmailSettingsPatch = parse_payload(request.settings)?;
            let current_record = state.settings.get_group(group).await?;
            let mut merged: EmailSettings = parse_payload(current_record.payload)?;
            // 只合并提交的字段：未提交字段（None）保持当前值，避免部分 PATCH 静默重置。
            if let Some(enabled) = patch.enabled {
                merged.enabled = enabled;
            }
            if let Some(smtp_host) = patch.smtp_host {
                merged.smtp_host = Some(smtp_host);
            }
            if let Some(smtp_port) = patch.smtp_port {
                merged.smtp_port = Some(smtp_port);
            }
            if let Some(smtp_username) = patch.smtp_username {
                merged.smtp_username = Some(smtp_username);
            }
            if let Some(from_address) = patch.from_address {
                merged.from_address = Some(from_address);
            }
            if let Some(from_name) = patch.from_name {
                merged.from_name = Some(from_name);
            }
            if let Some(secret) = patch.smtp_password {
                merged.smtp_password = secret.filter(|value| !value.is_empty());
            }
            serde_json::to_value(merged).map_err(|_| ApiError::internal())?
        }
        SettingGroup::Integrations => {
            let parsed: IntegrationSettings = parse_payload(request.settings)?;
            serde_json::to_value(parsed).map_err(|_| ApiError::internal())?
        }
        SettingGroup::Ai => {
            let patch: AiSettingsPatch = parse_payload(request.settings)?;
            let current_record = state.settings.get_group(group).await?;
            let mut merged: AiSettings = parse_payload(current_record.payload)?;
            // 只合并提交的字段：未提交字段（None）保持当前值，避免部分 PATCH 静默重置。
            if let Some(enabled) = patch.enabled {
                merged.enabled = enabled;
            }
            if let Some(protocol) = patch.protocol {
                merged.protocol = protocol;
            }
            if let Some(base_url) = patch.base_url {
                merged.base_url = Some(base_url);
            }
            if let Some(model) = patch.model {
                merged.model = Some(model);
            }
            if let Some(features) = patch.features {
                merged.features = features;
            }
            if let Some(secret) = patch.api_key {
                merged.api_key = secret.filter(|value| !value.is_empty());
            }
            serde_json::to_value(merged).map_err(|_| ApiError::internal())?
        }
    };

    let record = state
        .settings
        .update_group(
            group,
            SettingGroupUpdate {
                payload,
                expected_version: request.expected_version,
                updated_by: current.user.id,
            },
        )
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: format!("settings.{group}.updated"),
            target_type: "setting_group".to_owned(),
            target_id: Some(group.as_str().to_owned()),
            metadata: serde_json::json!({ "version": record.version }),
        })
        .await?;
    Ok(Json(to_response(record)?))
}
