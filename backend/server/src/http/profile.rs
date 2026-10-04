use aries_core::auth::{AuditEvent, Permission, ProfileUpdate, UserStatus, validate_password};
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use crate::state::AppState;

use super::{
    auth::{
        CurrentUser, UserResponse, hash_password, normalize_email, remove_session_cookie,
        validate_display_name, verify_password,
    },
    error::ApiError,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/profile", get(get_profile).put(update_profile))
        .route("/profile/password", axum::routing::put(update_password))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(
    title = "UpdateProfileRequest",
    description = "更新当前登录用户的 Profile。"
)]
struct UpdateProfileRequest {
    /// 邮箱；会 Normalize 并校验格式与唯一性，冲突返回 409。
    email: String,
    /// 显示名；长度与字符白名单校验，非法返回 400。
    display_name: String,
    /// 头像 URL；缺省/null/空串表示清除，非空必须是 http/https 绝对 URL（400 `INVALID_AVATAR_URL`）。
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[schema(
    title = "UpdatePasswordRequest",
    description = "修改当前登录用户密码；成功后撤销该用户全部 Session 并清除当前 Cookie。"
)]
struct UpdatePasswordRequest {
    /// 当前密码；校验失败返回 400 `INVALID_CURRENT_PASSWORD`。
    current_password: String,
    /// 新密码；需通过 Password Policy 校验（400 `INVALID_PASSWORD`）。
    new_password: String,
}

#[utoipa::path(
    get,
    path = "/api/admin/profile",
    tag = "Admin Profile",
    operation_id = "getProfile",
    summary = "当前登录用户 Profile",
    description = "返回当前 Session 对应的 User Profile，含 Role 与权限列表。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "当前 User Profile", body = super::auth::UserResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 profile:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_profile(current: CurrentUser) -> Result<Json<UserResponse>, ApiError> {
    current.require(Permission::ManageProfile)?;
    Ok(Json(UserResponse::from(&current.user)))
}

#[utoipa::path(
    put,
    path = "/api/admin/profile",
    tag = "Admin Profile",
    operation_id = "updateProfile",
    summary = "更新当前用户 Profile",
    description = "更新邮箱、显示名与头像；写 Audit（`profile.updated`）。邮箱与其他用户冲突返回 409。",
    security(("cookieAuth" = [])),
    request_body(content = UpdateProfileRequest, content_type = "application/json", description = "Profile 入参"),
    responses(
        (status = 200, description = "返回更新后的 User Profile", body = super::auth::UserResponse),
        (status = 400, description = "邮箱/显示名/头像 URL 校验失败", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 profile:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 409, description = "邮箱已被其他用户使用", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_profile(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<UpdateProfileRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    current.require(Permission::ManageProfile)?;
    let email = normalize_email(&request.email)?;
    let display_name = validate_display_name(&request.display_name)?;
    let avatar_url = validate_avatar_url(request.avatar_url)?;
    let user = state
        .auth
        .update_profile(
            current.user.id,
            ProfileUpdate {
                email,
                display_name,
                avatar_url,
            },
        )
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "profile.updated".to_owned(),
            target_type: "user".to_owned(),
            target_id: Some(current.user.id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(Json(UserResponse::from(&user)))
}

#[utoipa::path(
    put,
    path = "/api/admin/profile/password",
    tag = "Admin Profile",
    operation_id = "updatePassword",
    summary = "修改当前用户密码",
    description = "校验当前密码后更新为新密码；成功后撤销该用户全部 Session 并清除当前 Cookie，写 Audit（`profile.password_changed`）。",
    security(("cookieAuth" = [])),
    request_body(content = UpdatePasswordRequest, content_type = "application/json", description = "密码修改入参"),
    responses(
        (status = 204, description = "密码已更新，Session 已全部撤销"),
        (status = 400, description = "当前密码错误或新密码不通过 Policy 校验", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 profile:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_password(
    State(state): State<AppState>,
    jar: CookieJar,
    current: CurrentUser,
    Json(request): Json<UpdatePasswordRequest>,
) -> Result<(CookieJar, StatusCode), ApiError> {
    current.require(Permission::ManageProfile)?;
    validate_password(&request.new_password)?;
    let credentials = state
        .auth
        .find_credentials(&current.user.username)
        .await?
        .filter(|credentials| {
            credentials.user.id == current.user.id && credentials.user.status == UserStatus::Active
        })
        .ok_or_else(ApiError::unauthorized)?;
    if !verify_password(&state, request.current_password, credentials.password_hash).await? {
        return Err(ApiError::bad_request(
            "INVALID_CURRENT_PASSWORD",
            "Current password is invalid",
        ));
    }

    let password_hash = hash_password(&state, request.new_password).await?;
    state
        .auth
        .update_password(current.user.id, &password_hash)
        .await?;
    state.auth.revoke_user_sessions(current.user.id).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "profile.password_changed".to_owned(),
            target_type: "user".to_owned(),
            target_id: Some(current.user.id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok((remove_session_cookie(jar, &state), StatusCode::NO_CONTENT))
}

fn validate_avatar_url(value: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 2048
        || !(value.starts_with("https://") || value.starts_with("http://"))
        || value.parse::<axum::http::Uri>().is_err()
    {
        return Err(ApiError::bad_request(
            "INVALID_AVATAR_URL",
            "Avatar URL is invalid",
        ));
    }
    Ok(Some(value.to_owned()))
}
