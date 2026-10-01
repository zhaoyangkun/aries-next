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

#[derive(Debug, Deserialize)]
struct UpdateProfileRequest {
    email: String,
    display_name: String,
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdatePasswordRequest {
    current_password: String,
    new_password: String,
}

async fn get_profile(current: CurrentUser) -> Result<Json<UserResponse>, ApiError> {
    current.require(Permission::ManageProfile)?;
    Ok(Json(UserResponse::from(&current.user)))
}

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
