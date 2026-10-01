//! 评论域：状态机、Repository Contract 与隐私保护。
//! 访客提交默认 pending，管理员通过 Admin API 审核；Email 和 IP 只存 Hash/Digest。

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

// ============================================================
// 评论目标类型
// ============================================================

/// 评论目标类型，与数据库 Check Constraint 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentTargetType {
    Article,
    Page,
    Link,
}

impl CommentTargetType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Article => "article",
            Self::Page => "page",
            Self::Link => "link",
        }
    }
}

impl fmt::Display for CommentTargetType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CommentTargetType {
    type Err = CommentError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "article" => Ok(Self::Article),
            "page" => Ok(Self::Page),
            "link" => Ok(Self::Link),
            _ => Err(CommentError::InvalidTargetType),
        }
    }
}

// ============================================================
// 评论状态
// ============================================================

/// 评论生命周期状态，状态转换由 `can_transition_to` 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentStatus {
    Pending,
    Approved,
    Rejected,
    Spam,
    Recycled,
}

impl CommentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Spam => "spam",
            Self::Recycled => "recycled",
        }
    }

    /// 状态转换规则：
    /// - pending → approved/rejected/spam
    /// - approved/rejected/spam → recycled
    /// - recycled → approved（恢复）
    pub const fn can_transition_to(self, target: Self) -> bool {
        matches!(
            (self, target),
            (Self::Pending, Self::Approved | Self::Rejected | Self::Spam)
                | (Self::Approved | Self::Rejected | Self::Spam, Self::Recycled)
                | (Self::Recycled, Self::Approved)
        )
    }
}

impl fmt::Display for CommentStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CommentStatus {
    type Err = CommentError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "spam" => Ok(Self::Spam),
            "recycled" => Ok(Self::Recycled),
            _ => Err(CommentError::InvalidStatus),
        }
    }
}

// ============================================================
// AI 审核结论
// ============================================================

/// AI 风险结论，与 `comments.ai_risk` 的 CHECK 一一对应。
/// AI 只能标记风险，最终状态由管理员决定（Phase 08 §3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiRisk {
    Safe,
    Suspicious,
    Spam,
}

impl AiRisk {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Suspicious => "suspicious",
            Self::Spam => "spam",
        }
    }
}

impl fmt::Display for AiRisk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AiRisk {
    type Err = CommentError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "safe" => Ok(Self::Safe),
            "suspicious" => Ok(Self::Suspicious),
            "spam" => Ok(Self::Spam),
            _ => Err(CommentError::Validation),
        }
    }
}

/// AI 审核结论入参：risk = spam 时由 Repository 一并把状态置为 spam
/// （高置信垃圾标记是本批唯一允许的自动处置，见 Phase 08 实施偏差说明）。
#[derive(Debug, Clone)]
pub struct AiAssessment {
    pub comment_id: i64,
    pub risk: AiRisk,
    pub reason: Option<String>,
    pub confidence: Option<f32>,
}

// ============================================================
// Domain Model
// ============================================================

/// 评论实体：包含内容、状态、层级关系和隐私字段。
#[derive(Debug, Clone)]
pub struct Comment {
    pub id: i64,
    pub target_type: CommentTargetType,
    pub target_id: i64,
    pub root_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub author_name: String,
    pub author_email: String,
    pub author_url: Option<String>,
    pub content_markdown: String,
    pub content_html: String,
    pub status: CommentStatus,
    pub moderator_id: Option<i64>,
    pub moderated_at: Option<OffsetDateTime>,
    pub moderation_reason: Option<String>,
    pub ip_hash: String,
    pub user_agent_digest: String,
    pub is_admin_reply: bool,
    /// AI 审核结论（可空表示未经过 AI 审核）。
    pub ai_risk: Option<AiRisk>,
    pub ai_reason: Option<String>,
    pub ai_confidence: Option<f32>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// 新评论提交请求（访客端），不接受可信字段。
/// `content_html` 由 HTTP 层渲染并 Sanitize 后传入，不信任客户端 HTML；
/// `initial_status` 由 HTTP 层按站点评论策略决定（pending / approved）。
#[derive(Debug, Clone)]
pub struct NewComment {
    pub target_type: CommentTargetType,
    pub target_id: i64,
    pub parent_id: Option<i64>,
    pub author_name: String,
    pub author_email: String,
    pub author_url: Option<String>,
    pub content_markdown: String,
    pub content_html: String,
    pub initial_status: CommentStatus,
    pub ip_hash: String,
    pub user_agent_digest: String,
}

/// 管理员回复请求。`content_html` 由 HTTP 层渲染并 Sanitize 后传入，
/// 保证 Admin 回复与访客评论共用同一套渲染规则。
#[derive(Debug, Clone)]
pub struct AdminReply {
    pub target_type: CommentTargetType,
    pub target_id: i64,
    pub parent_id: Option<i64>,
    /// 展示用署名（通常为操作管理员的 Display Name），不存 Email。
    pub author_name: String,
    pub content_markdown: String,
    pub content_html: String,
    pub moderator_id: i64,
}

/// 评论状态变更请求。
#[derive(Debug, Clone)]
pub struct CommentStatusChange {
    pub comment_id: i64,
    pub new_status: CommentStatus,
    pub moderator_id: i64,
    pub reason: Option<String>,
}

/// 评论列表查询参数。
#[derive(Debug, Clone, Default)]
pub struct CommentListQuery {
    pub page: u32,
    pub page_size: u32,
    pub target_type: Option<CommentTargetType>,
    pub target_id: Option<i64>,
    pub status: Option<CommentStatus>,
    pub keyword: Option<String>,
}

/// 分页评论列表。
#[derive(Debug, Clone)]
pub struct CommentPage {
    pub items: Vec<Comment>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// Dashboard 聚合数据。
#[derive(Debug, Clone)]
pub struct DashboardStats {
    pub article_total: i64,
    pub article_draft: i64,
    pub article_published: i64,
    pub article_recycled: i64,
    pub comment_total: i64,
    pub comment_pending: i64,
    pub comment_today: i64,
    pub recent_pending_comments: Vec<Comment>,
    pub recent_failed_jobs: Vec<crate::jobs::BackgroundJob>,
}

// ============================================================
// Error
// ============================================================

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CommentError {
    #[error("invalid comment target type")]
    InvalidTargetType,
    #[error("invalid comment parameter")]
    Validation,
    #[error("invalid comment status")]
    InvalidStatus,
    #[error("invalid status transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },
    #[error("comment not found")]
    NotFound,
    #[error("comment already moderated")]
    AlreadyModerated,
    #[error("target not found or not commentable")]
    TargetNotFound,
    #[error("content too short or too long")]
    ContentLength,
    /// 访客昵称：必填，1–60 字符（与数据库 CHECK 一致）。
    #[error("author name must contain 1 to 60 characters")]
    InvalidAuthorName,
    /// 访客 Email：必填且格式合法，最长 254 字符。
    #[error("author email is invalid")]
    InvalidAuthorEmail,
    /// 访客站点链接：可选，仅允许 http/https。
    #[error("author url must use http or https scheme")]
    InvalidAuthorUrl,
    /// 站点评论策略为 closed 时拒绝新评论。
    #[error("comments are closed")]
    Closed,
    #[error("duplicate submission detected")]
    Duplicate,
    #[error("rate limit exceeded")]
    RateLimited,
    #[error("comment store unavailable")]
    StoreUnavailable,
}

// ============================================================
// 访客提交校验
// ============================================================

/// 访客昵称校验：必填，1–60 字符。
pub fn validate_author_name(value: &str) -> Result<String, CommentError> {
    let value = value.trim();
    if !(1..=60).contains(&value.chars().count()) {
        return Err(CommentError::InvalidAuthorName);
    }
    Ok(value.to_owned())
}

/// 访客 Email 校验：必填、限长、最简格式（本地段 + @ + 域名含点）。
/// 完整 RFC 5322 校验无收益，投递结果以实际发送为准。
pub fn validate_author_email(value: &str) -> Result<String, CommentError> {
    let value = value.trim();
    let valid = value.chars().count() <= 254
        && value.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        });
    if !valid {
        return Err(CommentError::InvalidAuthorEmail);
    }
    Ok(value.to_owned())
}

/// 访客站点链接校验：可选，仅允许 http/https，最长 2048。
pub fn validate_author_url(value: Option<&str>) -> Result<Option<String>, CommentError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let lower = value.to_ascii_lowercase();
    if value.chars().count() > 2048
        || !(lower.starts_with("https://") || lower.starts_with("http://"))
    {
        return Err(CommentError::InvalidAuthorUrl);
    }
    Ok(Some(value.to_owned()))
}

/// 评论内容长度校验：1–2000 字符（与数据库 CHECK 一致）。
pub fn validate_content_length(value: &str) -> Result<(), CommentError> {
    if !(1..=2000).contains(&value.trim().chars().count()) {
        return Err(CommentError::ContentLength);
    }
    Ok(())
}

// ============================================================
// Repository Contract
// ============================================================

/// 评论 Repository 接口，由 infra 层实现。
#[async_trait]
pub trait CommentRepository: Send + Sync {
    /// 创建新评论（访客提交），返回创建后的完整实体。
    async fn create(&self, new_comment: NewComment) -> Result<Comment, CommentError>;

    /// 创建管理员回复，自动设为 approved 状态。
    async fn create_admin_reply(&self, reply: AdminReply) -> Result<Comment, CommentError>;

    /// 查找单个评论。
    async fn find(&self, comment_id: i64) -> Result<Option<Comment>, CommentError>;

    /// 分页列表查询（Admin 端）。
    async fn list(&self, query: CommentListQuery) -> Result<CommentPage, CommentError>;

    /// 获取指定 Target 的已批准评论列表（Public 端）。
    async fn list_approved_by_target(
        &self,
        target_type: CommentTargetType,
        target_id: i64,
    ) -> Result<Vec<Comment>, CommentError>;

    /// 变更评论状态。
    async fn change_status(&self, change: CommentStatusChange) -> Result<Comment, CommentError>;

    /// 物理删除评论（仅 recycled 状态允许）。
    async fn delete(&self, comment_id: i64) -> Result<(), CommentError>;

    /// 统计指定 Target 的已批准评论数。
    async fn count_approved(
        &self,
        target_type: CommentTargetType,
        target_id: i64,
    ) -> Result<i64, CommentError>;

    /// 写入 AI 审核结论；risk = spam 时同时把状态置为 spam。
    async fn apply_ai_assessment(&self, assessment: AiAssessment) -> Result<Comment, CommentError>;

    /// Dashboard 聚合统计。
    async fn dashboard_stats(&self) -> Result<DashboardStats, CommentError>;
}

/// 站点设置 Repository 扩展：评论策略查询。
#[async_trait]
pub trait CommentPolicyRepository: Send + Sync {
    /// 获取当前站点评论策略：closed / moderated / auto_approve。
    async fn comment_policy(&self) -> Result<String, CommentError>;
}

#[cfg(test)]
mod tests {
    use super::{
        AiRisk, CommentError, CommentStatus, CommentTargetType, validate_author_email,
        validate_author_name, validate_author_url, validate_content_length,
    };

    #[test]
    fn comment_target_type_round_trips_through_str() {
        for target in [
            CommentTargetType::Article,
            CommentTargetType::Page,
            CommentTargetType::Link,
        ] {
            assert_eq!(target.as_str().parse::<CommentTargetType>(), Ok(target));
            assert_eq!(target.to_string(), target.as_str());
        }
        // 未知目标类型拒绝，与数据库 CHECK 一一对应。
        assert_eq!(
            "journal".parse::<CommentTargetType>(),
            Err(CommentError::InvalidTargetType)
        );
    }

    #[test]
    fn comment_status_round_trips_through_str() {
        for status in [
            CommentStatus::Pending,
            CommentStatus::Approved,
            CommentStatus::Rejected,
            CommentStatus::Spam,
            CommentStatus::Recycled,
        ] {
            assert_eq!(status.as_str().parse::<CommentStatus>(), Ok(status));
            assert_eq!(status.to_string(), status.as_str());
        }
        assert_eq!(
            "archived".parse::<CommentStatus>(),
            Err(CommentError::InvalidStatus)
        );
    }

    #[test]
    fn ai_risk_parse_maps_invalid_values_to_validation_error() {
        for risk in [AiRisk::Safe, AiRisk::Suspicious, AiRisk::Spam] {
            assert_eq!(risk.as_str().parse::<AiRisk>(), Ok(risk));
            assert_eq!(risk.to_string(), risk.as_str());
        }
        // 非法 AI 结论归入 Validation 而非 InvalidStatus（见 FromStr 实现）。
        assert_eq!("toxic".parse::<AiRisk>(), Err(CommentError::Validation));
    }

    #[test]
    fn comment_status_machine_rejects_self_and_cross_verdict_transitions() {
        // 已审核结论之间不允许互转（含原地迁移）。
        for from in [
            CommentStatus::Approved,
            CommentStatus::Rejected,
            CommentStatus::Spam,
        ] {
            for to in [
                CommentStatus::Approved,
                CommentStatus::Rejected,
                CommentStatus::Spam,
            ] {
                assert!(!from.can_transition_to(to));
            }
        }
        // 任何状态都不允许原地迁移。
        for status in [
            CommentStatus::Pending,
            CommentStatus::Approved,
            CommentStatus::Rejected,
            CommentStatus::Spam,
            CommentStatus::Recycled,
        ] {
            assert!(!status.can_transition_to(status));
        }
        // 回收站只能恢复为 approved，不能回到 pending 重新排队。
        assert!(!CommentStatus::Recycled.can_transition_to(CommentStatus::Pending));
    }

    #[test]
    fn comment_status_machine_allows_expected_transitions() {
        // pending 可进入三种审核结论。
        assert!(CommentStatus::Pending.can_transition_to(CommentStatus::Approved));
        assert!(CommentStatus::Pending.can_transition_to(CommentStatus::Rejected));
        assert!(CommentStatus::Pending.can_transition_to(CommentStatus::Spam));
        // pending 不能直接回收：必须先有审核结论，再进入回收站。
        assert!(!CommentStatus::Pending.can_transition_to(CommentStatus::Recycled));

        for from in [
            CommentStatus::Approved,
            CommentStatus::Rejected,
            CommentStatus::Spam,
        ] {
            assert!(from.can_transition_to(CommentStatus::Recycled));
            // 已审核结论之间不允许互转，也不允许回到 pending。
            assert!(!from.can_transition_to(CommentStatus::Pending));
        }

        // 回收后仅允许恢复为 approved。
        assert!(CommentStatus::Recycled.can_transition_to(CommentStatus::Approved));
        assert!(!CommentStatus::Recycled.can_transition_to(CommentStatus::Rejected));
        assert!(!CommentStatus::Recycled.can_transition_to(CommentStatus::Spam));
    }

    #[test]
    fn visitor_fields_are_validated() {
        assert!(validate_author_name("读者").is_ok());
        assert_eq!(
            validate_author_name("  "),
            Err(CommentError::InvalidAuthorName)
        );

        assert!(validate_author_email("reader@example.com").is_ok());
        assert_eq!(
            validate_author_email("not-an-email"),
            Err(CommentError::InvalidAuthorEmail)
        );
        assert_eq!(
            validate_author_email("a@b"),
            Err(CommentError::InvalidAuthorEmail)
        );

        assert_eq!(validate_author_url(None), Ok(None));
        assert_eq!(validate_author_url(Some("  ")), Ok(None));
        assert!(validate_author_url(Some("https://blog.example.com")).is_ok());
        assert_eq!(
            validate_author_url(Some("javascript:alert(1)")),
            Err(CommentError::InvalidAuthorUrl)
        );

        assert!(validate_content_length("你好").is_ok());
        assert_eq!(
            validate_content_length(" "),
            Err(CommentError::ContentLength)
        );
    }

    #[test]
    fn visitor_fields_enforce_length_and_format_boundaries() {
        // 昵称：首尾空白裁剪后返回；裁剪后 1–60 字符。
        assert_eq!(validate_author_name("  读者甲  "), Ok("读者甲".to_owned()));
        assert!(validate_author_name(&"名".repeat(60)).is_ok());
        assert_eq!(
            validate_author_name(&"名".repeat(61)),
            Err(CommentError::InvalidAuthorName)
        );

        // Email：本地段不能为空，域名必须含点且不能以点开头/结尾，总长 ≤ 254。
        assert_eq!(
            validate_author_email("@example.com"),
            Err(CommentError::InvalidAuthorEmail)
        );
        assert_eq!(
            validate_author_email("a@.com"),
            Err(CommentError::InvalidAuthorEmail)
        );
        assert_eq!(
            validate_author_email("a@com."),
            Err(CommentError::InvalidAuthorEmail)
        );
        assert!(validate_author_email("a@b.co").is_ok());
        let long_email = format!("{}@example.com", "a".repeat(300));
        assert_eq!(
            validate_author_email(&long_email),
            Err(CommentError::InvalidAuthorEmail)
        );

        // URL：仅 http/https（大小写不敏感），返回裁剪后保留原大小写的值。
        assert_eq!(
            validate_author_url(Some(" HTTP://Example.COM ")),
            Ok(Some("HTTP://Example.COM".to_owned()))
        );
        assert_eq!(
            validate_author_url(Some("ftp://example.com")),
            Err(CommentError::InvalidAuthorUrl)
        );
        let long_url = format!("https://{}", "a".repeat(2048));
        assert_eq!(
            validate_author_url(Some(&long_url)),
            Err(CommentError::InvalidAuthorUrl)
        );

        // 内容长度：1–2000 字符，按裁剪后的字符数计算。
        assert!(validate_content_length(&"字".repeat(2000)).is_ok());
        assert_eq!(
            validate_content_length(&"字".repeat(2001)),
            Err(CommentError::ContentLength)
        );
    }
}
