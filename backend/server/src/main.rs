use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use aries_infra::{
    Argon2PasswordHasher, ComrakMarkdownRenderer, DispatchingAiProvider,
    PostgresAiRequestRepository, PostgresAuthRepository, PostgresChunkRepository,
    PostgresCommentRepository, PostgresContentRepository, PostgresGalleryRepository,
    PostgresJobRepository, PostgresJournalRepository, PostgresLinkRepository,
    PostgresLogRepository, PostgresMediaRepository, PostgresNavigationRepository,
    PostgresPageRepository, PostgresSettingRepository, PostgresSiteSettingsRepository,
};
use aries_server::{build_app, config::ServerConfig, log_store, logging, state::AppState, worker};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_dotenv()?;
    // worker 持有文件日志的后台写入线程，必须存活到进程退出。
    // receiver 暂存 DB 就绪前的早期日志，Migration 完成后由 writer 任务消费。
    let logging::LogGuard {
        worker,
        handle,
        receiver,
        retention_days,
    } = logging::init_tracing()?;
    let _worker = worker;
    let log_handle = Arc::new(handle);

    let config = ServerConfig::from_env()?;
    let database_config = aries_infra::PostgresConfig::from_env()?;
    let database = aries_infra::connect_postgres(&database_config).await?;
    aries_infra::ensure_schema(&database, database_config.schema()).await?;
    aries_infra::run_migrations(&database).await?;
    // 日志表由 Migration 提供，跑完 Migration 再启动写入任务。
    // shutdown 信号端与 JoinHandle 持有到退出路径：SIGTERM 后通知 writer 排空
    // channel 残留日志并等待其写完（退出前的启动失败/panic 前兆日志最有价值）。
    // log_wake：writer 每次批量落库成功后广播，SSE tail 由轮询改为唤醒驱动。
    let (log_shutdown_tx, log_shutdown_rx) = tokio::sync::watch::channel(false);
    let log_wake = log_store::log_wake_channel();
    let log_writer = log_store::spawn_writer(
        database.clone(),
        receiver,
        retention_days,
        log_shutdown_rx,
        log_wake.clone(),
    );
    let storage = aries_infra::storage::storage_from_env()?;

    let state = AppState {
        database: database.clone(),
        auth: Arc::new(PostgresAuthRepository::new(database.clone())),
        content: Arc::new(PostgresContentRepository::new(database.clone())),
        markdown: Arc::new(ComrakMarkdownRenderer),
        passwords: Arc::new(Argon2PasswordHasher),
        media: Arc::new(PostgresMediaRepository::new(database.clone())),
        storage,
        jobs: Arc::new(PostgresJobRepository::new(database.clone())),
        site_settings: Arc::new(PostgresSiteSettingsRepository::new(database.clone())),
        comments: Arc::new(PostgresCommentRepository::new(database.clone())),
        pages: Arc::new(PostgresPageRepository::new(database.clone())),
        journals: Arc::new(PostgresJournalRepository::new(database.clone())),
        galleries: Arc::new(PostgresGalleryRepository::new(database.clone())),
        links: Arc::new(PostgresLinkRepository::new(database.clone())),
        logs: Arc::new(PostgresLogRepository::new(database.clone())),
        navigation: Arc::new(PostgresNavigationRepository::new(database.clone())),
        settings: Arc::new(PostgresSettingRepository::new(database.clone())),
        chunks: Arc::new(PostgresChunkRepository::new(database.clone())),
        // Provider 为无状态 HTTP Client；配置（base_url/model/api_key）随每次请求
        // 从 setting_groups 实时读取，settings 更新后下一次请求即生效。
        ai: Arc::new(DispatchingAiProvider::new()),
        ai_requests: Arc::new(PostgresAiRequestRepository::new(database.clone())),
        config: Arc::new(config.clone()),
        rate_limiter: Default::default(),
        log_handle,
        log_wake,
    };
    let app = build_app(state.clone())?;
    // 后台任务 Worker 随服务启动；失败任务按 max_attempts 自动重试。
    let worker_handle = worker::spawn(state);
    let listener = tokio::net::TcpListener::bind(config.address).await?;
    info!(address = %config.address, "aries server started");
    // ConnectInfo 向 Handler 提供连接对端地址，用于登录限流的 IP 维度。
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;

    // axum 优雅退出完成（在途请求已处理完）后按顺序收尾：
    // 1. 通知日志 writer 排空：先 flush 当前批次，再尽力写完 channel 残留记录。
    //    JoinHandle 外层包 timeout 兜底（writer 内部排空本身有 5s 上限，这里再留余量），
    //    兜底失败只记 error，不阻塞进程退出。
    let _ = log_shutdown_tx.send(true);
    if tokio::time::timeout(std::time::Duration::from_secs(10), log_writer)
        .await
        .is_err()
    {
        tracing::error!("log writer did not finish drain in time; exiting anyway");
    }
    // 2. 后台任务轮询语义是「可直接停」（任务按 max_attempts 重试，无排空需求），
    //    直接 abort；与日志 writer 的排空等待刻意区分。
    worker_handle.abort();
    // 3. `_worker`（tracing_appender 的 WorkerGuard）在此之后随 main 返回 drop，
    //    flush 文件日志缓冲区。

    Ok(())
}

fn load_dotenv() -> anyhow::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(dotenvy::Error::Io(error)) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("failed to load .env"),
    }
}

async fn shutdown() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
