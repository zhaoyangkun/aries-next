//! Admin Dashboard 聚合端点：文章/评论统计 + 最近待审核评论 + 最近失败 Job。

use aries_core::{
    auth::Permission,
    jobs::{BackgroundJob, JobStatus},
};
use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;
use time::OffsetDateTime;

use crate::state::AppState;

use super::{auth::CurrentUser, comments::CommentResponse, error::ApiError};

pub fn router() -> Router<AppState> {
    Router::new().route("/dashboard", get(get_dashboard))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "DashboardResponse",
    description = "Dashboard 聚合：文章/评论统计、最近待审核评论与最近失败的后台任务。"
)]
struct DashboardResponse {
    articles: ArticleStats,
    comments: CommentStats,
    recent_pending_comments: Vec<CommentResponse>,
    recent_failed_jobs: Vec<FailedJobResponse>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "ArticleStats", description = "文章计数统计。")]
struct ArticleStats {
    /// 全部文章数（不含软删除）。
    total: i64,
    /// 草稿数。
    draft: i64,
    /// 已发布数。
    published: i64,
    /// 回收站数。
    recycled: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(title = "CommentStats", description = "评论计数统计。")]
struct CommentStats {
    /// 全部评论数。
    total: i64,
    /// 待审核数。
    pending: i64,
    /// 今日新增数。
    today: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "FailedJob",
    description = "最近失败的后台任务（status = failed）。"
)]
struct FailedJobResponse {
    id: i64,
    kind: String,
    attempts: i32,
    max_attempts: i32,
    last_error: Option<String>,
    updated_at: OffsetDateTime,
}

impl From<&BackgroundJob> for FailedJobResponse {
    fn from(job: &BackgroundJob) -> Self {
        debug_assert_eq!(job.status, JobStatus::Failed);
        Self {
            id: job.id,
            kind: job.kind.as_str().to_owned(),
            attempts: job.attempts,
            max_attempts: job.max_attempts,
            last_error: job.last_error.clone(),
            updated_at: job.updated_at,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/admin/dashboard",
    tag = "Admin Dashboard",
    operation_id = "getDashboard",
    summary = "Dashboard 聚合统计",
    description = "Dashboard 聚合：文章/评论统计、最近待审核评论与最近失败的后台任务，一次请求完成。",
    security(("cookieAuth" = [])),
    responses(
        (status = 200, description = "聚合统计", body = DashboardResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 dashboard:view 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_dashboard(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<DashboardResponse>, ApiError> {
    current.require(Permission::ViewDashboard)?;
    let stats = state.comments.dashboard_stats().await?;
    Ok(Json(DashboardResponse {
        articles: ArticleStats {
            total: stats.article_total,
            draft: stats.article_draft,
            published: stats.article_published,
            recycled: stats.article_recycled,
        },
        comments: CommentStats {
            total: stats.comment_total,
            pending: stats.comment_pending,
            today: stats.comment_today,
        },
        recent_pending_comments: stats
            .recent_pending_comments
            .into_iter()
            .map(Into::into)
            .collect(),
        recent_failed_jobs: stats.recent_failed_jobs.iter().map(Into::into).collect(),
    }))
}
