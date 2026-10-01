use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use aries_infra::{
    Argon2PasswordHasher, ComrakMarkdownRenderer, DispatchingAiProvider,
    PostgresAiRequestRepository, PostgresAuthRepository, PostgresCommentRepository,
    PostgresContentRepository, PostgresGalleryRepository, PostgresJobRepository,
    PostgresJournalRepository, PostgresLinkRepository, PostgresLogRepository,
    PostgresMediaRepository, PostgresNavigationRepository, PostgresPageRepository,
    PostgresSettingRepository, PostgresSiteSettingsRepository,
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
    log_store::spawn_writer(database.clone(), receiver, retention_days);
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
        // Provider 为无状态 HTTP Client；配置（base_url/model/api_key）随每次请求
        // 从 setting_groups 实时读取，settings 更新后下一次请求即生效。
        ai: Arc::new(DispatchingAiProvider::new()),
        ai_requests: Arc::new(PostgresAiRequestRepository::new(database.clone())),
        config: Arc::new(config.clone()),
        rate_limiter: Default::default(),
        log_handle,
    };
    let app = build_app(state.clone())?;
    // 后台任务 Worker 随服务启动；失败任务按 max_attempts 自动重试。
    worker::spawn(state);
    let listener = tokio::net::TcpListener::bind(config.address).await?;
    info!(address = %config.address, "aries server started");
    // ConnectInfo 向 Handler 提供连接对端地址，用于登录限流的 IP 维度。
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;

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
