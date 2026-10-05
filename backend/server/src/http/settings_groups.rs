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

use super::{auth::CurrentUser, error::ApiError, extract::ApiJson};

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/settings/{group}",
        get(get_setting_group).put(update_setting_group),
    )
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "SettingGroupResponse",
    description = "分组设置与乐观锁版本；`settings` 结构按 group 分别为 `AppearanceSettings` / `EmailSettingsView` / `IntegrationSettings` / `AiSettingsView`。"
)]
struct SettingGroupResponse {
    /// 分组名：`appearance` / `email` / `integrations` / `ai`。
    group: String,
    /// 乐观锁版本号，每次更新递增。
    version: i32,
    updated_at: OffsetDateTime,
    /// 分组设置内容，JSON 结构随 group 而不同。
    settings: serde_json::Value,
}

/// 邮件设置的公开视图：secret 不返回值，只回是否已设置。
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "EmailSettingsView",
    description = "邮件设置的公开视图；`smtp_password` 永不回传，只回 `smtp_password_set`。"
)]
struct EmailSettingsView {
    enabled: bool,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    smtp_username: Option<String>,
    /// 是否已配置 smtp_password（永不回传明文）。
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

/// `aries_core::ai::AiProtocol` 的 Schema 镜像（core 类型不能 derive ToSchema）。
#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
#[schema(
    title = "AiProtocol",
    description = "Provider 协议；openai 走 `/chat/completions` 兼容协议，anthropic 走 Messages API。旧 payload 无此字段时按 openai 处理。"
)]
enum AiProtocolSchema {
    #[schema(rename = "openai")]
    OpenAi,
    #[schema(rename = "anthropic")]
    Anthropic,
}

/// `aries_core::ai::AiFeatureToggles` 的 Schema 镜像（core 类型不能 derive ToSchema）。
#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
#[schema(
    title = "AiFeatureToggles",
    description = "AI 功能开关；`editor_assist` 控制编辑器助手端点；`comment_moderation` 控制评论 AI 审核；`smart_search` 控制相关文章推荐与站内 AI 问答（默认关闭，开启需配置 Embedding 端点）。"
)]
struct AiFeatureTogglesSchema {
    /// 编辑器助手（改写/摘要/元数据/标签/导读）。
    editor_assist: bool,
    /// 评论 AI 审核。
    comment_moderation: bool,
    /// 相关文章推荐与对话式搜索。
    smart_search: bool,
}

/// AI 设置的公开视图：api_key 不回明文，只回是否已设置。
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "AiSettingsView",
    description = "AI 设置的读取视图；`api_key` 为 write-only，只回 `api_key_set`。"
)]
struct AiSettingsView {
    enabled: bool,
    /// Provider 协议。
    #[schema(value_type = AiProtocolSchema)]
    protocol: aries_core::ai::AiProtocol,
    /// Provider Base URL；anthropic 缺省为 `https://api.anthropic.com`。
    base_url: Option<String>,
    model: Option<String>,
    /// 是否已配置 api_key（永不回传明文）。
    api_key_set: bool,
    #[schema(value_type = AiFeatureTogglesSchema)]
    features: aries_core::ai::AiFeatureToggles,
    /// Embedding 端点（OpenAI 兼容 `/embeddings`，Ollama 的 /v1 同样兼容）；未配置时 smart_search 相关能力不可用。
    embedding_base_url: Option<String>,
    /// Embedding 模型名，如 `bge-m3`。
    embedding_model: Option<String>,
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
            embedding_base_url: settings.embedding_base_url,
            embedding_model: settings.embedding_model,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(
    title = "UpdateSettingGroupRequest",
    description = "分组设置更新体。`settings` 结构按 group 而不同：appearance / integrations 组全量覆盖（`AppearanceSettings` / `IntegrationSettings`）；email / ai 组部分合并（`EmailSettingsPatch` / `AiSettingsPatch`），字段省略或 `null` 保持不变。乐观锁：`expected_version` 与当前版本不匹配返回 409 `SETTING_VERSION_CONFLICT`。"
)]
struct SettingUpdateRequest {
    /// 乐观锁版本号（先 GET 读取当前 version）。
    expected_version: i32,
    /// 分组设置内容，JSON 结构随 group 而不同（见 description）。
    settings: serde_json::Value,
}

/// AI 设置更新入参：全部字段可选——`None`（字段缺省或显式 null）表示保持不变，
/// 只合并 `Some` 字段，部分 PATCH 不会重置未提交字段。
/// `api_key` 双层 Option 三态语义同 email 的 smtp_password。
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
#[schema(
    title = "AiSettingsPatch",
    description = "AI 设置更新入参（部分合并语义）；所有字段可选，省略或 `null` 的字段保持当前值，只更新提交的字段。`api_key` 三态：省略或 `null` 保持不变、空字符串清除、非空字符串更新。"
)]
struct AiSettingsPatch {
    enabled: Option<bool>,
    /// Provider 协议。
    #[schema(value_type = Option<AiProtocolSchema>)]
    protocol: Option<aries_core::ai::AiProtocol>,
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<Option<String>>,
    #[schema(value_type = Option<AiFeatureTogglesSchema>)]
    features: Option<aries_core::ai::AiFeatureToggles>,
    embedding_base_url: Option<String>,
    embedding_model: Option<String>,
}

/// 邮件设置更新入参：全部字段可选——`None`（字段缺省或显式 null）表示保持不变，
/// 只合并 `Some` 字段，部分 PATCH 不会重置未提交字段。
/// `smtp_password` 用双层 Option 表达三种语义——
/// 字段缺省/null（`None`/`Some(None)`）保持不变，空字符串清除，非空更新。
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
#[schema(
    title = "EmailSettingsPatch",
    description = "邮件设置更新入参（部分合并语义）；所有字段可选，省略或 `null` 的字段保持当前值，只更新提交的字段。`smtp_password` 三态：省略或 `null` 保持不变、空字符串清除、非空字符串更新。"
)]
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

#[utoipa::path(
    get,
    path = "/api/admin/settings/{group}",
    tag = "Admin Settings Groups",
    operation_id = "getSettingGroup",
    summary = "读取分组设置",
    description = "读取分组设置，需要 `settings:manage` 权限。email 组的 `smtp_password`、ai 组的 `api_key` 为 write-only，响应只携带 `*_set` 表示是否已设置。",
    security(("cookieAuth" = [])),
    params(("group" = String, Path, description = "分组名：appearance / email / integrations / ai")),
    responses(
        (status = 200, description = "分组设置与乐观锁版本", body = SettingGroupResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "分组名非法（SETTING_GROUP_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
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

#[utoipa::path(
    put,
    path = "/api/admin/settings/{group}",
    tag = "Admin Settings Groups",
    operation_id = "updateSettingGroup",
    summary = "更新分组设置",
    description = "更新分组设置，需要 `settings:manage` 权限。乐观锁：`expected_version` 与当前版本不匹配返回 409 `SETTING_VERSION_CONFLICT`，成功后 `version` 递增。appearance / integrations 组为全量覆盖；email / ai 组为部分合并（PATCH 语义）：字段省略或 `null` 保持不变。email 组 `smtp_password`、ai 组 `api_key` 三态：省略或 `null` 保持不变、空字符串清除、非空字符串更新。",
    security(("cookieAuth" = [])),
    params(("group" = String, Path, description = "分组名：appearance / email / integrations / ai")),
    request_body(content = SettingUpdateRequest, content_type = "application/json", description = "分组设置更新体"),
    responses(
        (status = 200, description = "返回更新后的分组设置", body = SettingGroupResponse),
        (status = 400, description = "Payload 结构不合法（INVALID_SETTING_PAYLOAD）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 settings:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "分组名非法（SETTING_GROUP_NOT_FOUND）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "乐观锁版本冲突（SETTING_VERSION_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_setting_group(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(group): Path<String>,
    ApiJson(request): ApiJson<SettingUpdateRequest>,
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
            if let Some(embedding_base_url) = patch.embedding_base_url {
                merged.embedding_base_url = Some(embedding_base_url);
            }
            if let Some(embedding_model) = patch.embedding_model {
                merged.embedding_model = Some(embedding_model);
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
