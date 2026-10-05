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

use super::{error::ApiError, extract::ApiJson};

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

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct BootstrapStatusResponse {
    /// 是否已创建首个 User。
    initialized: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct BootstrapRequest {
    /// 一次性引导密钥（环境变量 BOOTSTRAP_SECRET，≥ 24 字符）。
    bootstrap_secret: String,
    /// 登录用户名：3–30 位字母、数字、下划线或短横线。
    username: String,
    /// 邮箱地址。
    email: String,
    /// 显示名：1–60 字符。
    display_name: String,
    /// 初始密码：10–128 字符。
    password: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct LoginRequest {
    /// 用户名或邮箱。
    login: String,
    /// 密码。
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct ForgotPasswordRequest {
    /// 邮箱地址。
    email: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct ResetPasswordRequest {
    /// Password Reset Token。
    token: String,
    /// 新密码：10–128 字符。
    password: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct UserResponse {
    /// User ID。
    id: i64,
    /// 登录用户名。
    username: String,
    /// 邮箱地址。
    email: String,
    /// 显示名。
    display_name: String,
    /// 头像 URL，未设置时为 null。
    avatar_url: Option<String>,
    /// 角色：owner / editor / moderator。
    role: String,
    /// 该角色拥有的权限标识列表。
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct SessionResponse {
    /// 当前 User 视图。
    user: UserResponse,
    /// Session 过期时间（UTC）。
    expires_at: OffsetDateTime,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct MessageResponse {
    /// 人类可读结果信息。
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

#[utoipa::path(
    get,
    path = "/api/admin/bootstrap/status",
    tag = "Admin Auth",
    operation_id = "bootstrapStatus",
    summary = "查询系统初始化状态",
    description = "返回是否已创建首个 User；未初始化时前端应引导进入 Bootstrap 流程。",
    responses(
        (status = 200, description = "是否已创建首个 User", body = BootstrapStatusResponse),
        (status = 500, description = "数据库查询失败", body = crate::openapi::ErrorResponse),
    )
)]
async fn bootstrap_status(
    State(state): State<AppState>,
) -> Result<Json<BootstrapStatusResponse>, ApiError> {
    Ok(Json(BootstrapStatusResponse {
        initialized: state.auth.is_bootstrapped().await?,
    }))
}

#[utoipa::path(
    post,
    path = "/api/admin/bootstrap",
    tag = "Admin Auth",
    operation_id = "bootstrap",
    summary = "创建首个 Owner 并完成初始化",
    description = "校验一次性 bootstrap_secret 后创建首个 Owner User，并签发 Session Cookie。仅限未初始化时调用，限流 5 次/15 分钟。",
    request_body = BootstrapRequest,
    responses(
        (status = 200, description = "创建首个 Owner 并签发 Session Cookie", body = SessionResponse),
        (status = 400, description = "用户名、邮箱、显示名或密码不合法", body = crate::openapi::ErrorResponse),
        (status = 403, description = "bootstrap_secret 无效", body = crate::openapi::ErrorResponse),
        (status = 409, description = "系统已初始化", body = crate::openapi::ErrorResponse),
        (status = 429, description = "触发限流", body = crate::openapi::ErrorResponse),
    )
)]
async fn bootstrap(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    ApiJson(request): ApiJson<BootstrapRequest>,
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

#[utoipa::path(
    post,
    path = "/api/admin/auth/login",
    tag = "Admin Auth",
    operation_id = "login",
    summary = "管理员登录",
    description = "用户名或邮箱 + 密码认证；成功后撤销旧 Session、签发新 Session Cookie（HttpOnly，Path 限定 /api/admin）。账号维度 5 次/15 分钟、IP 维度 30 次/15 分钟双重限流，未命中用户也执行一次 Argon2id 以掩盖账号枚举的时间差异。",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "认证成功并签发 Session Cookie", body = SessionResponse),
        (status = 401, description = "用户名或密码错误，或账号已被禁用", body = crate::openapi::ErrorResponse),
        (status = 403, description = "Origin 校验失败", body = crate::openapi::ErrorResponse),
        (status = 429, description = "触发限流", body = crate::openapi::ErrorResponse),
    )
)]
async fn login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
    peer: Option<ClientPeer>,
    ApiJson(request): ApiJson<LoginRequest>,
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

#[utoipa::path(
    post,
    path = "/api/admin/auth/logout",
    tag = "Admin Auth",
    operation_id = "logout",
    summary = "管理员登出",
    description = "撤销当前 Session 并清除 Session Cookie，返回 204。",
    security(("cookieAuth" = [])),
    responses(
        (status = 204, description = "撤销当前 Session 并清除 Cookie"),
        (status = 401, description = "未认证或 Session 已失效", body = crate::openapi::ErrorResponse),
        (status = 403, description = "Origin 校验失败", body = crate::openapi::ErrorResponse),
    )
)]
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

#[utoipa::path(
    get,
    path = "/api/admin/auth/session",
    tag = "Admin Auth",
    operation_id = "getSession",
    summary = "查询当前会话",
    description = "返回当前 Session 对应的 User 视图与 Session 过期时间；无有效 Session Cookie 时返回 401。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "当前 Session 与 User 视图", body = SessionResponse),
        (status = 401, description = "未认证或 Session 已失效", body = crate::openapi::ErrorResponse),
    )
)]
async fn session(current: CurrentUser) -> Json<SessionResponse> {
    Json(SessionResponse {
        user: UserResponse::from(&current.user),
        expires_at: current.expires_at,
    })
}

#[utoipa::path(
    post,
    path = "/api/admin/auth/password/forgot",
    tag = "Admin Auth",
    operation_id = "forgotPassword",
    summary = "请求密码重置",
    description = "无论账号是否存在都返回相同响应（202），避免账号枚举；每邮箱 3 次/15 分钟限流。Phase 05 接入 Email Adapter 前不会发送 Email，Reset Token 不落库明文。",
    request_body = ForgotPasswordRequest,
    responses(
        (status = 202, description = "无论账号是否存在都返回相同响应", body = MessageResponse),
        (status = 400, description = "邮箱格式不合法", body = crate::openapi::ErrorResponse),
        (status = 403, description = "Origin 校验失败", body = crate::openapi::ErrorResponse),
        (status = 429, description = "触发限流", body = crate::openapi::ErrorResponse),
    )
)]
async fn forgot_password(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ForgotPasswordRequest>,
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

#[utoipa::path(
    post,
    path = "/api/admin/auth/password/reset",
    tag = "Admin Auth",
    operation_id = "resetPassword",
    summary = "重置密码",
    description = "凭 Password Reset Token 设置新密码，成功后撤销该 User 的全部 Session，返回 204；Token 无效或已过期时返回 400。每 Token 维度 3 次/15 分钟限流。",
    request_body = ResetPasswordRequest,
    responses(
        (status = 204, description = "更新 Password 并撤销该 User 的全部 Session"),
        (status = 400, description = "Token 无效或已过期，或新密码不合法", body = crate::openapi::ErrorResponse),
        (status = 403, description = "Origin 校验失败", body = crate::openapi::ErrorResponse),
        (status = 429, description = "触发限流", body = crate::openapi::ErrorResponse),
    )
)]
async fn reset_password(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<ResetPasswordRequest>,
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
