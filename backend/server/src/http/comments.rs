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

#[derive(Debug, Deserialize)]
struct CommentListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    status: Option<String>,
    target_type: Option<String>,
    target_id: Option<i64>,
    keyword: Option<String>,
}

#[derive(Debug, Serialize)]
struct CommentPageResponse {
    items: Vec<CommentResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

/// 评论 DTO：不返回 `ip_hash`/`user_agent_digest` 等内部字段。
#[derive(Debug, Serialize)]
pub(super) struct CommentResponse {
    pub(super) id: i64,
    target_type: String,
    target_id: i64,
    root_id: Option<i64>,
    parent_id: Option<i64>,
    author_name: String,
    author_email: String,
    author_url: Option<String>,
    content_markdown: String,
    content_html: String,
    status: String,
    is_admin_reply: bool,
    moderation_reason: Option<String>,
    moderated_at: Option<OffsetDateTime>,
    /// AI 审核结论（未审核为 null），供管理端辅助判断。
    ai_risk: Option<String>,
    ai_reason: Option<String>,
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

#[derive(Debug, Deserialize)]
struct ChangeCommentStatusRequest {
    status: String,
    reason: Option<String>,
}

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

#[derive(Debug, Deserialize)]
struct ReplyCommentRequest {
    content_markdown: String,
}

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
