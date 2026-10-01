use std::{convert::Infallible, net::SocketAddr, time::Duration as StdDuration};

use aries_core::auth::{
    AuditEvent, NewOwner, NewPasswordReset, NewSession, Permission, User, UserStatus,
    validate_password, validate_username,
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, FromRequestParts, OptionalFromRequestParts, State},
    http::{HeaderMap, StatusCode, header::USER_AGENT, request::Parts},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use cookie::{Cookie, SameSite, time::Duration as CookieDuration};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    security::{generate_token, hash_token, secrets_equal},
    state::AppState,
};

use super::error::ApiError;

const SESSION_COOKIE: &str = "aries_admin_session";
const LOGIN_LIMIT: usize = 5;
/// 每 IP 维度的登录配额：独立于账号维度，缓解定向锁定账号与分布式撞库。
const LOGIN_IP_LIMIT: usize = 30;
const RESET_LIMIT: usize = 3;
const RATE_WINDOW: StdDuration = StdDuration::from_secs(15 * 60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/bootstrap/status", get(bootstrap_status))
        .route("/bootstrap", post(bootstrap))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/session", get(session))
        .route("/auth/password/forgot", post(forgot_password))
        .route("/auth/password/reset", post(reset_password))
}

#[derive(Debug, Serialize)]
struct BootstrapStatusResponse {
    initialized: bool,
}

#[derive(Debug, Deserialize)]
struct BootstrapRequest {
    bootstrap_secret: String,
    username: String,
    email: String,
    display_name: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    login: String,
    password: String,
}

/// 连接对端地址：由 `into_make_service_with_connect_info` 注入请求扩展；
/// 集成测试 oneshot 无该扩展时退化为 `None`，login 再回退到 X-Forwarded-For / X-Real-IP。
struct ClientPeer(Option<SocketAddr>);

impl<S: Send + Sync> OptionalFromRequestParts<S> for ClientPeer {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(Some(ClientPeer(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct ForgotPasswordRequest {
    email: String,
}

#[derive(Debug, Deserialize)]
struct ResetPasswordRequest {
    token: String,
    password: String,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    id: i64,
    username: String,
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    role: String,
    permissions: Vec<&'static str>,
}

impl From<&User> for UserResponse {
    fn from(user: &User) -> Self {
        let permissions = [
            (Permission::ViewDashboard, "dashboard:view"),
            (Permission::ManageContent, "content:manage"),
            (Permission::ModerateComments, "comments:moderate"),
            (Permission::ManageUsers, "users:manage"),
            (Permission::ManageSettings, "settings:manage"),
            (Permission::ManageProfile, "profile:manage"),
        ]
        .into_iter()
        .filter_map(|(permission, name)| user.role.allows(permission).then_some(name))
        .collect();

        Self {
            id: user.id,
            username: user.username.clone(),
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            avatar_url: user.avatar_url.clone(),
            role: user.role.to_string(),
            permissions,
        }
    }
}

#[derive(Debug, Serialize)]
struct SessionResponse {
    user: UserResponse,
    expires_at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
struct MessageResponse {
    message: &'static str,
}

pub struct CurrentUser {
    pub session_id: Uuid,
    pub user: User,
    pub expires_at: OffsetDateTime,
}

impl CurrentUser {
    pub fn require(&self, permission: Permission) -> Result<(), ApiError> {
        if self.user.role.allows(permission) {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "PERMISSION_DENIED",
                "Permission denied",
            ))
        }
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let token = jar
            .get(SESSION_COOKIE)
            .map(|cookie| cookie.value())
            .ok_or_else(ApiError::unauthorized)?;
        let found = state
            .auth
            .find_session(&hash_token(token))
            .await?
            .ok_or_else(ApiError::unauthorized)?;
        if found.is_expired(OffsetDateTime::now_utc()) || found.user.status != UserStatus::Active {
            return Err(ApiError::unauthorized());
        }
        state.auth.touch_session(found.id).await?;

        Ok(Self {
            session_id: found.id,
            user: found.user,
            expires_at: found.expires_at,
        })
    }
}

async fn bootstrap_status(
    State(state): State<AppState>,
) -> Result<Json<BootstrapStatusResponse>, ApiError> {
    Ok(Json(BootstrapStatusResponse {
        initialized: state.auth.is_bootstrapped().await?,
    }))
}

async fn bootstrap(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(request): Json<BootstrapRequest>,
) -> Result<(CookieJar, Json<SessionResponse>), ApiError> {
    let decision = state
        .rate_limiter
        .check("bootstrap", LOGIN_LIMIT, RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }
    if !secrets_equal(&request.bootstrap_secret, &state.config.bootstrap_secret) {
        return Err(ApiError::forbidden(
            "INVALID_BOOTSTRAP_SECRET",
            "Bootstrap secret is invalid",
        ));
    }
    if state.auth.is_bootstrapped().await? {
        return Err(ApiError::conflict(
            "ALREADY_INITIALIZED",
            "Application is already initialized",
        ));
    }

    let username = request.username.trim().to_ascii_lowercase();
    let email = normalize_email(&request.email)?;
    let display_name = validate_display_name(&request.display_name)?;
    validate_username(&username)?;
    validate_password(&request.password)?;
    let password_hash = hash_password(&state, request.password).await?;
    let user = state
        .auth
        .create_owner(NewOwner {
            username,
            email,
            display_name,
            password_hash,
        })
        .await
        .map_err(|error| match error {
            aries_core::auth::AuthError::Conflict => {
                ApiError::conflict("ALREADY_INITIALIZED", "Application is already initialized")
            }
            other => other.into(),
        })?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(user.id),
            action: "auth.bootstrap".to_owned(),
            target_type: "user".to_owned(),
            target_id: Some(user.id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    info!(
        user_id = user.id,
        "bootstrap completed, first owner created"
    );

    issue_session(&state, jar, &headers, user).await
}

async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    peer: Option<ClientPeer>,
    Json(request): Json<LoginRequest>,
) -> Result<(CookieJar, Json<SessionResponse>), ApiError> {
    let login = request.login.trim().to_ascii_lowercase();
    let rate_key = format!("login:{login}");
    let decision = state
        .rate_limiter
        .check(&rate_key, LOGIN_LIMIT, RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }
    // 每 IP 独立配额：账号维度之外再限制单来源的请求速率，两种超限返回相同错误，防枚举。
    let ip = super::public::client_ip(&headers, peer.and_then(|ClientPeer(addr)| addr));
    let ip_rate_key = format!("login_ip:{ip}");
    let decision = state
        .rate_limiter
        .check(&ip_rate_key, LOGIN_IP_LIMIT, RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }

    let credentials = state.auth.find_credentials(&login).await?;
    let Some(credentials) = credentials else {
        // 未命中用户时仍执行一次 Argon2id，减少账号枚举的时间差异。
        let _ = hash_password(&state, request.password).await?;
        record_login_failure(&state, &login, None).await?;
        warn!("login failed: unknown account");
        return Err(ApiError::invalid_credentials());
    };

    let valid = verify_password(
        &state,
        request.password.clone(),
        credentials.password_hash.clone(),
    )
    .await?;
    if !valid || credentials.user.status != UserStatus::Active {
        record_login_failure(&state, &login, Some(credentials.user.id)).await?;
        warn!(
            user_id = credentials.user.id,
            "login failed: invalid password or inactive user"
        );
        return Err(ApiError::invalid_credentials());
    }

    // Legacy bcrypt Hash 在首次登录成功后升级为 Argon2id；升级失败不阻断登录。
    if state.passwords.needs_rehash(&credentials.password_hash) {
        match hash_password(&state, request.password).await {
            Ok(upgraded) => {
                match state
                    .auth
                    .update_password(credentials.user.id, &upgraded)
                    .await
                {
                    Ok(()) => {
                        info!(
                            user_id = credentials.user.id,
                            "legacy bcrypt password upgraded to argon2id"
                        );
                    }
                    Err(error) => {
                        warn!(
                            user_id = credentials.user.id,
                            ?error,
                            "legacy password upgrade failed"
                        );
                    }
                }
            }
            Err(error) => {
                warn!(
                    user_id = credentials.user.id,
                    ?error,
                    "legacy password rehash failed"
                );
            }
        }
    }

    revoke_cookie_session(&state, &jar).await?;
    state.auth.update_last_login(credentials.user.id).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(credentials.user.id),
            action: "auth.login_succeeded".to_owned(),
            target_type: "session".to_owned(),
            target_id: None,
            metadata: serde_json::json!({}),
        })
        .await?;
    info!(user_id = credentials.user.id, "login succeeded");

    issue_session(&state, jar, &headers, credentials.user).await
}

async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
    current: CurrentUser,
) -> Result<(CookieJar, StatusCode), ApiError> {
    state.auth.revoke_session(current.session_id).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "auth.logout".to_owned(),
            target_type: "session".to_owned(),
            target_id: Some(current.session_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    info!(user_id = current.user.id, "logout");
    Ok((remove_session_cookie(jar, &state), StatusCode::NO_CONTENT))
}

async fn session(current: CurrentUser) -> Json<SessionResponse> {
    Json(SessionResponse {
        user: UserResponse::from(&current.user),
        expires_at: current.expires_at,
    })
}

async fn forgot_password(
    State(state): State<AppState>,
    Json(request): Json<ForgotPasswordRequest>,
) -> Result<(StatusCode, Json<MessageResponse>), ApiError> {
    let email = normalize_email(&request.email)?;
    let decision = state
        .rate_limiter
        .check(&format!("forgot:{email}"), RESET_LIMIT, RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }

    if let Some(user) = state.auth.find_user_by_email(&email).await? {
        let token = generate_token()?;
        state
            .auth
            .create_password_reset(NewPasswordReset {
                id: Uuid::now_v7(),
                user_id: user.id,
                token_hash: hash_token(&token),
                expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(30),
            })
            .await?;
        state
            .auth
            .write_audit(AuditEvent {
                actor_user_id: Some(user.id),
                action: "auth.password_reset_requested".to_owned(),
                target_type: "user".to_owned(),
                target_id: Some(user.id.to_string()),
                // Phase 05 接入 Email Adapter 前，不记录或返回原始 Reset Token。
                metadata: serde_json::json!({ "delivery": "not_configured" }),
            })
            .await?;
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(MessageResponse {
            message: "If the account exists, password reset instructions will be sent",
        }),
    ))
}

async fn reset_password(
    State(state): State<AppState>,
    Json(request): Json<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    validate_password(&request.password)?;
    let token_hash = hash_token(&request.token);
    let rate_key = format!("reset:{}", hex_prefix(&token_hash));
    let decision = state
        .rate_limiter
        .check(&rate_key, RESET_LIMIT, RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }

    let password_hash = hash_password(&state, request.password).await?;
    let user_id = state
        .auth
        .consume_password_reset(&token_hash, &password_hash)
        .await?
        .ok_or_else(|| {
            ApiError::bad_request("INVALID_RESET_TOKEN", "Reset token is invalid or expired")
        })?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(user_id),
            action: "auth.password_reset_completed".to_owned(),
            target_type: "user".to_owned(),
            target_id: Some(user_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    info!(user_id, "password reset completed");
    Ok(StatusCode::NO_CONTENT)
}

async fn issue_session(
    state: &AppState,
    jar: CookieJar,
    headers: &HeaderMap,
    user: User,
) -> Result<(CookieJar, Json<SessionResponse>), ApiError> {
    let token = generate_token()?;
    let expires_at = OffsetDateTime::now_utc() + state.config.session_ttl;
    state
        .auth
        .create_session(NewSession {
            id: Uuid::now_v7(),
            token_hash: hash_token(&token),
            user_id: user.id,
            expires_at,
            user_agent: headers
                .get(USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.chars().take(512).collect()),
        })
        .await?;

    let cookie = Cookie::build((SESSION_COOKIE, token))
        .path("/api/admin")
        .http_only(true)
        .secure(state.config.cookie_secure)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::seconds(
            state.config.session_ttl.whole_seconds(),
        ))
        .build();
    Ok((
        jar.add(cookie),
        Json(SessionResponse {
            user: UserResponse::from(&user),
            expires_at,
        }),
    ))
}

async fn revoke_cookie_session(state: &AppState, jar: &CookieJar) -> Result<(), ApiError> {
    let Some(token) = jar.get(SESSION_COOKIE).map(|cookie| cookie.value()) else {
        return Ok(());
    };
    if let Some(session) = state.auth.find_session(&hash_token(token)).await? {
        state.auth.revoke_session(session.id).await?;
    }
    Ok(())
}

pub fn remove_session_cookie(jar: CookieJar, state: &AppState) -> CookieJar {
    let cookie = Cookie::build((SESSION_COOKIE, ""))
        .path("/api/admin")
        .http_only(true)
        .secure(state.config.cookie_secure)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::ZERO)
        .build();
    jar.remove(cookie)
}

async fn record_login_failure(
    state: &AppState,
    login: &str,
    user_id: Option<i64>,
) -> Result<(), ApiError> {
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: user_id,
            action: "auth.login_failed".to_owned(),
            target_type: "user".to_owned(),
            target_id: user_id.map(|value| value.to_string()),
            metadata: serde_json::json!({ "login": login }),
        })
        .await?;
    Ok(())
}

pub(crate) async fn hash_password(state: &AppState, password: String) -> Result<String, ApiError> {
    let hasher = state.passwords.clone();
    tokio::task::spawn_blocking(move || hasher.hash(&password))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(Into::into)
}

pub(crate) async fn verify_password(
    state: &AppState,
    password: String,
    password_hash: String,
) -> Result<bool, ApiError> {
    let hasher = state.passwords.clone();
    tokio::task::spawn_blocking(move || hasher.verify(&password, &password_hash))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(Into::into)
}

pub(crate) fn normalize_email(email: &str) -> Result<String, ApiError> {
    let email = email.trim().to_ascii_lowercase();
    let (local, domain) = email.split_once('@').unwrap_or_default();
    if email.len() > 254
        || local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || email.chars().any(char::is_whitespace)
    {
        return Err(ApiError::bad_request(
            "INVALID_EMAIL",
            "Email format is invalid",
        ));
    }
    Ok(email)
}

pub(crate) fn validate_display_name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if !(1..=60).contains(&value.chars().count()) {
        return Err(ApiError::bad_request(
            "INVALID_DISPLAY_NAME",
            "Display name must contain 1 to 60 characters",
        ));
    }
    Ok(value.to_owned())
}

fn hex_prefix(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
