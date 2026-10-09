use std::sync::Arc;

use aries_core::{
    ai::{AiProvider, AiRequestRepository},
    auth::{AuthRepository, PasswordHasher},
    comments::CommentRepository,
    content::{ContentRepository, MarkdownRenderer},
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
    config::ServerConfig, log_store::LogWakeSender, logging::LogHandle, rate_limit::RateLimiter,
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
    pub config: Arc<ServerConfig>,
    pub rate_limiter: RateLimiter,
    pub log_handle: Arc<LogHandle>,
    /// 日志落库唤醒信号（`log_store::spawn_writer` 每次批量 INSERT 成功后广播）：
    /// SSE tail 连接 `subscribe()` 后按需拉取，空闲时零 DB 轮询。
    pub log_wake: LogWakeSender,
}
