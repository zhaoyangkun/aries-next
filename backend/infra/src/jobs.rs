use std::str::FromStr;

use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use aries_core::jobs::{
    BackgroundJob, JobError, JobKind, JobListQuery, JobPage, JobRepository, JobStatus,
    NewBackgroundJob,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresJobRepository {
    pool: PgPool,
}

impl PostgresJobRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, FromRow)]
struct JobRow {
    id: i64,
    kind: String,
    payload: serde_json::Value,
    status: String,
    attempts: i32,
    max_attempts: i32,
    run_at: OffsetDateTime,
    last_error: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<JobRow> for BackgroundJob {
    type Error = JobError;

    fn try_from(row: JobRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            kind: JobKind::from_str(&row.kind)?,
            payload: row.payload,
            status: JobStatus::from_str(&row.status)?,
            attempts: row.attempts,
            max_attempts: row.max_attempts,
            run_at: row.run_at,
            last_error: row.last_error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const JOB_COLUMNS: &str = "id, kind, payload, status, attempts, max_attempts, run_at, \
    last_error, created_at, updated_at";

#[async_trait]
impl JobRepository for PostgresJobRepository {
    async fn enqueue(&self, job: NewBackgroundJob) -> Result<BackgroundJob, JobError> {
        let query = format!(
            "INSERT INTO background_jobs (kind, payload, max_attempts) \
             VALUES ($1, $2, $3) RETURNING {JOB_COLUMNS}"
        );
        let row = logged_query_as::<JobRow>(&query)
            .bind(job.kind.as_str())
            .bind(job.payload)
            .bind(job.max_attempts)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }

    async fn find(&self, job_id: i64) -> Result<Option<BackgroundJob>, JobError> {
        let query = format!("SELECT {JOB_COLUMNS} FROM background_jobs WHERE id = $1");
        let row = logged_query_as::<JobRow>(&query)
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn claim_next(&self) -> Result<Option<BackgroundJob>, JobError> {
        // FOR UPDATE SKIP LOCKED：并发 Worker 各自领取不同任务，不互相阻塞也不重复执行。
        let query = format!(
            "UPDATE background_jobs SET status = 'running', updated_at = now() \
             WHERE id = (SELECT id FROM background_jobs \
                 WHERE status = 'pending' AND run_at <= now() \
                 ORDER BY run_at, id LIMIT 1 FOR UPDATE SKIP LOCKED) \
             RETURNING {JOB_COLUMNS}"
        );
        let row = logged_query_as::<JobRow>(&query)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn complete(&self, job_id: i64) -> Result<(), JobError> {
        let result = logged_query(
            "UPDATE background_jobs SET status = 'done', last_error = NULL, updated_at = now() \
             WHERE id = $1 AND status IN ('pending', 'running')",
        )
        .bind(job_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(JobError::NotFound);
        }
        Ok(())
    }

    async fn fail(&self, job_id: i64, error: &str) -> Result<BackgroundJob, JobError> {
        // attempts 递增后与 max_attempts 比较：超限转 Failed，否则回到 Pending 延迟重试。
        let query = format!(
            "UPDATE background_jobs SET attempts = attempts + 1, last_error = $2, \
             status = CASE WHEN attempts + 1 >= max_attempts THEN 'failed' ELSE 'pending' END, \
             run_at = CASE WHEN attempts + 1 >= max_attempts THEN run_at \
                 ELSE now() + interval '30 seconds' END, \
             updated_at = now() \
             WHERE id = $1 AND status IN ('pending', 'running') RETURNING {JOB_COLUMNS}"
        );
        let row = logged_query_as::<JobRow>(&query)
            .bind(job_id)
            .bind(error.chars().take(2000).collect::<String>())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(JobError::NotFound)?;
        row.try_into()
    }

    async fn list(&self, query: JobListQuery) -> Result<JobPage, JobError> {
        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, 100);
        let offset = i64::from(page - 1) * i64::from(page_size);
        let kind = query.kind.map(|value| value.as_str().to_owned());
        let status = query.status.map(|value| value.as_str().to_owned());

        let total = logged_query_scalar::<i64>(
            "SELECT count(*) FROM background_jobs \
             WHERE ($1::text IS NULL OR kind = $1) AND ($2::text IS NULL OR status = $2)",
        )
        .bind(kind.as_deref())
        .bind(status.as_deref())
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let list_query = format!(
            "SELECT {JOB_COLUMNS} FROM background_jobs \
             WHERE ($1::text IS NULL OR kind = $1) AND ($2::text IS NULL OR status = $2) \
             ORDER BY id DESC LIMIT $3 OFFSET $4"
        );
        let rows = logged_query_as::<JobRow>(&list_query)
            .bind(kind.as_deref())
            .bind(status.as_deref())
            .bind(i64::from(page_size))
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let items = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(JobPage {
            items,
            total,
            page,
            page_size,
        })
    }
}

fn map_sqlx(error: sqlx::Error) -> JobError {
    tracing::error!(error = %error, "job repository operation failed");
    JobError::StoreUnavailable
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, ensure};
    use uuid::Uuid;

    use super::*;

    /// 搭建随机 Schema 隔离的测试库，与 content.rs 的集成测试同一模式：
    /// 环境变量门控 + dotenvy 读取 .env + 独立 search_path 运行 Migration。
    async fn create_test_schema() -> anyhow::Result<(PgPool, PgPool, String)> {
        let _ = dotenvy::dotenv();
        let base_config = crate::PostgresConfig::from_env()?;
        let admin_pool = crate::connect_postgres(&base_config).await?;
        let test_schema = format!("aries_test_jobs_{}", Uuid::now_v7().simple());

        // 随机 Schema 隔离业务数据，测试无论成功或失败都会执行清理。
        logged_query(&format!("CREATE SCHEMA \"{test_schema}\""))
            .execute(&admin_pool)
            .await
            .context("failed to create isolated job test schema")?;
        let test_config =
            base_config.with_search_path(format!("{test_schema},{}", base_config.schema()));
        let test_pool = crate::connect_postgres(&test_config).await?;
        crate::run_migrations(&test_pool).await?;
        Ok((admin_pool, test_pool, test_schema))
    }

    async fn drop_test_schema(admin_pool: &PgPool, test_schema: &str) -> anyhow::Result<()> {
        logged_query(&format!("DROP SCHEMA \"{test_schema}\" CASCADE"))
            .execute(admin_pool)
            .await
            .context("failed to remove isolated job test schema")?;
        Ok(())
    }

    #[tokio::test]
    async fn postgresql_job_repository_covers_claim_and_complete_lifecycle() -> anyhow::Result<()> {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let (admin_pool, test_pool, test_schema) = create_test_schema().await?;
        let scenario_result = run_claim_lifecycle_scenario(&test_pool).await;

        test_pool.close().await;
        let cleanup_result = drop_test_schema(&admin_pool, &test_schema).await;
        admin_pool.close().await;

        scenario_result?;
        cleanup_result?;
        Ok(())
    }

    async fn run_claim_lifecycle_scenario(pool: &PgPool) -> anyhow::Result<()> {
        let repository = PostgresJobRepository::new(pool.clone());

        // enqueue：默认 pending / attempts = 0，payload 原样落库。
        let first = repository
            .enqueue(NewBackgroundJob::new(
                JobKind::MediaCleanup,
                serde_json::json!({"asset_id": 1}),
            ))
            .await?;
        ensure!(first.status == JobStatus::Pending);
        ensure!(first.attempts == 0);
        ensure!(first.max_attempts == 3);
        ensure!(first.payload == serde_json::json!({"asset_id": 1}));
        ensure!(first.last_error.is_none());
        let second = repository
            .enqueue(NewBackgroundJob {
                kind: JobKind::CommentNotification,
                payload: serde_json::json!({}),
                max_attempts: 1,
            })
            .await?;

        // find：命中返回完整任务；未知 ID 返回 None 而不是报错。
        let found = repository
            .find(first.id)
            .await?
            .context("enqueued job must be found")?;
        ensure!(found.kind == JobKind::MediaCleanup);
        ensure!(repository.find(i64::MAX).await?.is_none());

        // claim_next 按 run_at、id 顺序领取：先入库的先执行，状态转 Running。
        let claimed = repository
            .claim_next()
            .await?
            .context("pending job must be claimable")?;
        ensure!(claimed.id == first.id);
        ensure!(claimed.status == JobStatus::Running);
        // Running 中的任务不会被重复领取，轮到下一条 Pending。
        let claimed_second = repository
            .claim_next()
            .await?
            .context("second pending job must be claimable")?;
        ensure!(claimed_second.id == second.id);
        ensure!(repository.claim_next().await?.is_none());

        // complete：仅 pending/running 允许完成，重复完成或未知 ID 报 NotFound。
        repository.complete(claimed.id).await?;
        let done = repository
            .find(first.id)
            .await?
            .context("completed job must be found")?;
        ensure!(done.status == JobStatus::Done);
        ensure!(done.last_error.is_none());
        ensure!(matches!(
            repository.complete(first.id).await,
            Err(JobError::NotFound)
        ));
        ensure!(matches!(
            repository.complete(i64::MAX).await,
            Err(JobError::NotFound)
        ));

        // fail：max_attempts = 1 时一次失败直接转 Failed，并记录 last_error。
        let failed = repository.fail(second.id, "boom").await?;
        ensure!(failed.status == JobStatus::Failed);
        ensure!(failed.attempts == 1);
        ensure!(failed.last_error.as_deref() == Some("boom"));
        // Failed 是终态：不能再 fail / complete，也不会被领取。
        ensure!(matches!(
            repository.fail(second.id, "boom").await,
            Err(JobError::NotFound)
        ));
        ensure!(matches!(
            repository.complete(second.id).await,
            Err(JobError::NotFound)
        ));
        ensure!(repository.claim_next().await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn postgresql_job_repository_retries_with_backoff_and_lists() -> anyhow::Result<()> {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let (admin_pool, test_pool, test_schema) = create_test_schema().await?;
        let scenario_result = run_retry_and_list_scenario(&test_pool).await;

        test_pool.close().await;
        let cleanup_result = drop_test_schema(&admin_pool, &test_schema).await;
        admin_pool.close().await;

        scenario_result?;
        cleanup_result?;
        Ok(())
    }

    async fn run_retry_and_list_scenario(pool: &PgPool) -> anyhow::Result<()> {
        let repository = PostgresJobRepository::new(pool.clone());
        let job = repository
            .enqueue(NewBackgroundJob {
                kind: JobKind::MetadataProbe,
                payload: serde_json::json!({}),
                max_attempts: 2,
            })
            .await?;

        // 第一次失败：attempts +1 未超限，回到 Pending 并延迟 30 秒重试。
        let claimed = repository
            .claim_next()
            .await?
            .context("pending job must be claimable")?;
        let retried = repository.fail(claimed.id, "temporary").await?;
        ensure!(retried.status == JobStatus::Pending);
        ensure!(retried.attempts == 1);
        ensure!(retried.last_error.as_deref() == Some("temporary"));
        ensure!(retried.run_at > OffsetDateTime::now_utc());
        // run_at 未到，claim_next 不能立即再次领取（延迟重试窗口）。
        ensure!(repository.claim_next().await?.is_none());

        // 把 run_at 拨回当前模拟重试窗口到期，任务可被再次领取。
        logged_query("UPDATE background_jobs SET run_at = now() WHERE id = $1")
            .bind(job.id)
            .execute(pool)
            .await?;
        let reclaimed = repository
            .claim_next()
            .await?
            .context("retried job must be claimable after run_at")?;
        ensure!(reclaimed.id == job.id);
        ensure!(reclaimed.attempts == 1);
        // 第二次失败：attempts 达到 max_attempts，转 Failed。
        let failed = repository.fail(reclaimed.id, "permanent").await?;
        ensure!(failed.status == JobStatus::Failed);
        ensure!(failed.attempts == 2);
        ensure!(failed.last_error.as_deref() == Some("permanent"));

        // last_error 截断到 2000 字符，避免超长堆栈撑爆行。
        let long_error = "x".repeat(5000);
        let another = repository
            .enqueue(NewBackgroundJob::new(
                JobKind::ImportMarkdown,
                serde_json::json!({}),
            ))
            .await?;
        let failed_long = repository.fail(another.id, &long_error).await?;
        ensure!(failed_long.status == JobStatus::Pending);
        ensure!(
            failed_long
                .last_error
                .as_deref()
                .is_some_and(|error| error.chars().count() == 2000)
        );

        let third = repository
            .enqueue(NewBackgroundJob::new(
                JobKind::MediaCleanup,
                serde_json::json!({}),
            ))
            .await?;

        // list：不过滤返回全部，按 id DESC（最新入库在前）。
        // 注意：page_size 默认 0 会被 clamp 到 1，必须显式给分页大小。
        let all = repository
            .list(JobListQuery {
                page: 1,
                page_size: 10,
                ..JobListQuery::default()
            })
            .await?;
        ensure!(all.total == 3 && all.items.len() == 3);
        ensure!(all.items[0].id == third.id);
        ensure!(all.items[2].id == job.id);
        // 按 status / kind 过滤，total 与过滤结果一致。
        let failed_only = repository
            .list(JobListQuery {
                status: Some(JobStatus::Failed),
                ..JobListQuery::default()
            })
            .await?;
        ensure!(failed_only.total == 1 && failed_only.items[0].id == job.id);
        let by_kind = repository
            .list(JobListQuery {
                kind: Some(JobKind::ImportMarkdown),
                ..JobListQuery::default()
            })
            .await?;
        ensure!(by_kind.total == 1 && by_kind.items[0].id == another.id);
        // 组合过滤：kind + status 同时生效。
        let combined = repository
            .list(JobListQuery {
                kind: Some(JobKind::ImportMarkdown),
                status: Some(JobStatus::Failed),
                ..JobListQuery::default()
            })
            .await?;
        ensure!(combined.total == 0 && combined.items.is_empty());

        // 分页：page 0 按 1 处理，page_size 超上限截断到 100。
        let clamped = repository
            .list(JobListQuery {
                page: 0,
                page_size: 1000,
                ..JobListQuery::default()
            })
            .await?;
        ensure!(clamped.page == 1 && clamped.page_size == 100);
        ensure!(clamped.items.len() == 3);
        let first_page = repository
            .list(JobListQuery {
                page: 1,
                page_size: 2,
                ..JobListQuery::default()
            })
            .await?;
        let second_page = repository
            .list(JobListQuery {
                page: 2,
                page_size: 2,
                ..JobListQuery::default()
            })
            .await?;
        ensure!(first_page.items.len() == 2 && second_page.items.len() == 1);
        // 两页拼接恰好覆盖全部且不重叠（id DESC）。
        ensure!(first_page.items[0].id == third.id);
        ensure!(first_page.items[1].id == another.id);
        ensure!(second_page.items[0].id == job.id);
        Ok(())
    }

    #[tokio::test]
    async fn postgresql_job_repository_claim_next_skips_locked_rows() -> anyhow::Result<()> {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let (admin_pool, test_pool, test_schema) = create_test_schema().await?;
        let scenario_result = run_skip_locked_scenario(&test_pool).await;

        test_pool.close().await;
        let cleanup_result = drop_test_schema(&admin_pool, &test_schema).await;
        admin_pool.close().await;

        scenario_result?;
        cleanup_result?;
        Ok(())
    }

    /// SKIP LOCKED 语义：被其他事务锁住的 Pending 任务必须被跳过，
    /// 否则并发 Worker 会在锁上阻塞，失去并行消费能力。
    async fn run_skip_locked_scenario(pool: &PgPool) -> anyhow::Result<()> {
        let repository = PostgresJobRepository::new(pool.clone());
        let locked = repository
            .enqueue(NewBackgroundJob::new(
                JobKind::MediaCleanup,
                serde_json::json!({}),
            ))
            .await?;
        let free = repository
            .enqueue(NewBackgroundJob::new(
                JobKind::MetadataProbe,
                serde_json::json!({}),
            ))
            .await?;

        // 在未提交事务里显式锁住第一条任务，模拟另一个 Worker 正在处理。
        let mut transaction = pool.begin().await?;
        let (locked_id,): (i64,) =
            sqlx::query_as("SELECT id FROM background_jobs WHERE id = $1 FOR UPDATE")
                .bind(locked.id)
                .fetch_one(&mut *transaction)
                .await?;
        ensure!(locked_id == locked.id);

        // claim_next 跳过被锁任务领取下一条，而不是阻塞等待锁释放。
        let claimed = repository
            .claim_next()
            .await?
            .context("claim_next must skip the locked row")?;
        ensure!(claimed.id == free.id);
        ensure!(claimed.status == JobStatus::Running);
        // 剩余的 Pending 只有被锁住的那条，claim_next 立即返回 None。
        ensure!(repository.claim_next().await?.is_none());

        // 锁释放后，原任务可以正常领取。
        transaction.rollback().await?;
        let claimed_after = repository
            .claim_next()
            .await?
            .context("previously locked job must be claimable after rollback")?;
        ensure!(claimed_after.id == locked.id);
        Ok(())
    }
}
