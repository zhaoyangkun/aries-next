use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

/// 管理员角色，决定固定权限矩阵（见 `Role::allows`）。
///
/// 序列化为 snake_case 字符串（`owner`/`editor`/`moderator`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Editor,
    Moderator,
}

impl Role {
    /// 返回角色的小写字符串表示，与数据库 `role` 列取值一致。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Editor => "editor",
            Self::Moderator => "moderator",
        }
    }

    /// 判断该角色是否拥有指定 Permission。权限以这里的矩阵为准，
    /// 前端 Guard 只做入口隐藏。
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

    /// 解析数据库/请求中的角色字符串；未知取值返回 `AuthError::InvalidRole`。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "editor" => Ok(Self::Editor),
            "moderator" => Ok(Self::Moderator),
            _ => Err(AuthError::InvalidRole),
        }
    }
}

/// 功能级权限点，`Role::allows` 在此枚举上求值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ViewDashboard,
    ManageContent,
    ModerateComments,
    ManageUsers,
    ManageSettings,
    ManageProfile,
}

/// 用户账号状态；`Disabled` 的账号不能登录，其 Session 会被全部撤销。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Disabled,
}

impl UserStatus {
    /// 返回状态的小写字符串表示，与数据库 `status` 列取值一致。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }
}

impl FromStr for UserStatus {
    type Err = AuthError;

    /// 解析状态字符串；未知取值返回 `AuthError::InvalidUserStatus`。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            _ => Err(AuthError::InvalidUserStatus),
        }
    }
}

/// 管理员用户领域对象；不含 Password Hash（登录校验专用视图见 `CredentialUser`）。
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

/// 登录校验专用视图：在 `User` 基础上附带 Password Hash，仅供凭据比对，
/// 不得进入任何响应 DTO。
#[derive(Debug, Clone)]
pub struct CredentialUser {
    pub user: User,
    pub password_hash: String,
}

/// 首次 Bootstrap 时创建 Owner 的输入；`password_hash` 必须是 Argon2id 产物。
#[derive(Debug, Clone)]
pub struct NewOwner {
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
}

/// 资料更新输入；`avatar_url` 为 `None` 表示清除头像。
#[derive(Debug, Clone)]
pub struct ProfileUpdate {
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

/// 新 Session 输入；`token_hash` 是浏览器 Token 的 SHA-256，原始 Token 不落库。
#[derive(Debug, Clone)]
pub struct NewSession {
    pub id: Uuid,
    pub token_hash: Vec<u8>,
    pub user_id: i64,
    pub expires_at: OffsetDateTime,
    pub user_agent: Option<String>,
}

/// 通过校验的 Session 视图：关联完整 `User`，供请求上下文直接取用身份。
#[derive(Debug, Clone)]
pub struct AuthenticatedSession {
    pub id: Uuid,
    pub user: User,
    pub expires_at: OffsetDateTime,
}

impl AuthenticatedSession {
    /// 判断 Session 在给定时刻是否已过期；有效期右开，`expires_at == now` 视为过期。
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        self.expires_at <= now
    }
}

/// 密码重置申请输入；`token_hash` 是重置 Token 的 SHA-256，
/// 原始 Token 只出现在发送的邮件链接里。
#[derive(Debug, Clone)]
pub struct NewPasswordReset {
    pub id: Uuid,
    pub user_id: i64,
    pub token_hash: Vec<u8>,
    pub expires_at: OffsetDateTime,
}

/// 一条待写入的审计事件；`actor_user_id` 为 `None` 表示匿名行为
/// （如未认证的登录尝试）。
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

/// Authentication 领域错误；`thiserror` Display 文案为英文，
/// 面向用户的提示由 API 层按错误码统一转换。
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

/// 校验用户名：3–30 个字符，仅允许 ASCII 字母、数字、`_` 和 `-`。
///
/// # Errors
/// 长度或字符集不合法时返回 `AuthError::InvalidUsername`。
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

/// 校验密码强度：10–128 个字符，且必须同时包含字母和数字。
///
/// # Errors
/// 长度不符返回 `InvalidPasswordLength`，缺字母或数字返回 `WeakPassword`。
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

/// Password Hash 抽象；生产实现为 Argon2id（`Argon2PasswordHasher`）。
/// 迁移来的 Legacy bcrypt Hash 由 `needs_rehash` 驱动首次登录后的透明升级。
pub trait PasswordHasher: Send + Sync {
    /// 计算明文密码的 Hash。
    fn hash(&self, password: &str) -> Result<String, AuthError>;

    /// 校验明文密码与存储的 Hash 是否匹配。
    fn verify(&self, password: &str, password_hash: &str) -> Result<bool, AuthError>;

    /// 判断 Hash 是否为需要升级的 Legacy 算法（如 bcrypt）。默认 `false`。
    fn needs_rehash(&self, password_hash: &str) -> bool {
        let _ = password_hash;
        false
    }
}

/// Authentication 与账号存储契约；实现位于 `aries-infra`（`PostgresAuthRepository`）。
///
/// Session 相关方法只接收/返回 Token 的 SHA-256 Hash，原始 Token 不允许进入本层。
#[async_trait]
pub trait AuthRepository: Send + Sync {
    /// 系统是否已完成首次 Bootstrap（存在至少一个 Owner）。
    async fn is_bootstrapped(&self) -> Result<bool, AuthError>;
    /// 创建首个 Owner；仅在未 Bootstrap 时允许调用。
    async fn create_owner(&self, owner: NewOwner) -> Result<User, AuthError>;
    /// 按用户名或 Email 查找登录凭据；未找到返回 `Ok(None)`。
    async fn find_credentials(&self, login: &str) -> Result<Option<CredentialUser>, AuthError>;
    /// 按 Email 精确查找用户。
    async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, AuthError>;
    /// 全部 Active 管理员的 Email（评论新评论通知的收件人列表）；不含已禁用账号。
    async fn list_active_admin_emails(&self) -> Result<Vec<String>, AuthError>;
    /// 签发新 Session；Login 流程先撤销旧 Session 再调用本方法（防 Session Fixation）。
    async fn create_session(&self, session: NewSession) -> Result<(), AuthError>;
    /// 按 Token Hash 查找未过期 Session；命中后由 HTTP 层负责滑动续期。
    async fn find_session(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<AuthenticatedSession>, AuthError>;
    /// 滑动延长 Session 过期时间（活跃用户的续期点）。
    async fn touch_session(&self, session_id: Uuid) -> Result<(), AuthError>;
    /// 吊销单个 Session（登出）。
    async fn revoke_session(&self, session_id: Uuid) -> Result<(), AuthError>;
    /// 吊销用户的全部 Session；改密、重置密码、禁用账号时调用。
    async fn revoke_user_sessions(&self, user_id: i64) -> Result<(), AuthError>;
    /// 物理删除已过期或已撤销的 Session 行，防止 `admin_sessions` 无限增长；返回删除行数。
    async fn delete_expired_sessions(&self) -> Result<u64, AuthError>;
    /// 记录登录成功时间。
    async fn update_last_login(&self, user_id: i64) -> Result<(), AuthError>;
    /// 更新用户资料，返回更新后的完整 `User`。
    async fn update_profile(&self, user_id: i64, profile: ProfileUpdate)
    -> Result<User, AuthError>;
    /// 仅更新 Password Hash；调用方需先撤销该用户全部 Session。
    async fn update_password(&self, user_id: i64, password_hash: &str) -> Result<(), AuthError>;
    /// 创建密码重置申请。
    async fn create_password_reset(&self, reset: NewPasswordReset) -> Result<(), AuthError>;
    /// 按 Token Hash 消费一次重置申请：命中则重置密码并返回用户 ID，否则返回 `Ok(None)`。
    async fn consume_password_reset(
        &self,
        token_hash: &[u8],
        password_hash: &str,
    ) -> Result<Option<i64>, AuthError>;
    /// 写入一条审计事件。
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
