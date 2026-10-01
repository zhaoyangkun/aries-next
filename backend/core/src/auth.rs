use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Editor,
    Moderator,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Editor => "editor",
            Self::Moderator => "moderator",
        }
    }

    pub const fn allows(self, permission: Permission) -> bool {
        match self {
            Self::Owner => true,
            Self::Editor => matches!(
                permission,
                Permission::ViewDashboard | Permission::ManageContent | Permission::ManageProfile
            ),
            Self::Moderator => matches!(
                permission,
                Permission::ViewDashboard
                    | Permission::ModerateComments
                    | Permission::ManageProfile
            ),
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = AuthError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "editor" => Ok(Self::Editor),
            "moderator" => Ok(Self::Moderator),
            _ => Err(AuthError::InvalidRole),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ViewDashboard,
    ManageContent,
    ModerateComments,
    ManageUsers,
    ManageSettings,
    ManageProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Disabled,
}

impl UserStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }
}

impl FromStr for UserStatus {
    type Err = AuthError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            _ => Err(AuthError::InvalidUserStatus),
        }
    }
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub role: Role,
    pub status: UserStatus,
}

#[derive(Debug, Clone)]
pub struct CredentialUser {
    pub user: User,
    pub password_hash: String,
}

#[derive(Debug, Clone)]
pub struct NewOwner {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
}

#[derive(Debug, Clone)]
pub struct ProfileUpdate {
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: Uuid,
    pub token_hash: Vec<u8>,
    pub user_id: i64,
    pub expires_at: OffsetDateTime,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AuthenticatedSession {
    pub id: Uuid,
    pub user: User,
    pub expires_at: OffsetDateTime,
}

impl AuthenticatedSession {
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        self.expires_at <= now
    }
}

#[derive(Debug, Clone)]
pub struct NewPasswordReset {
    pub id: Uuid,
    pub user_id: i64,
    pub token_hash: Vec<u8>,
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub actor_user_id: Option<i64>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub metadata: serde_json::Value,
}

/// Audit Log 列表查询参数。所有筛选字段可选，未提供时不过滤。
#[derive(Debug, Clone, Default)]
pub struct AuditListQuery {
    pub page: u32,
    pub page_size: u32,
    pub actor_user_id: Option<i64>,
    pub action: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    /// 时间范围按 `created_at` 左闭右开过滤。
    pub start: Option<OffsetDateTime>,
    pub end: Option<OffsetDateTime>,
}

/// Audit Log 单条记录，`actor_username` 用于展示（用户被删除时为 None）。
#[derive(Debug, Clone)]
pub struct AuditLogEntry {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    pub actor_username: Option<String>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: OffsetDateTime,
}

/// 分页 Audit Log。
#[derive(Debug, Clone)]
pub struct AuditPage {
    pub items: Vec<AuditLogEntry>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AuthError {
    #[error("invalid role")]
    InvalidRole,
    #[error("invalid user status")]
    InvalidUserStatus,
    #[error("username must contain 3 to 30 characters")]
    InvalidUsername,
    #[error("password must contain 10 to 128 characters")]
    InvalidPasswordLength,
    #[error("password must include letters and numbers")]
    WeakPassword,
    #[error("resource conflict")]
    Conflict,
    #[error("resource not found")]
    NotFound,
    #[error("authentication store unavailable")]
    StoreUnavailable,
}

pub fn validate_username(username: &str) -> Result<(), AuthError> {
    let length = username.chars().count();
    if !(3..=30).contains(&length)
        || !username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(AuthError::InvalidUsername);
    }
    Ok(())
}

pub fn validate_password(password: &str) -> Result<(), AuthError> {
    let length = password.chars().count();
    if !(10..=128).contains(&length) {
        return Err(AuthError::InvalidPasswordLength);
    }

    let has_letter = password.chars().any(|character| character.is_alphabetic());
    let has_number = password.chars().any(|character| character.is_ascii_digit());
    if !has_letter || !has_number {
        return Err(AuthError::WeakPassword);
    }
    Ok(())
}

pub trait PasswordHasher: Send + Sync {
    fn hash(&self, password: &str) -> Result<String, AuthError>;
    fn verify(&self, password: &str, password_hash: &str) -> Result<bool, AuthError>;
    /// 判断 Hash 是否为需要升级的 Legacy 算法（如 bcrypt）。默认 `false`。
    fn needs_rehash(&self, password_hash: &str) -> bool {
        let _ = password_hash;
        false
    }
}

#[async_trait]
pub trait AuthRepository: Send + Sync {
    async fn is_bootstrapped(&self) -> Result<bool, AuthError>;
    async fn create_owner(&self, owner: NewOwner) -> Result<User, AuthError>;
    async fn find_credentials(&self, login: &str) -> Result<Option<CredentialUser>, AuthError>;
    async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, AuthError>;
    async fn create_session(&self, session: NewSession) -> Result<(), AuthError>;
    async fn find_session(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<AuthenticatedSession>, AuthError>;
    async fn touch_session(&self, session_id: Uuid) -> Result<(), AuthError>;
    async fn revoke_session(&self, session_id: Uuid) -> Result<(), AuthError>;
    async fn revoke_user_sessions(&self, user_id: i64) -> Result<(), AuthError>;
    /// 物理删除已过期或已撤销的 Session 行，防止 `admin_sessions` 无限增长；返回删除行数。
    async fn delete_expired_sessions(&self) -> Result<u64, AuthError>;
    async fn update_last_login(&self, user_id: i64) -> Result<(), AuthError>;
    async fn update_profile(&self, user_id: i64, profile: ProfileUpdate)
    -> Result<User, AuthError>;
    async fn update_password(&self, user_id: i64, password_hash: &str) -> Result<(), AuthError>;
    async fn create_password_reset(&self, reset: NewPasswordReset) -> Result<(), AuthError>;
    async fn consume_password_reset(
        &self,
        token_hash: &[u8],
        password_hash: &str,
    ) -> Result<Option<i64>, AuthError>;
    async fn write_audit(&self, event: AuditEvent) -> Result<(), AuthError>;
    /// 分页查询 Audit Log（Admin 端），按 `created_at` 降序。
    async fn list_audit(&self, query: AuditListQuery) -> Result<AuditPage, AuthError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_policy_requires_length_letters_and_numbers() {
        assert_eq!(
            validate_password("short1"),
            Err(AuthError::InvalidPasswordLength)
        );
        assert_eq!(
            validate_password("onlyletterslong"),
            Err(AuthError::WeakPassword)
        );
        assert!(validate_password("reliable-pass-2026").is_ok());
    }

    #[test]
    fn role_permissions_follow_the_fixed_matrix() {
        assert!(Role::Owner.allows(Permission::ManageUsers));
        assert!(Role::Editor.allows(Permission::ManageContent));
        assert!(!Role::Editor.allows(Permission::ManageUsers));
        assert!(Role::Moderator.allows(Permission::ModerateComments));
        assert!(!Role::Moderator.allows(Permission::ManageContent));
    }

    #[test]
    fn session_expiry_uses_an_exclusive_upper_bound() {
        let now = OffsetDateTime::now_utc();
        let session = AuthenticatedSession {
            id: Uuid::now_v7(),
            user: User {
                id: 1,
                username: "owner".to_owned(),
                email: "owner@example.com".to_owned(),
                display_name: "Owner".to_owned(),
                avatar_url: None,
                role: Role::Owner,
                status: UserStatus::Active,
            },
            expires_at: now,
        };

        assert!(session.is_expired(now));
    }
}
