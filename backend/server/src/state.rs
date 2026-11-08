//! AppState：全部 Repository / Service 的装配点，以 Arc<dyn Trait> 持有实现（依赖注入与测试替身的基础）。
use std::sync::Arc;

use aries_core::{
    ai::{AiProvider, AiRequestRepository},
    auth::{AuthRepository, PasswordHasher},
    comments::CommentRepository,
    content::{ContentRepository, MarkdownRenderer},
    email::{EmailDeliveryRepository, EmailSender},
    galleries::GalleryRepository,
    jobs::JobRepository,
    journals::JournalRepository,
    links::LinkRepository,
    logs::LogRepository,
    media::{MediaRepository, MediaStorage, SiteSettingsRepository},
    navigation::NavigationRepository,
    pages::PageRepository,
    retrieval::ChunkRepository,
    settings::SettingRepository,
};
use sqlx::PgPool;

use crate::{
    config::ServerConfig,
    log_store::LogWakeSender,
    logging::LogHandle,
    metrics::{MetricsCollector, MetricsHistory},
    rate_limit::RateLimiter,
};

#[derive(Clone)]
pub struct AppState {
    pub database: PgPool,
    pub auth: Arc<dyn AuthRepository>,
    pub content: Arc<dyn ContentRepository>,
    pub markdown: Arc<dyn MarkdownRenderer>,
    pub passwords: Arc<dyn PasswordHasher>,
    pub media: Arc<dyn MediaRepository>,
    pub storage: Arc<dyn MediaStorage>,
    pub jobs: Arc<dyn JobRepository>,
    pub site_settings: Arc<dyn SiteSettingsRepository>,
    pub comments: Arc<dyn CommentRepository>,
    pub pages: Arc<dyn PageRepository>,
    pub journals: Arc<dyn JournalRepository>,
    pub galleries: Arc<dyn GalleryRepository>,
    pub links: Arc<dyn LinkRepository>,
    pub logs: Arc<dyn LogRepository>,
    pub navigation: Arc<dyn NavigationRepository>,
    pub settings: Arc<dyn SettingRepository>,
    pub chunks: Arc<dyn ChunkRepository>,
    pub ai: Arc<dyn AiProvider>,
    pub ai_requests: Arc<dyn AiRequestRepository>,
    pub email: Arc<dyn EmailSender>,
    pub email_deliveries: Arc<dyn EmailDeliveryRepository>,
    pub config: Arc<ServerConfig>,
    pub rate_limiter: RateLimiter,
    pub log_handle: Arc<LogHandle>,
    /// 日志落库唤醒信号（`log_store::spawn_writer` 每次批量 INSERT 成功后广播）：
    /// SSE tail 连接 `subscribe()` 后按需拉取，空闲时零 DB 轮询。
    pub log_wake: LogWakeSender,
    /// 系统指标采集器（主机 + 进程自身 + PostgreSQL 快照缓存）。
    pub metrics: Arc<MetricsCollector>,
    /// 历史趋势环形缓冲（10s 采样、24h 容量；由 `metrics::spawn_history_sampler` 回填）。
    pub metrics_history: Arc<MetricsHistory>,
}
