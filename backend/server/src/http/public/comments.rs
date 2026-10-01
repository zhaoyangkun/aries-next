//! Public 评论端点：访客发表/回复、已批准评论树。
//! 隐私边界：Email 只入库不出 API；头像用 Email SHA-256 拼 Cravatar URL；
//! IP/UA 只存 Hash。写端点 no-store，提交限流 + 重复提交防护。

use std::str::FromStr;
use std::time::Duration;

use aries_core::comments::{
    Comment, CommentError, CommentStatus, CommentTargetType, NewComment, validate_author_email,
    validate_author_name, validate_author_url, validate_content_length,
};
use aries_core::jobs::{JobKind, NewBackgroundJob};
use aries_core::media::CommentPolicy;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::http::articles::render_markdown;
use crate::http::error::ApiError;
use crate::state::AppState;

use super::{
    CACHE_AGGREGATE, CACHE_NO_STORE, client_fingerprint, json_with_cache, reject_out_of_range_page,
};

/// 提交限流：同一客户端 60 秒最多 3 条评论（含回复）。
const SUBMIT_RATE_LIMIT: usize = 3;
const SUBMIT_RATE_WINDOW: Duration = Duration::from_secs(60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/articles/{slug}/comments", get(list_article_comments))
        .route("/pages/{slug}/comments", get(list_page_comments))
        .route("/comments", post(create_comment))
        .route("/comments/{id}/replies", post(reply_comment))
}

// ============================================================
// DTO
// ============================================================

/// 公开评论 DTO：永不包含 email/ip/ua；管理员回复的头像取站点 Logo（无则 null）。
/// `website` 是访客自己提交的公开展示信息（对齐 Twikoo：昵称渲染成外链），
/// 入库前已经过 http/https 校验，可直接回传。
#[derive(Debug, Serialize)]
struct PublicCommentResponse {
    id: i64,
    parent_id: Option<i64>,
    nickname: String,
    avatar_url: Option<String>,
    website: Option<String>,
    content_html: String,
    is_admin: bool,
    created_at: OffsetDateTime,
    children: Vec<PublicCommentResponse>,
}

/// 提交成功的响应：附带当前 status，前端据此提示「待审核」。
#[derive(Debug, Serialize)]
struct PublicCommentCreatedResponse {
    #[serde(flatten)]
    comment: PublicCommentResponse,
    status: String,
}

/// 访客头像：Email 不出 API，用其 SHA-256 拼 Cravatar URL。
fn visitor_avatar(email: &str) -> String {
    format!(
        "https://cravatar.cn/avatar/{}?d=retro",
        hex_digest(email.trim().to_lowercase().as_bytes())
    )
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn to_public_comment(
    comment: &Comment,
    site_logo: &str,
    children: Vec<PublicCommentResponse>,
) -> PublicCommentResponse {
    let avatar_url = if comment.is_admin_reply {
        // 管理员回复用站点 Logo；未配置时为 null。
        if site_logo.is_empty() {
            None
        } else {
            Some(site_logo.to_owned())
        }
    } else {
        Some(visitor_avatar(&comment.author_email))
    };
    PublicCommentResponse {
        id: comment.id,
        parent_id: comment.parent_id,
        nickname: comment.author_name.clone(),
        avatar_url,
        // 管理员回复没有访客站点链接。
        website: if comment.is_admin_reply {
            None
        } else {
            comment.author_url.clone()
        },
        content_html: comment.content_html.clone(),
        is_admin: comment.is_admin_reply,
        created_at: comment.created_at,
        children,
    }
}

// ============================================================
// 已批准评论树
// ============================================================

#[derive(Debug, Deserialize)]
struct CommentListParams {
    #[serde(default = "crate::http::default_page")]
    page: u32,
    #[serde(default)]
    page_size: Option<u32>,
}

#[derive(Debug, Serialize)]
struct PublicCommentPageResponse {
    items: Vec<PublicCommentResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

async fn list_article_comments(
    state: State<AppState>,
    slug: Path<String>,
    params: Query<CommentListParams>,
) -> Result<Response, ApiError> {
    list_approved_comments(state, slug, params, CommentTargetType::Article).await
}

async fn list_page_comments(
    state: State<AppState>,
    slug: Path<String>,
    params: Query<CommentListParams>,
) -> Result<Response, ApiError> {
    list_approved_comments(state, slug, params, CommentTargetType::Page).await
}

/// 文章/页面共用的已批准评论树：按 target_type + slug 解析已发布目标，其余逻辑一致。
async fn list_approved_comments(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<CommentListParams>,
    target_type: CommentTargetType,
) -> Result<Response, ApiError> {
    let target_id = resolve_target(&state, target_type, &slug).await?;

    let settings = state.site_settings.get().await?;
    let page = params.page.max(1);
    // 默认分页大小取自站点设置（comments_per_page）。
    let page_size = params
        .page_size
        .unwrap_or(settings.comments_per_page.max(1) as u32)
        .clamp(1, 100);

    let all = state
        .comments
        .list_approved_by_target(target_type, target_id)
        .await?;

    // 分页只作用于根评论；回复整组挂在所属根下。根按创建时间升序（旧版语义）。
    let mut roots: Vec<&Comment> = all.iter().filter(|c| c.root_id == Some(c.id)).collect();
    roots.sort_by_key(|c| c.id);
    let total = roots.len() as i64;
    let start = usize::try_from((page - 1) * page_size).unwrap_or(usize::MAX);
    let page_roots: Vec<&Comment> = roots
        .into_iter()
        .skip(start)
        .take(usize::try_from(page_size).unwrap_or(0))
        .collect();

    reject_out_of_range_page(page, page_roots.len())?;

    let items = page_roots
        .into_iter()
        .map(|root| {
            let children = all
                .iter()
                .filter(|c| c.root_id == Some(root.id) && c.id != root.id)
                .map(|c| to_public_comment(c, &settings.logo_url, Vec::new()))
                .collect();
            to_public_comment(root, &settings.logo_url, children)
        })
        .collect();

    Ok(json_with_cache(
        &PublicCommentPageResponse {
            items,
            total,
            page,
            page_size,
        },
        CACHE_AGGREGATE,
    ))
}

// ============================================================
// 访客提交
// ============================================================

#[derive(Debug, Deserialize)]
struct CreateCommentRequest {
    target_type: String,
    target_slug: String,
    nickname: String,
    email: String,
    website: Option<String>,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ReplyCommentRequest {
    nickname: String,
    email: String,
    website: Option<String>,
    content: String,
}

/// 一次访客提交通过全部前置检查后的产物。
struct PreparedSubmission {
    initial_status: CommentStatus,
    author_name: String,
    author_email: String,
    author_url: Option<String>,
    content_markdown: String,
    content_html: String,
    ip_hash: String,
    user_agent_digest: String,
}

/// 按 comment_policy 决定初始状态；closed 直接拒绝。
fn initial_status(policy: CommentPolicy) -> Result<CommentStatus, CommentError> {
    match policy {
        CommentPolicy::Closed => Err(CommentError::Closed),
        CommentPolicy::Moderated => Ok(CommentStatus::Pending),
        CommentPolicy::AutoApprove => Ok(CommentStatus::Approved),
    }
}

/// 评论通知入队：Worker 当前只记录日志（Email Adapter 未接入），
/// 入队失败只记日志，绝不影响评论创建响应（静默降级）。
async fn enqueue_comment_notification(
    state: &AppState,
    comment: &Comment,
    recipient: Option<&str>,
) {
    let mut payload = serde_json::json!({
        "comment_id": comment.id,
        "target_type": comment.target_type.as_str(),
        "target_id": comment.target_id,
    });
    // 回复时通知被回复的作者；根评论的站点所有者通知待 Email Adapter 接入后补充。
    if let Some(recipient) = recipient.filter(|value| !value.is_empty()) {
        payload["recipient_email"] = serde_json::Value::String(recipient.to_owned());
    }
    if let Err(error) = state
        .jobs
        .enqueue(NewBackgroundJob::new(JobKind::CommentNotification, payload))
        .await
    {
        tracing::error!(
            error = %error,
            comment_id = comment.id,
            "failed to enqueue comment notification job"
        );
    }
}

/// 解析评论目标：只允许已发布文章 / 已发布页面（link 暂不对访客开放）。
async fn resolve_target(
    state: &AppState,
    target_type: CommentTargetType,
    slug: &str,
) -> Result<i64, ApiError> {
    let target_id = match target_type {
        CommentTargetType::Article => state
            .content
            .find_published_article_by_slug(slug)
            .await?
            .map(|article| article.id),
        CommentTargetType::Page => state
            .pages
            .find_published_by_slug(slug)
            .await?
            .map(|page| page.id),
        CommentTargetType::Link => None,
    };
    target_id.ok_or_else(|| CommentError::TargetNotFound.into())
}

/// 访客提交的公共流程：限流 → 策略 → 字段校验 → 渲染 → 隐私 Hash。
async fn prepare_submission(
    state: &AppState,
    headers: &HeaderMap,
    nickname: &str,
    email: &str,
    website: Option<&str>,
    content: &str,
) -> Result<PreparedSubmission, ApiError> {
    let fingerprint = client_fingerprint(headers);
    let rate_key = format!("comment-submit:{fingerprint}");
    let decision = state
        .rate_limiter
        .check(&rate_key, SUBMIT_RATE_LIMIT, SUBMIT_RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }

    let settings = state.site_settings.get().await?;
    let status = initial_status(settings.comment_policy)?;

    let author_name = validate_author_name(nickname)?;
    let author_email = validate_author_email(email)?;
    let author_url = validate_author_url(website)?;
    validate_content_length(content)?;
    let content_markdown = content.trim().to_owned();
    // 与管理员回复共用同一渲染管线：Markdown → Sanitized HTML，不信任客户端 HTML。
    let content_html = render_markdown(state, content_markdown.clone()).await?;

    // 隐私字段只存 Hash：fingerprint 形如 "ip|ua"，拆开分别摘要。
    let (ip, user_agent) = fingerprint
        .split_once('|')
        .unwrap_or((fingerprint.as_str(), ""));
    let ip_hash = hex_digest(ip.as_bytes());
    let user_agent_digest = hex_digest(user_agent.as_bytes());

    Ok(PreparedSubmission {
        initial_status: status,
        author_name,
        author_email,
        author_url,
        content_markdown,
        content_html,
        ip_hash,
        user_agent_digest,
    })
}

async fn create_comment(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<CreateCommentRequest>,
) -> Result<Response, ApiError> {
    let target_type = CommentTargetType::from_str(&request.target_type)?;
    let target_id = resolve_target(&state, target_type, &request.target_slug).await?;
    let prepared = prepare_submission(
        &state,
        &headers,
        &request.nickname,
        &request.email,
        request.website.as_deref(),
        &request.content,
    )
    .await?;

    let comment = state
        .comments
        .create(NewComment {
            target_type,
            target_id,
            parent_id: None,
            author_name: prepared.author_name,
            author_email: prepared.author_email,
            author_url: prepared.author_url,
            content_markdown: prepared.content_markdown,
            content_html: prepared.content_html,
            initial_status: prepared.initial_status,
            ip_hash: prepared.ip_hash,
            user_agent_digest: prepared.user_agent_digest,
        })
        .await?;

    // AI 审核挂钩：高置信垃圾标记为 spam，其余保持原状态；失败静默回退。
    let comment = crate::http::ai::moderate_comment_with_ai(&state, comment).await;

    // 评论落库成功即异步通知；入队失败只记日志，不影响响应。
    enqueue_comment_notification(&state, &comment, None).await;

    let logo = state.site_settings.get().await?.logo_url;
    let status = comment.status.as_str().to_owned();
    let body = PublicCommentCreatedResponse {
        comment: to_public_comment(&comment, &logo, Vec::new()),
        status,
    };
    Ok(json_with_cache(&body, CACHE_NO_STORE))
}

async fn reply_comment(
    State(state): State<AppState>,
    Path(comment_id): Path<i64>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ReplyCommentRequest>,
) -> Result<Response, ApiError> {
    let parent = state
        .comments
        .find(comment_id)
        .await?
        .ok_or(CommentError::NotFound)?;
    // 只允许回复已批准的评论；回复未审核中的评论等同于不存在，不泄露其状态。
    if parent.status != CommentStatus::Approved {
        return Err(CommentError::NotFound.into());
    }
    // 层级限制沿用 core 模型：公开端只展示两级，回复的回复挂在同一根下。
    let root_parent_id = if parent.parent_id.is_some() {
        // 父评论本身是回复时，把新回复挂到同一根的直接父级，保持两级展示。
        parent.parent_id
    } else {
        Some(parent.id)
    };

    let prepared = prepare_submission(
        &state,
        &headers,
        &request.nickname,
        &request.email,
        request.website.as_deref(),
        &request.content,
    )
    .await?;

    let comment = state
        .comments
        .create(NewComment {
            target_type: parent.target_type,
            target_id: parent.target_id,
            parent_id: root_parent_id,
            author_name: prepared.author_name,
            author_email: prepared.author_email,
            author_url: prepared.author_url,
            content_markdown: prepared.content_markdown,
            content_html: prepared.content_html,
            initial_status: prepared.initial_status,
            ip_hash: prepared.ip_hash,
            user_agent_digest: prepared.user_agent_digest,
        })
        .await?;

    // AI 审核挂钩：同 create_comment，失败静默回退。
    let comment = crate::http::ai::moderate_comment_with_ai(&state, comment).await;

    // 回复通知被回复评论的作者（管理员回复无访客 Email，recipient 缺省）。
    enqueue_comment_notification(&state, &comment, Some(&parent.author_email)).await;

    let logo = state.site_settings.get().await?.logo_url;
    let status = comment.status.as_str().to_owned();
    let body = PublicCommentCreatedResponse {
        comment: to_public_comment(&comment, &logo, Vec::new()),
        status,
    };
    Ok(json_with_cache(&body, CACHE_NO_STORE))
}
