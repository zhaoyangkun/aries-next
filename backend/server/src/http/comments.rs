//! Admin 评论管理：列表、详情、审核（状态变更）、管理员回复、删除。
//! 隐私边界：Admin 需要看到访客 Email 辅助审核判断；Public API 永不返回该字段。

use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    comments::{
        AdminReply, Comment, CommentListQuery, CommentStatus, CommentStatusChange,
        CommentTargetType,
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::state::AppState;

use super::{articles::render_markdown, auth::CurrentUser, error::ApiError};

const MIN_COMMENT_LENGTH: usize = 1;
const MAX_COMMENT_LENGTH: usize = 2_000;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/comments", get(list_comments))
        .route(
            "/comments/{id}",
            axum::routing::get(get_comment).delete(delete_comment),
        )
        .route(
            "/comments/{id}/status",
            axum::routing::patch(change_comment_status),
        )
        .route(
            "/comments/{id}/reply",
            axum::routing::post(reply_to_comment),
        )
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
struct CommentListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 按状态筛选：`pending` / `approved` / `rejected` / `spam` / `recycled`。
    status: Option<String>,
    /// 按目标类型筛选：`article` / `page` / `link`。
    target_type: Option<String>,
    /// 按目标 ID 筛选。
    target_id: Option<i64>,
    /// 对内容 Markdown 与评论者名称做模糊匹配。
    keyword: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct CommentPageResponse {
    items: Vec<CommentResponse>,
    /// 当前筛选条件下的评论总数。
    total: i64,
    page: u32,
    page_size: u32,
}

/// 评论 DTO：不返回 `ip_hash`/`user_agent_digest` 等内部字段。
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(super) struct CommentResponse {
    pub(super) id: i64,
    /// 评论目标类型：`article` / `page` / `link`。
    target_type: String,
    target_id: i64,
    /// 所属根评论 ID（根评论为自身 ID）。
    root_id: Option<i64>,
    /// 直接父评论 ID；根评论为 null。
    parent_id: Option<i64>,
    author_name: String,
    /// 仅 Admin API 返回，Public API 永不暴露。
    author_email: String,
    author_url: Option<String>,
    content_markdown: String,
    /// 服务端 Comrak 渲染并 Sanitize 后的 HTML，可安全直出。
    content_html: String,
    /// `pending` / `approved` / `rejected` / `spam` / `recycled`。
    status: String,
    is_admin_reply: bool,
    /// 审核备注（状态变更时可选填写）。
    moderation_reason: Option<String>,
    moderated_at: Option<OffsetDateTime>,
    /// AI 审核结论（未审核为 null），供管理端辅助判断。
    ai_risk: Option<String>,
    /// AI 给出的简短理由。
    ai_reason: Option<String>,
    /// AI 置信度（0–1）。
    ai_confidence: Option<f32>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<Comment> for CommentResponse {
    fn from(comment: Comment) -> Self {
        Self {
            id: comment.id,
            target_type: comment.target_type.as_str().to_owned(),
            target_id: comment.target_id,
            root_id: comment.root_id,
            parent_id: comment.parent_id,
            author_name: comment.author_name,
            author_email: comment.author_email,
            author_url: comment.author_url,
            content_markdown: comment.content_markdown,
            content_html: comment.content_html,
            status: comment.status.as_str().to_owned(),
            is_admin_reply: comment.is_admin_reply,
            moderation_reason: comment.moderation_reason,
            moderated_at: comment.moderated_at,
            ai_risk: comment.ai_risk.map(|risk| risk.as_str().to_owned()),
            ai_reason: comment.ai_reason,
            ai_confidence: comment.ai_confidence,
            created_at: comment.created_at,
            updated_at: comment.updated_at,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/admin/comments",
    tag = "Admin Comments",
    operation_id = "listComments",
    summary = "Admin 评论列表",
    description = "返回访客 Email 辅助审核判断；Public API 永不暴露该字段。支持按状态、目标类型/ID、关键词筛选。",
    security(("cookieAuth" = [])),
    params(CommentListParams),
    responses(
        (status = 200, description = "具备稳定 Total 语义的评论分页", body = CommentPageResponse),
        (status = 400, description = "status / target_type 参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无评论审核权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_comments(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<CommentListParams>,
) -> Result<Json<CommentPageResponse>, ApiError> {
    current.require(Permission::ModerateComments)?;
    let status = params
        .status
        .as_deref()
        .map(CommentStatus::from_str)
        .transpose()?;
    let target_type = params
        .target_type
        .as_deref()
        .map(CommentTargetType::from_str)
        .transpose()?;
    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let result = state
        .comments
        .list(CommentListQuery {
            page,
            page_size,
            target_type,
            target_id: params.target_id,
            status,
            keyword: params
                .keyword
                .clone()
                .filter(|value| !value.trim().is_empty()),
        })
        .await?;
    Ok(Json(CommentPageResponse {
        items: result.items.into_iter().map(Into::into).collect(),
        total: result.total,
        page: result.page,
        page_size: result.page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/api/admin/comments/{id}",
    tag = "Admin Comments",
    operation_id = "getComment",
    summary = "评论详情",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "评论 ID")),
    responses(
        (status = 200, description = "评论详情", body = CommentResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无评论审核权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "评论不存在", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_comment(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(comment_id): Path<i64>,
) -> Result<Json<CommentResponse>, ApiError> {
    current.require(Permission::ModerateComments)?;
    let comment = state
        .comments
        .find(comment_id)
        .await?
        .ok_or(aries_core::comments::CommentError::NotFound)?;
    Ok(Json(comment.into()))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct ChangeCommentStatusRequest {
    /// 目标状态：`pending` / `approved` / `rejected` / `spam` / `recycled`。
    status: String,
    /// 审核备注（可选，trim 后非空才持久化）。
    reason: Option<String>,
}

#[utoipa::path(
    patch,
    path = "/api/admin/comments/{id}/status",
    tag = "Admin Comments",
    operation_id = "changeCommentStatus",
    summary = "评论审核（状态变更）",
    description = "状态转换遵循 core 状态机：pending → approved/rejected/spam；approved/rejected/spam → recycled；recycled → approved（恢复并公开）。非法转换返回 409 `INVALID_COMMENT_TRANSITION`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "评论 ID")),
    request_body(content = ChangeCommentStatusRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "状态变更成功", body = CommentResponse),
        (status = 400, description = "status 参数非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无评论审核权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "评论不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "非法状态转换（INVALID_COMMENT_TRANSITION）", body = crate::openapi::ErrorResponse),
    )
)]
async fn change_comment_status(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(comment_id): Path<i64>,
    Json(request): Json<ChangeCommentStatusRequest>,
) -> Result<Json<CommentResponse>, ApiError> {
    current.require(Permission::ModerateComments)?;
    let target = CommentStatus::from_str(&request.status)?;
    let existing = state
        .comments
        .find(comment_id)
        .await?
        .ok_or(aries_core::comments::CommentError::NotFound)?;
    // 状态机规则在 core（can_transition_to），HTTP 层负责把违规转换成 409。
    if !existing.status.can_transition_to(target) {
        return Err(aries_core::comments::CommentError::InvalidTransition {
            from: existing.status.as_str().to_owned(),
            to: target.as_str().to_owned(),
        }
        .into());
    }
    let reason = request.reason.filter(|value| !value.trim().is_empty());
    let comment = state
        .comments
        .change_status(CommentStatusChange {
            comment_id,
            new_status: target,
            moderator_id: current.user.id,
            reason,
        })
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "comment.moderated".to_owned(),
            target_type: "comment".to_owned(),
            target_id: Some(comment.id.to_string()),
            metadata: serde_json::json!({
                "status": comment.status.as_str(),
                "reason": comment.moderation_reason,
            }),
        })
        .await?;
    Ok(Json(comment.into()))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct ReplyCommentRequest {
    /// Markdown 源文本，长度 1–2000 字符。
    content_markdown: String,
}

#[utoipa::path(
    post,
    path = "/api/admin/comments/{id}/reply",
    tag = "Admin Comments",
    operation_id = "replyToComment",
    summary = "管理员回复评论",
    description = "Markdown 渲染并 Sanitize 后返回，状态直接为 `approved` 并公开，署名取操作管理员的展示名。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "被回复的评论 ID")),
    request_body(content = ReplyCommentRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "回复已创建并公开", body = CommentResponse),
        (status = 400, description = "内容长度非法（INVALID_COMMENT_CONTENT）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无评论审核权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "被回复的评论不存在", body = crate::openapi::ErrorResponse),
    )
)]
async fn reply_to_comment(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(comment_id): Path<i64>,
    Json(request): Json<ReplyCommentRequest>,
) -> Result<(StatusCode, Json<CommentResponse>), ApiError> {
    current.require(Permission::ModerateComments)?;
    let content = request.content_markdown.trim().to_owned();
    if !(MIN_COMMENT_LENGTH..=MAX_COMMENT_LENGTH).contains(&content.chars().count()) {
        return Err(aries_core::comments::CommentError::ContentLength.into());
    }
    let target = state
        .comments
        .find(comment_id)
        .await?
        .ok_or(aries_core::comments::CommentError::NotFound)?;
    // 与访客评论共用同一渲染管线：Markdown → Sanitized HTML。
    let rendered_html = render_markdown(&state, content.clone()).await?;
    let comment = state
        .comments
        .create_admin_reply(AdminReply {
            target_type: target.target_type,
            target_id: target.target_id,
            parent_id: Some(target.id),
            author_name: current.user.display_name,
            content_markdown: content,
            content_html: rendered_html,
            moderator_id: current.user.id,
        })
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "comment.reply_created".to_owned(),
            target_type: "comment".to_owned(),
            target_id: Some(comment.id.to_string()),
            metadata: serde_json::json!({ "parent_id": comment_id }),
        })
        .await?;
    Ok((StatusCode::CREATED, Json(comment.into())))
}

#[utoipa::path(
    delete,
    path = "/api/admin/comments/{id}",
    tag = "Admin Comments",
    operation_id = "deleteComment",
    summary = "物理删除评论",
    description = "仅 `recycled` 状态允许物理删除，其余状态返回 409 `COMMENT_NOT_RECYCLED`。删除不可恢复。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "评论 ID")),
    responses(
        (status = 204, description = "评论已物理删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无评论审核权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "评论不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "评论未进入 recycled 状态（COMMENT_NOT_RECYCLED）", body = crate::openapi::ErrorResponse),
    )
)]
async fn delete_comment(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(comment_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ModerateComments)?;
    let existing = state
        .comments
        .find(comment_id)
        .await?
        .ok_or(aries_core::comments::CommentError::NotFound)?;
    // 与文章删除一致：只有 recycled 状态允许物理删除，Repository 事务内二次校验。
    if existing.status != CommentStatus::Recycled {
        return Err(ApiError::conflict(
            "COMMENT_NOT_RECYCLED",
            "Comment must be recycled before deletion",
        ));
    }
    state.comments.delete(comment_id).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "comment.deleted".to_owned(),
            target_type: "comment".to_owned(),
            target_id: Some(comment_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
