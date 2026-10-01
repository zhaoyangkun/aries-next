//! 后台任务 Worker：轮询 `background_jobs`，执行媒体清理与尺寸补探测。
//! Markdown 导入由 HTTP 同步流程完成，Worker 遇到该类型直接标 Done，避免任务悬挂。
//! 另承载周期性的过期 Session 物理清理，防止 `admin_sessions` 无限增长。

use std::{
    panic::AssertUnwindSafe,
    time::{Duration, Instant},
};

use aries_core::jobs::{BackgroundJob, JobKind};
use futures_util::{FutureExt, StreamExt};
use tracing::Instrument;

use crate::{log_store::QUIET_INTERNAL_SPAN, state::AppState};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// 单次执行处理的最大资产数：限制单任务耗时，剩余资产由下一次清理任务处理。
const BATCH_LIMIT: i64 = 20;
/// 过期 Session 清理周期：每天一次足够，DELETE 谓词走主键外的全表条件，不宜高频。
const SESSION_CLEANUP_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// ERROR 尖峰检测周期：窗口与周期一致（最近 5 分钟的 ERROR 数）。
const SPIKE_CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// 启动周期轮询任务；只在 `main.rs` 调用，集成测试直接驱动 `run_pending_jobs`。
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        // 启动即视为已清理，首次实际清理发生在一天后；刚跑完 Migration 的库无历史过期 Session。
        let mut last_session_cleanup = Instant::now();
        // 尖峰检测同样跳过启动即时点：首个 5 分钟窗口从启动后算起。
        let mut last_spike_check = Instant::now();
        loop {
            ticker.tick().await;
            run_pending_jobs(&state).await;
            if last_session_cleanup.elapsed() >= SESSION_CLEANUP_INTERVAL {
                cleanup_expired_sessions(&state).await;
                last_session_cleanup = Instant::now();
            }
            if last_spike_check.elapsed() >= SPIKE_CHECK_INTERVAL {
                check_error_spike(&state).await;
                last_spike_check = Instant::now();
            }
        }
    })
}

/// ERROR 尖峰检测：最近 5 分钟 ERROR 数达到阈值打 WARN（Email Adapter 接入后可挂通知通道）。
/// 检测查询包进静默 Span：它每 5 分钟执行一次，属例行噪音；WARN 本身照常记录。
pub async fn check_error_spike(state: &AppState) {
    let threshold = state.config.log_error_spike_threshold;
    if threshold == 0 {
        return;
    }
    let since = time::OffsetDateTime::now_utc() - time::Duration::minutes(5);
    let count = async { state.logs.count_errors_since(since).await }
        .instrument(tracing::info_span!(QUIET_INTERNAL_SPAN))
        .await;
    match count {
        Ok(count) if count >= threshold.max(1) as i64 => {
            tracing::warn!(count, threshold, "error spike detected in server logs");
        }
        Ok(_) => {}
        Err(error) => tracing::error!(error = %error, "failed to check error spike"),
    }
}

/// 领取并执行所有到期的 Pending 任务，直到队列为空。
/// 单个任务失败不影响其他任务：记录 last_error 并按 max_attempts 退避重试；
/// 单个任务 panic 同样被隔离为失败，不会杀死整个轮询循环。
pub async fn run_pending_jobs(state: &AppState) {
    loop {
        // 空轮询每 5s 一次，其 SQL 属例行噪音：包进静默 Span 后三路输出层都不记录；
        // 真正领取到任务后的执行与 finalize 日志不受影响。
        let claimed = state
            .jobs
            .claim_next()
            .instrument(tracing::info_span!(QUIET_INTERNAL_SPAN))
            .await;
        match claimed {
            Ok(Some(job)) => {
                // 任务执行与 finalize 包在同一个 Span 内：span_name=background_job、
                // job_id/kind 入 fields，任务内的业务日志与 SQL 日志可整体追踪。
                // 不能用 quiet_internal（那是静默例行噪音），任务日志需要被记录。
                async {
                    let result = match AssertUnwindSafe(execute(state, &job)).catch_unwind().await {
                        Ok(result) => result,
                        Err(_) => {
                            tracing::error!(
                                job_id = job.id,
                                kind = ?job.kind,
                                "background job panicked; marking as failed"
                            );
                            Err("background job panicked".to_owned())
                        }
                    };
                    let outcome = match result {
                        Ok(()) => state.jobs.complete(job.id).await.map_err(|e| e.to_string()),
                        Err(error) => match state.jobs.fail(job.id, &error).await {
                            Ok(_) => Ok(()),
                            Err(e) => Err(e.to_string()),
                        },
                    };
                    if let Err(error) = outcome {
                        tracing::error!(
                            error,
                            job_id = job.id,
                            "failed to finalize background job"
                        );
                    }
                }
                .instrument(tracing::info_span!(
                    "background_job",
                    job_id = job.id,
                    kind = ?job.kind,
                ))
                .await;
            }
            Ok(None) => break,
            Err(error) => {
                tracing::error!(error = %error, "failed to claim background job");
                break;
            }
        }
    }
}

/// 物理删除已过期或已撤销的 Session 行；失败仅记日志，不影响任务轮询。
pub async fn cleanup_expired_sessions(state: &AppState) {
    match state.auth.delete_expired_sessions().await {
        Ok(deleted) => {
            if deleted > 0 {
                tracing::info!(deleted, "expired admin sessions physically removed");
            }
        }
        Err(error) => {
            tracing::error!(error = %error, "failed to clean up expired admin sessions");
        }
    }
}

async fn execute(state: &AppState, job: &BackgroundJob) -> Result<(), String> {
    match job.kind {
        JobKind::MediaCleanup => media_cleanup(state).await,
        JobKind::MetadataProbe => metadata_probe(state).await,
        // 导入在 Commit 端点同步完成并直接置 Done；到这里说明是历史遗留任务。
        JobKind::ImportMarkdown => Ok(()),
        // 评论通知由 Background Job 异步发送；Phase 05 首版先记录日志，后续接入 Email Adapter。
        JobKind::CommentNotification => comment_notification(state, job).await,
    }
}

/// 物理清除已软删除且零引用的资产：先在事务内复核引用并删除数据库行，成功后再删存储对象。
/// 顺序不可调换：若先删文件，purge 随后返回 Referenced 会造成已发布内容引用坏图。
async fn media_cleanup(state: &AppState) -> Result<(), String> {
    let purgeable = state
        .media
        .list_purgeable(BATCH_LIMIT)
        .await
        .map_err(|error| error.to_string())?;
    for asset in purgeable {
        // purge 在事务内复核引用数；被并发引用的资产会返回 Referenced，
        // 此时存储对象必须保持完好，不得触碰。
        match state.media.purge(asset.id).await {
            Ok(()) => {}
            Err(aries_core::media::MediaError::Referenced { .. }) => continue,
            // 行已被并发清理（含资产被恢复引用导致 status 变化）：对象归属不明，跳过删除，
            // 宁可留下孤儿文件也不误删仍被引用的对象。
            Err(aries_core::media::MediaError::NotFound) => continue,
            Err(error) => return Err(error.to_string()),
        }
        // 数据库行已删；存储对象删除失败只产生无害的孤儿文件，记日志即可，不阻断任务。
        if let Err(error) = state.storage.delete(&asset.object_key).await {
            tracing::error!(
                error = %error,
                asset_id = asset.id,
                object_key = %asset.object_key,
                "media asset purged from database but storage object deletion failed; orphan file left behind"
            );
        }
    }
    Ok(())
}

/// 评论通知任务：首版仅记录日志，后续接入 Email Adapter 后发送实际通知邮件。
async fn comment_notification(_state: &AppState, job: &BackgroundJob) -> Result<(), String> {
    let comment_id = job
        .payload
        .get("comment_id")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let recipient = job
        .payload
        .get("recipient_email")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    tracing::info!(
        comment_id,
        recipient,
        "comment notification job executed (email adapter not yet implemented)"
    );
    Ok(())
}
async fn metadata_probe(state: &AppState) -> Result<(), String> {
    let missing = state
        .media
        .list_missing_dimensions(BATCH_LIMIT)
        .await
        .map_err(|error| error.to_string())?;
    // 单个资产探测失败（如存储对象缺失/损坏）只记 WARN 并继续处理下一个，
    // 与 media_cleanup 的孤儿文件处理哲学一致：坏资产不得让整个 job 失败。
    // buffered 限流并发读取存储对象，避免 S3 等远程存储下 20 个资产串行排队。
    let results: Vec<Result<(), String>> = futures_util::stream::iter(missing)
        .map(|asset| probe_asset_dimensions(state, asset))
        .buffered(4)
        .collect()
        .await;
    for result in results {
        result?;
    }
    Ok(())
}

/// 探测单个资产的尺寸并回写；存储读取失败记 WARN 跳过（Ok），
/// 只有尺寸回写失败才返回 Err 让 job 失败重试。
async fn probe_asset_dimensions(
    state: &AppState,
    asset: aries_core::media::MediaAsset,
) -> Result<(), String> {
    let bytes = match state.storage.get(&asset.object_key).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(()),
        Err(error) => {
            tracing::warn!(
                error = %error,
                asset_id = asset.id,
                object_key = %asset.object_key,
                "metadata probe failed to read storage object; skipping asset"
            );
            return Ok(());
        }
    };
    if let Some((width, height)) = aries_infra::upload::probe_dimensions(&bytes) {
        state
            .media
            .update_dimensions(asset.id, width, height)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
