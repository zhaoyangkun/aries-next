use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;

/// 任务类型固定枚举，与数据库 Check Constraint 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    MediaCleanup,
    ImportMarkdown,
    MetadataProbe,
    CommentNotification,
}

impl JobKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MediaCleanup => "media_cleanup",
            Self::ImportMarkdown => "import_markdown",
            Self::MetadataProbe => "metadata_probe",
            Self::CommentNotification => "comment_notification",
        }
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for JobKind {
    type Err = JobError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "media_cleanup" => Ok(Self::MediaCleanup),
            "import_markdown" => Ok(Self::ImportMarkdown),
            "metadata_probe" => Ok(Self::MetadataProbe),
            "comment_notification" => Ok(Self::CommentNotification),
            _ => Err(JobError::InvalidKind),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl JobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for JobStatus {
    type Err = JobError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            _ => Err(JobError::InvalidStatus),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackgroundJob {
    pub id: i64,
    pub kind: JobKind,
    pub payload: serde_json::Value,
    pub status: JobStatus,
    pub attempts: i32,
    pub max_attempts: i32,
    pub run_at: OffsetDateTime,
    pub last_error: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct NewBackgroundJob {
    pub kind: JobKind,
    pub payload: serde_json::Value,
    pub max_attempts: i32,
}

impl NewBackgroundJob {
    /// 默认重试 3 次：瞬时故障（如存储不可用）通常重试即可恢复。
    pub fn new(kind: JobKind, payload: serde_json::Value) -> Self {
        Self {
            kind,
            payload,
            max_attempts: 3,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct JobListQuery {
    pub page: u32,
    pub page_size: u32,
    pub kind: Option<JobKind>,
    pub status: Option<JobStatus>,
}

#[derive(Debug, Clone)]
pub struct JobPage {
    pub items: Vec<BackgroundJob>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum JobError {
    #[error("invalid background job kind")]
    InvalidKind,
    #[error("invalid background job status")]
    InvalidStatus,
    #[error("background job not found")]
    NotFound,
    #[error("job store unavailable")]
    StoreUnavailable,
}

#[async_trait]
pub trait JobRepository: Send + Sync {
    async fn enqueue(&self, job: NewBackgroundJob) -> Result<BackgroundJob, JobError>;
    async fn find(&self, job_id: i64) -> Result<Option<BackgroundJob>, JobError>;
    /// 领取下一个到期的 Pending 任务并标记 Running；
    /// 实现必须使用 `FOR UPDATE SKIP LOCKED`，保证多个 Worker 不会重复执行同一任务。
    async fn claim_next(&self) -> Result<Option<BackgroundJob>, JobError>;
    async fn complete(&self, job_id: i64) -> Result<(), JobError>;
    /// 记录失败并递增 attempts；未超限时回到 Pending 等待重试，超限转为 Failed。
    async fn fail(&self, job_id: i64, error: &str) -> Result<BackgroundJob, JobError>;
    async fn list(&self, query: JobListQuery) -> Result<JobPage, JobError>;
}
