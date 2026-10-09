use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use aries_core::auth::{
    AuditEvent, AuditListQuery, AuditPage, AuthError, AuthRepository, AuthenticatedSession,
    CredentialUser, NewOwner, NewPasswordReset, NewSession, PasswordHasher, ProfileUpdate, Role,
    User, UserStatus,
};
use aries_core::content::{
    Article, ArticleListQuery, ArticlePage, ArticleRevision, ArticleStatus, ArticleUpdate,
    Category, CategoryKind, CategoryUpdate, ContentError, ContentRepository, MarkdownRenderer,
    MoveDirection, NewArticle, NewCategory, NewTag, RevisionRestore, Tag, TagUpdate,
};
use aries_server::{build_app, config::ServerConfig, rate_limit::RateLimiter, state::AppState};
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode, header},
};
use http_body_util::BodyExt as _;
use sqlx::postgres::PgPoolOptions;
use time::Duration;
use tower::ServiceExt as _;
use uuid::Uuid;

const ORIGIN: &str = "http://127.0.0.1:5173";
const PASSWORD: &str = "reliable-pass-2026";

#[derive(Default)]
struct MockData {
    credentials: Option<CredentialUser>,
    sessions: HashMap<Vec<u8>, AuthenticatedSession>,
    reset_tokens: HashMap<Vec<u8>, i64>,
    audits: Vec<AuditEvent>,
}

#[derive(Default)]
struct MockAuthRepository {
    data: Mutex<MockData>,
}

impl MockAuthRepository {
    fn with_owner() -> Self {
        Self::with_password_hash(format!("hash:{PASSWORD}"))
    }

    fn with_legacy_owner() -> Self {
        Self::with_password_hash(format!("legacy:{PASSWORD}"))
    }

    fn with_password_hash(password_hash: String) -> Self {
        Self {
            data: Mutex::new(MockData {
                credentials: Some(CredentialUser {
                    user: owner(),
                    password_hash,
                }),
                ..Default::default()
            }),
        }
    }

    fn stored_password_hash(&self) -> String {
        self.data
            .lock()
            .unwrap()
            .credentials
            .as_ref()
            .unwrap()
            .password_hash
            .clone()
    }
}

#[async_trait]
impl AuthRepository for MockAuthRepository {
    async fn is_bootstrapped(&self) -> Result<bool, AuthError> {
        Ok(self.data.lock().unwrap().credentials.is_some())
    }

    async fn create_owner(&self, new_owner: NewOwner) -> Result<User, AuthError> {
        let mut data = self.data.lock().unwrap();
        if data.credentials.is_some() {
            return Err(AuthError::Conflict);
        }
        let user = User {
            id: 1,
            username: new_owner.username,
            email: new_owner.email,
            display_name: new_owner.display_name,
            avatar_url: None,
            role: Role::Owner,
            status: UserStatus::Active,
        };
        data.credentials = Some(CredentialUser {
            user: user.clone(),
            password_hash: new_owner.password_hash,
        });
        Ok(user)
    }

    async fn find_credentials(&self, login: &str) -> Result<Option<CredentialUser>, AuthError> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .credentials
            .clone()
            .filter(|credentials| {
                credentials.user.username == login || credentials.user.email == login
            }))
    }

    async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, AuthError> {
        Ok(self
            .data
            .lock()
            .unwrap()
            .credentials
            .as_ref()
            .map(|credentials| credentials.user.clone())
            .filter(|user| user.email == email))
    }

    async fn create_session(&self, session: NewSession) -> Result<(), AuthError> {
        let mut data = self.data.lock().unwrap();
        let user = data
            .credentials
            .as_ref()
            .map(|credentials| credentials.user.clone())
            .ok_or(AuthError::NotFound)?;
        data.sessions.insert(
            session.token_hash,
            AuthenticatedSession {
                id: session.id,
                user,
                expires_at: session.expires_at,
            },
        );
        Ok(())
    }

    async fn find_session(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<AuthenticatedSession>, AuthError> {
        Ok(self.data.lock().unwrap().sessions.get(token_hash).cloned())
    }

    async fn touch_session(&self, _session_id: Uuid) -> Result<(), AuthError> {
        Ok(())
    }

    async fn revoke_session(&self, session_id: Uuid) -> Result<(), AuthError> {
        self.data
            .lock()
            .unwrap()
            .sessions
            .retain(|_, session| session.id != session_id);
        Ok(())
    }

    async fn revoke_user_sessions(&self, user_id: i64) -> Result<(), AuthError> {
        self.data
            .lock()
            .unwrap()
            .sessions
            .retain(|_, session| session.user.id != user_id);
        Ok(())
    }

    async fn update_last_login(&self, _user_id: i64) -> Result<(), AuthError> {
        Ok(())
    }

    async fn delete_expired_sessions(&self) -> Result<u64, AuthError> {
        Ok(0)
    }

    async fn update_profile(
        &self,
        user_id: i64,
        profile: ProfileUpdate,
    ) -> Result<User, AuthError> {
        let mut data = self.data.lock().unwrap();
        let credentials = data.credentials.as_mut().ok_or(AuthError::NotFound)?;
        if credentials.user.id != user_id {
            return Err(AuthError::NotFound);
        }
        credentials.user.email = profile.email;
        credentials.user.display_name = profile.display_name;
        credentials.user.avatar_url = profile.avatar_url;
        Ok(credentials.user.clone())
    }

    async fn update_password(&self, user_id: i64, password_hash: &str) -> Result<(), AuthError> {
        let mut data = self.data.lock().unwrap();
        let credentials = data.credentials.as_mut().ok_or(AuthError::NotFound)?;
        if credentials.user.id != user_id {
            return Err(AuthError::NotFound);
        }
        credentials.password_hash = password_hash.to_owned();
        Ok(())
    }

    async fn create_password_reset(&self, reset: NewPasswordReset) -> Result<(), AuthError> {
        self.data
            .lock()
            .unwrap()
            .reset_tokens
            .insert(reset.token_hash, reset.user_id);
        Ok(())
    }

    async fn consume_password_reset(
        &self,
        token_hash: &[u8],
        password_hash: &str,
    ) -> Result<Option<i64>, AuthError> {
        let mut data = self.data.lock().unwrap();
        let Some(user_id) = data.reset_tokens.remove(token_hash) else {
            return Ok(None);
        };
        if let Some(credentials) = data.credentials.as_mut() {
            credentials.password_hash = password_hash.to_owned();
        }
        data.sessions.clear();
        Ok(Some(user_id))
    }

    async fn write_audit(&self, event: AuditEvent) -> Result<(), AuthError> {
        self.data.lock().unwrap().audits.push(event);
        Ok(())
    }

    async fn list_audit(&self, _query: AuditListQuery) -> Result<AuditPage, AuthError> {
        Err(AuthError::StoreUnavailable)
    }
}

struct MockPasswordHasher;

impl PasswordHasher for MockPasswordHasher {
    fn hash(&self, password: &str) -> Result<String, AuthError> {
        Ok(format!("hash:{password}"))
    }

    fn verify(&self, password: &str, password_hash: &str) -> Result<bool, AuthError> {
        Ok(password_hash == format!("hash:{password}"))
    }
}

/// 模拟 Legacy bcrypt Hash 的 Hasher：`legacy:` 前缀可验证但标记为待升级。
struct LegacyMockPasswordHasher;

impl PasswordHasher for LegacyMockPasswordHasher {
    fn hash(&self, password: &str) -> Result<String, AuthError> {
        Ok(format!("hash:{password}"))
    }

    fn verify(&self, password: &str, password_hash: &str) -> Result<bool, AuthError> {
        Ok(password_hash == format!("legacy:{password}")
            || password_hash == format!("hash:{password}"))
    }

    fn needs_rehash(&self, password_hash: &str) -> bool {
        password_hash.starts_with("legacy:")
    }
}

struct UnusedContentRepository;

#[async_trait]
impl ContentRepository for UnusedContentRepository {
    async fn create_article(&self, _article: NewArticle) -> Result<Article, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn reorder_articles(&self, _ordered_ids: Vec<i64>) -> Result<(), ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn move_article(
        &self,
        _article_id: i64,
        _direction: MoveDirection,
    ) -> Result<bool, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn find_article(&self, _article_id: i64) -> Result<Option<Article>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn find_published_article_by_slug(
        &self,
        _slug: &str,
    ) -> Result<Option<Article>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn find_article_by_slug(&self, _slug: &str) -> Result<Option<Article>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_public_articles(
        &self,
        _query: aries_core::content::PublicArticleQuery,
    ) -> Result<ArticlePage, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn search_public_articles(
        &self,
        _keyword: &str,
        _page: u32,
        _page_size: u32,
    ) -> Result<aries_core::content::SearchPage, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn search_suggest(
        &self,
        _keyword: &str,
        _limit: u32,
    ) -> Result<Vec<aries_core::content::SearchSuggestion>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn public_article_neighbors(
        &self,
        _article_id: i64,
    ) -> Result<aries_core::content::ArticleNeighbors, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn article_password_hash(
        &self,
        _article_id: i64,
    ) -> Result<Option<String>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn increment_visit_count(&self, _article_id: i64) -> Result<i64, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_public_categories(
        &self,
    ) -> Result<Vec<aries_core::content::CategorySummary>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_public_tags(&self) -> Result<Vec<aries_core::content::TagSummary>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn find_category_by_slug(
        &self,
        _kind: CategoryKind,
        _slug: &str,
    ) -> Result<Option<Category>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn find_tag_by_slug(&self, _slug: &str) -> Result<Option<Tag>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn find_category(&self, _category_id: i64) -> Result<Option<Category>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_tags_of_article(&self, _article_id: i64) -> Result<Vec<Tag>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_public_archives(
        &self,
    ) -> Result<Vec<aries_core::content::ArchiveMonth>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_articles(&self, _query: ArticleListQuery) -> Result<ArticlePage, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn update_article(
        &self,
        _article_id: i64,
        _update: ArticleUpdate,
    ) -> Result<Article, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn transition_article(
        &self,
        _article_id: i64,
        _expected_version: i64,
        _target: ArticleStatus,
        _operator_id: i64,
    ) -> Result<Article, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_categories(&self, _kind: CategoryKind) -> Result<Vec<Category>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn create_category(&self, _category: NewCategory) -> Result<Category, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn create_tag(&self, _tag: NewTag) -> Result<Tag, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn delete_article(&self, _article_id: i64) -> Result<(), ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn list_revisions(&self, _article_id: i64) -> Result<Vec<ArticleRevision>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn find_revision(
        &self,
        _article_id: i64,
        _revision_no: i64,
    ) -> Result<Option<ArticleRevision>, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn restore_revision(
        &self,
        _article_id: i64,
        _restore: RevisionRestore,
    ) -> Result<Article, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn update_category(
        &self,
        _category_id: i64,
        _kind: CategoryKind,
        _update: CategoryUpdate,
    ) -> Result<Category, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn delete_category(
        &self,
        _category_id: i64,
        _kind: CategoryKind,
    ) -> Result<(), ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn update_tag(&self, _tag_id: i64, _update: TagUpdate) -> Result<Tag, ContentError> {
        Err(ContentError::StoreUnavailable)
    }

    async fn delete_tag(&self, _tag_id: i64) -> Result<(), ContentError> {
        Err(ContentError::StoreUnavailable)
    }
}

struct PassthroughMarkdownRenderer;

impl MarkdownRenderer for PassthroughMarkdownRenderer {
    fn render(&self, source: &str) -> Result<String, ContentError> {
        Ok(source.to_owned())
    }
}

fn owner() -> User {
    User {
        id: 1,
        username: "owner".to_owned(),
        email: "owner@example.com".to_owned(),
        display_name: "Aries Owner".to_owned(),
        avatar_url: None,
        role: Role::Owner,
        status: UserStatus::Active,
    }
}

// ---- 媒体相关 Stub：本文件只测认证行为，媒体能力不需要真实实现 ----

use aries_core::jobs::{
    BackgroundJob, JobError, JobListQuery, JobPage, JobRepository, NewBackgroundJob,
};
use aries_core::media::{
    MediaAsset, MediaBatchDeleteResult, MediaError, MediaListQuery, MediaPage, MediaRepository,
    MediaStorage, MediaUpdate, MediaUsage, NewMediaAsset, SiteSettings, SiteSettingsRepository,
    SiteSettingsUpdate,
};

struct UnusedMediaRepository;

#[async_trait]
impl MediaRepository for UnusedMediaRepository {
    async fn create(&self, _asset: NewMediaAsset) -> Result<MediaAsset, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn find(&self, _asset_id: i64) -> Result<Option<MediaAsset>, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn list(&self, _query: MediaListQuery) -> Result<MediaPage, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn update(&self, _asset_id: i64, _update: MediaUpdate) -> Result<MediaAsset, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn soft_delete(&self, _asset_id: i64) -> Result<MediaAsset, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn batch_delete(&self, _asset_ids: &[i64]) -> Result<MediaBatchDeleteResult, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn find_by_hash(&self, _sha256: &str) -> Result<Option<MediaAsset>, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn find_by_object_keys(&self, _keys: &[String]) -> Result<Vec<MediaAsset>, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn usages_of(&self, _asset_id: i64) -> Result<Vec<MediaUsage>, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn replace_article_usages(
        &self,
        _article_id: i64,
        _cover_asset_id: Option<i64>,
        _content_asset_ids: &[i64],
    ) -> Result<(), MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn count_usages(&self, _asset_id: i64) -> Result<i64, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn list_missing_dimensions(&self, _limit: i64) -> Result<Vec<MediaAsset>, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn update_dimensions(
        &self,
        _asset_id: i64,
        _width: i32,
        _height: i32,
    ) -> Result<(), MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn list_purgeable(&self, _limit: i64) -> Result<Vec<MediaAsset>, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn purge(&self, _asset_id: i64) -> Result<(), MediaError> {
        Err(MediaError::StoreUnavailable)
    }
}

struct UnusedMediaStorage;

#[async_trait]
impl MediaStorage for UnusedMediaStorage {
    async fn put(&self, _key: &str, _bytes: &[u8], _mime: &str) -> Result<String, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn delete(&self, _key: &str) -> Result<(), MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    fn url_for(&self, key: &str) -> String {
        format!("/api/media/files/{key}")
    }

    fn serves_files_locally(&self) -> bool {
        true
    }
}

struct UnusedJobRepository;

#[async_trait]
impl JobRepository for UnusedJobRepository {
    async fn enqueue(&self, _job: NewBackgroundJob) -> Result<BackgroundJob, JobError> {
        Err(JobError::StoreUnavailable)
    }

    async fn find(&self, _job_id: i64) -> Result<Option<BackgroundJob>, JobError> {
        Err(JobError::StoreUnavailable)
    }

    async fn claim_next(&self) -> Result<Option<BackgroundJob>, JobError> {
        Err(JobError::StoreUnavailable)
    }

    async fn complete(&self, _job_id: i64) -> Result<(), JobError> {
        Err(JobError::StoreUnavailable)
    }

    async fn fail(&self, _job_id: i64, _error: &str) -> Result<BackgroundJob, JobError> {
        Err(JobError::StoreUnavailable)
    }

    async fn list(&self, _query: JobListQuery) -> Result<JobPage, JobError> {
        Err(JobError::StoreUnavailable)
    }
}

struct UnusedSiteSettingsRepository;

#[async_trait]
impl SiteSettingsRepository for UnusedSiteSettingsRepository {
    async fn get(&self) -> Result<SiteSettings, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn update(&self, _update: SiteSettingsUpdate) -> Result<SiteSettings, MediaError> {
        Err(MediaError::StoreUnavailable)
    }
}

struct UnusedCommentRepository;

#[async_trait]
impl aries_core::comments::CommentRepository for UnusedCommentRepository {
    async fn create(
        &self,
        _: aries_core::comments::NewComment,
    ) -> Result<aries_core::comments::Comment, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn create_admin_reply(
        &self,
        _: aries_core::comments::AdminReply,
    ) -> Result<aries_core::comments::Comment, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn find(
        &self,
        _: i64,
    ) -> Result<Option<aries_core::comments::Comment>, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn list(
        &self,
        _: aries_core::comments::CommentListQuery,
    ) -> Result<aries_core::comments::CommentPage, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn list_approved_by_target(
        &self,
        _: aries_core::comments::CommentTargetType,
        _: i64,
    ) -> Result<Vec<aries_core::comments::Comment>, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn change_status(
        &self,
        _: aries_core::comments::CommentStatusChange,
    ) -> Result<aries_core::comments::Comment, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn delete(&self, _: i64) -> Result<(), aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn count_approved(
        &self,
        _: aries_core::comments::CommentTargetType,
        _: i64,
    ) -> Result<i64, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn apply_ai_assessment(
        &self,
        _: aries_core::comments::AiAssessment,
    ) -> Result<aries_core::comments::Comment, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
    async fn dashboard_stats(
        &self,
    ) -> Result<aries_core::comments::DashboardStats, aries_core::comments::CommentError> {
        Err(aries_core::comments::CommentError::StoreUnavailable)
    }
}

fn test_app(repository: Arc<MockAuthRepository>) -> Router {
    test_app_with_hasher(repository, Arc::new(MockPasswordHasher))
}

fn test_app_with_hasher(
    repository: Arc<MockAuthRepository>,
    hasher: Arc<dyn PasswordHasher>,
) -> Router {
    let database = PgPoolOptions::new()
        .connect_lazy("postgres://test:test@127.0.0.1:1/test")
        .unwrap();
    build_app(AppState {
        database: database.clone(),
        auth: repository,
        content: Arc::new(UnusedContentRepository),
        markdown: Arc::new(PassthroughMarkdownRenderer),
        passwords: hasher,
        media: Arc::new(UnusedMediaRepository),
        storage: Arc::new(UnusedMediaStorage),
        jobs: Arc::new(UnusedJobRepository),
        site_settings: Arc::new(UnusedSiteSettingsRepository),
        comments: Arc::new(UnusedCommentRepository),
        // 以下端点不在本测试覆盖范围内，复用真实 Repository（连接为 lazy，不会实际查询）。
        pages: Arc::new(aries_infra::PostgresPageRepository::new(database.clone())),
        journals: Arc::new(aries_infra::PostgresJournalRepository::new(
            database.clone(),
        )),
        galleries: Arc::new(aries_infra::PostgresGalleryRepository::new(
            database.clone(),
        )),
        links: Arc::new(aries_infra::PostgresLinkRepository::new(database.clone())),
        logs: Arc::new(aries_infra::PostgresLogRepository::new(database.clone())),
        navigation: Arc::new(aries_infra::PostgresNavigationRepository::new(
            database.clone(),
        )),
        settings: Arc::new(aries_infra::PostgresSettingRepository::new(
            database.clone(),
        )),
        chunks: Arc::new(aries_infra::PostgresChunkRepository::new(database.clone())),
        ai: Arc::new(aries_infra::DispatchingAiProvider::new()),
        ai_requests: Arc::new(aries_infra::PostgresAiRequestRepository::new(database)),
        config: Arc::new(ServerConfig {
            address: "127.0.0.1:0".parse().unwrap(),
            admin_origins: vec![ORIGIN.to_owned(), "http://localhost:5173".to_owned()],
            bootstrap_secret: "test-bootstrap-secret-2026".to_owned(),
            session_ttl: Duration::hours(12),
            cookie_secure: false,
            media_public_base_url: "/api/media/files".to_owned(),
            media_provider: "local".to_owned(),
            slow_request_ms: 0,
            log_error_spike_threshold: 0,
            openapi_docs_enabled: false,
        }),
        rate_limiter: RateLimiter::default(),
        log_handle: Arc::new(aries_server::logging::LogHandle::noop()),
        log_wake: aries_server::log_store::log_wake_channel(),
    })
    .unwrap()
}

fn json_request(method: &str, uri: &str, payload: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, ORIGIN)
        .body(Body::from(payload.to_string()))
        .unwrap()
}

async fn response_body(response: Response<Body>) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

fn cookie_from(response: &Response<Body>) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn bootstrap_can_only_create_one_owner_and_sets_a_safe_cookie() {
    let repository = Arc::new(MockAuthRepository::default());
    let app = test_app(repository);
    let payload = serde_json::json!({
        "bootstrap_secret": "test-bootstrap-secret-2026",
        "username": "owner",
        "email": "owner@example.com",
        "display_name": "Aries Owner",
        "password": PASSWORD
    });

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/bootstrap",
            payload.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Path=/api/admin"));

    let second = app
        .oneshot(json_request("POST", "/api/admin/bootstrap", payload))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn login_session_restore_and_logout_form_a_complete_flow() {
    let app = test_app(Arc::new(MockAuthRepository::with_owner()));
    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/auth/login",
            serde_json::json!({ "login": "owner", "password": PASSWORD }),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = cookie_from(&login);

    let session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/auth/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::OK);
    assert!(response_body(session).await.contains("Aries Owner"));

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/auth/logout")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);

    let after_logout = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/auth/session")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_logout.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn legacy_password_hash_is_upgraded_after_successful_login() {
    let repository = Arc::new(MockAuthRepository::with_legacy_owner());
    let app = test_app_with_hasher(repository.clone(), Arc::new(LegacyMockPasswordHasher));

    let login = app
        .oneshot(json_request(
            "POST",
            "/api/admin/auth/login",
            serde_json::json!({ "login": "owner", "password": PASSWORD }),
        ))
        .await
        .unwrap();

    assert_eq!(login.status(), StatusCode::OK);
    assert_eq!(
        repository.stored_password_hash(),
        format!("hash:{PASSWORD}")
    );
}

#[tokio::test]
async fn unknown_user_and_wrong_password_return_the_same_error() {
    let app = test_app(Arc::new(MockAuthRepository::with_owner()));
    let wrong_password = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/auth/login",
            serde_json::json!({ "login": "owner", "password": "wrong-password-2026" }),
        ))
        .await
        .unwrap();
    let unknown_user = app
        .oneshot(json_request(
            "POST",
            "/api/admin/auth/login",
            serde_json::json!({ "login": "unknown", "password": "wrong-password-2026" }),
        ))
        .await
        .unwrap();

    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_user.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_body(wrong_password).await,
        response_body(unknown_user).await
    );
}

#[tokio::test]
async fn unsafe_requests_require_the_configured_origin() {
    let app = test_app(Arc::new(MockAuthRepository::with_owner()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "login": "owner", "password": PASSWORD }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(response_body(response).await.contains("INVALID_ORIGIN"));
}

#[tokio::test]
async fn unsafe_requests_accept_each_configured_origin() {
    let app = test_app(Arc::new(MockAuthRepository::with_owner()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(Body::from(
                    serde_json::json!({ "login": "owner", "password": PASSWORD }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn repeated_login_failures_are_rate_limited() {
    let app = test_app(Arc::new(MockAuthRepository::with_owner()));
    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/admin/auth/login",
                serde_json::json!({ "login": "owner", "password": "wrong-password-2026" }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let response = app
        .oneshot(json_request(
            "POST",
            "/api/admin/auth/login",
            serde_json::json!({ "login": "owner", "password": "wrong-password-2026" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    // 429 必须携带 Retry-After（登录限流窗口 15 分钟），值为正整数秒。
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .expect("429 response must carry Retry-After")
        .to_str()
        .unwrap()
        .parse::<u64>()
        .expect("Retry-After must be integer seconds");
    assert!(
        retry_after > 0 && retry_after <= 15 * 60,
        "unexpected Retry-After: {retry_after}"
    );
}

#[tokio::test]
async fn profile_update_and_password_change_revoke_the_current_session() {
    let app = test_app(Arc::new(MockAuthRepository::with_owner()));
    let login = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/auth/login",
            serde_json::json!({ "login": "owner", "password": PASSWORD }),
        ))
        .await
        .unwrap();
    let cookie = cookie_from(&login);

    let profile = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/admin/profile")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &cookie)
                .body(Body::from(
                    serde_json::json!({
                        "email": "updated@example.com",
                        "display_name": "Updated Owner",
                        "avatar_url": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(profile.status(), StatusCode::OK);
    assert!(response_body(profile).await.contains("Updated Owner"));

    let password = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/admin/profile/password")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ORIGIN, ORIGIN)
                .header(header::COOKIE, &cookie)
                .body(Body::from(
                    serde_json::json!({
                        "current_password": PASSWORD,
                        "new_password": "updated-pass-2026"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(password.status(), StatusCode::NO_CONTENT);

    let old_session = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/auth/session")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_session.status(), StatusCode::UNAUTHORIZED);

    let new_login = app
        .oneshot(json_request(
            "POST",
            "/api/admin/auth/login",
            serde_json::json!({ "login": "owner", "password": "updated-pass-2026" }),
        ))
        .await
        .unwrap();
    assert_eq!(new_login.status(), StatusCode::OK);
}
