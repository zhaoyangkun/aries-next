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

#[derive(Debug, Serialize)]
struct DashboardResponse {
    articles: ArticleStats,
    comments: CommentStats,
    recent_pending_comments: Vec<CommentResponse>,
    recent_failed_jobs: Vec<FailedJobResponse>,
}

#[derive(Debug, Serialize)]
struct ArticleStats {
    total: i64,
    draft: i64,
    published: i64,
    recycled: i64,
}

#[derive(Debug, Serialize)]
struct CommentStats {
    total: i64,
    pending: i64,
    today: i64,
}

#[derive(Debug, Serialize)]
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
