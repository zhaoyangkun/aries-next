use std::sync::{Arc, Mutex};

use aries_core::{
    auth::{
        AuditEvent, AuditListQuery, AuditPage, AuthError, AuthRepository, AuthenticatedSession,
        CredentialUser, NewOwner, NewPasswordReset, NewSession, PasswordHasher, ProfileUpdate,
        Role, User, UserStatus,
    },
    content::{
        Article, ArticleListQuery, ArticlePage, ArticleRevision, ArticleStatus, ArticleUpdate,
        Category, CategoryKind, CategoryUpdate, ContentError, ContentRepository, NewArticle,
        NewCategory, NewTag, RevisionMetadata, RevisionRestore, Tag, TagUpdate,
    },
};
use aries_infra::ComrakMarkdownRenderer;
use aries_server::{build_app, config::ServerConfig, rate_limit::RateLimiter, state::AppState};
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, Response, StatusCode, header},
};
use http_body_util::BodyExt as _;
use sqlx::postgres::PgPoolOptions;
use time::{Duration, OffsetDateTime};
use tower::ServiceExt as _;
use uuid::Uuid;

const ORIGIN: &str = "http://127.0.0.1:5173";
const COOKIE: &str = "aries_admin_session=test-session";

struct MockAuthRepository {
    user: User,
    audits: Mutex<Vec<AuditEvent>>,
}

impl MockAuthRepository {
    fn new(role: Role) -> Self {
        Self {
            user: User {
                id: 1,
                username: "editor".to_owned(),
                email: "editor@example.com".to_owned(),
                display_name: "Aries Editor".to_owned(),
                avatar_url: None,
                role,
                status: UserStatus::Active,
            },
            audits: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl AuthRepository for MockAuthRepository {
    async fn is_bootstrapped(&self) -> Result<bool, AuthError> {
        Ok(true)
    }

    async fn create_owner(&self, _owner: NewOwner) -> Result<User, AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn find_credentials(&self, _login: &str) -> Result<Option<CredentialUser>, AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn find_user_by_email(&self, _email: &str) -> Result<Option<User>, AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn create_session(&self, _session: NewSession) -> Result<(), AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn find_session(
        &self,
        _token_hash: &[u8],
    ) -> Result<Option<AuthenticatedSession>, AuthError> {
        Ok(Some(AuthenticatedSession {
            id: Uuid::nil(),
            user: self.user.clone(),
            expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
        }))
    }

    async fn touch_session(&self, _session_id: Uuid) -> Result<(), AuthError> {
        Ok(())
    }

    async fn revoke_session(&self, _session_id: Uuid) -> Result<(), AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn revoke_user_sessions(&self, _user_id: i64) -> Result<(), AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn delete_expired_sessions(&self) -> Result<u64, AuthError> {
        Ok(0)
    }

    async fn update_last_login(&self, _user_id: i64) -> Result<(), AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn update_profile(
        &self,
        _user_id: i64,
        _profile: ProfileUpdate,
    ) -> Result<User, AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn update_password(&self, _user_id: i64, _password_hash: &str) -> Result<(), AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn create_password_reset(&self, _reset: NewPasswordReset) -> Result<(), AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn consume_password_reset(
        &self,
        _token_hash: &[u8],
        _password_hash: &str,
    ) -> Result<Option<i64>, AuthError> {
        Err(AuthError::StoreUnavailable)
    }

    async fn write_audit(&self, event: AuditEvent) -> Result<(), AuthError> {
        self.audits.lock().unwrap().push(event);
        Ok(())
    }

    async fn list_audit(&self, _query: AuditListQuery) -> Result<AuditPage, AuthError> {
        Err(AuthError::StoreUnavailable)
    }
}

#[derive(Default)]
struct MockContentRepository {
    articles: Mutex<Vec<Article>>,
    revisions: Mutex<Vec<ArticleRevision>>,
    categories: Mutex<Vec<Category>>,
    tags: Mutex<Vec<Tag>>,
}

/// 模拟 infra 的自动快照：在每次写入前记录当前版本，revision_no 即当时的 version。
fn snapshot(article: &Article) -> ArticleRevision {
    ArticleRevision {
        article_id: article.id,
        revision_no: article.version,
        markdown_source: article.markdown_source.clone(),
        metadata: RevisionMetadata {
            title: article.title.clone(),
            slug: article.slug.clone(),
            summary: article.summary.clone(),
            category_id: article.category_id,
            cover_url: article.cover_url.clone(),
            seo_keywords: article.seo_keywords.clone(),
            // Mock 只需要“是否设置密码”的语义，不保存真实 hash。
            access_password_hash: article.password_protected.then(|| "mock-hash".to_owned()),
            allow_comments: article.allow_comments,
            is_pinned: article.is_pinned,
            tag_ids: article.tag_ids.clone(),
        },
        operator_id: article.author_id,
        created_at: OffsetDateTime::now_utc(),
    }
}

#[async_trait]
impl ContentRepository for MockContentRepository {
    async fn create_article(&self, article: NewArticle) -> Result<Article, ContentError> {
        let now = OffsetDateTime::now_utc();
        let mut articles = self.articles.lock().unwrap();
        let id = articles.iter().map(|article| article.id).max().unwrap_or(0) + 1;
        let article = Article {
            id,
            author_id: article.author_id,
            category_id: article.category_id,
            status: ArticleStatus::Draft,
            // Mock 只做最小化 slug 规范化，满足断言即可。
            slug: article.slug.to_lowercase().replace(' ', "-"),
            title: article.title,
            summary: article.summary,
            cover_url: article.cover_url,
            markdown_source: article.markdown_source,
            rendered_html: article.rendered_html,
            seo_keywords: article.seo_keywords,
            tag_ids: article.tag_ids,
            password_protected: article.access_password_hash.is_some(),
            allow_comments: article.allow_comments,
            is_pinned: article.is_pinned,
            version: 1,
            visit_count: 0,
            comment_count: 0,
            published_at: None,
            created_at: now,
            updated_at: now,
        };
        articles.push(article.clone());
        Ok(article)
    }

    async fn find_article(&self, article_id: i64) -> Result<Option<Article>, ContentError> {
        Ok(self
            .articles
            .lock()
            .unwrap()
            .iter()
            .find(|article| article.id == article_id)
            .cloned())
    }

    async fn find_published_article_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<Article>, ContentError> {
        Ok(self
            .articles
            .lock()
            .unwrap()
            .iter()
            .find(|article| article.slug == slug && article.status == ArticleStatus::Published)
            .cloned())
    }

    async fn find_article_by_slug(&self, slug: &str) -> Result<Option<Article>, ContentError> {
        Ok(self
            .articles
            .lock()
            .unwrap()
            .iter()
            .find(|article| article.slug == slug)
            .cloned())
    }

    // ---- Public 只读方法：本文件只测 Admin 行为，全部返回 StoreUnavailable ----

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

    async fn list_articles(&self, query: ArticleListQuery) -> Result<ArticlePage, ContentError> {
        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, 100);
        let items = self.articles.lock().unwrap().clone();
        let total = i64::try_from(items.len()).map_err(|_| ContentError::StoreUnavailable)?;
        let start =
            usize::try_from(page - 1).unwrap_or(0) * usize::try_from(page_size).unwrap_or(0);
        let items = items
            .into_iter()
            .skip(start)
            .take(usize::try_from(page_size).unwrap_or(0))
            .collect();
        Ok(ArticlePage {
            total,
            items,
            page,
            page_size,
        })
    }

    async fn update_article(
        &self,
        article_id: i64,
        update: ArticleUpdate,
    ) -> Result<Article, ContentError> {
        let mut articles = self.articles.lock().unwrap();
        let article = articles
            .iter_mut()
            .find(|article| article.id == article_id)
            .ok_or(ContentError::NotFound)?;
        if article.version != update.expected_version {
            return Err(ContentError::Conflict);
        }
        self.revisions.lock().unwrap().push(snapshot(article));
        article.title = update.title;
        article.slug = update.slug;
        article.summary = update.summary;
        article.markdown_source = update.markdown_source;
        article.rendered_html = update.rendered_html;
        article.category_id = update.category_id;
        article.cover_url = update.cover_url;
        article.seo_keywords = update.seo_keywords;
        article.allow_comments = update.allow_comments;
        article.is_pinned = update.is_pinned;
        article.tag_ids = update.tag_ids;
        // 三层语义：None 不变、Some(None) 清除、Some(Some(_)) 设置。
        if let Some(password) = update.access_password_hash {
            article.password_protected = password.is_some();
        }
        article.version += 1;
        article.updated_at = OffsetDateTime::now_utc();
        Ok(article.clone())
    }

    async fn transition_article(
        &self,
        article_id: i64,
        expected_version: i64,
        target: ArticleStatus,
        _operator_id: i64,
    ) -> Result<Article, ContentError> {
        let mut articles = self.articles.lock().unwrap();
        let article = articles
            .iter_mut()
            .find(|article| article.id == article_id)
            .ok_or(ContentError::NotFound)?;
        if article.version != expected_version {
            return Err(ContentError::Conflict);
        }
        if !article.status.can_transition_to(target) {
            return Err(ContentError::InvalidTransition);
        }
        self.revisions.lock().unwrap().push(snapshot(article));
        article.status = target;
        article.version += 1;
        article.published_at = match target {
            ArticleStatus::Published => article
                .published_at
                .or_else(|| Some(OffsetDateTime::now_utc())),
            ArticleStatus::Draft => None,
            ArticleStatus::Recycled => article.published_at,
        };
        article.updated_at = OffsetDateTime::now_utc();
        Ok(article.clone())
    }

    async fn delete_article(&self, article_id: i64) -> Result<(), ContentError> {
        let mut articles = self.articles.lock().unwrap();
        let position = articles
            .iter()
            .position(|article| article.id == article_id)
            .ok_or(ContentError::NotFound)?;
        if articles[position].status != ArticleStatus::Recycled {
            return Err(ContentError::NotRecycled);
        }
        articles.remove(position);
        self.revisions
            .lock()
            .unwrap()
            .retain(|revision| revision.article_id != article_id);
        Ok(())
    }

    async fn list_revisions(&self, article_id: i64) -> Result<Vec<ArticleRevision>, ContentError> {
        let mut revisions = self
            .revisions
            .lock()
            .unwrap()
            .iter()
            .filter(|revision| revision.article_id == article_id)
            .cloned()
            .collect::<Vec<_>>();
        revisions.sort_by_key(|revision| std::cmp::Reverse(revision.revision_no));
        Ok(revisions)
    }

    async fn find_revision(
        &self,
        article_id: i64,
        revision_no: i64,
    ) -> Result<Option<ArticleRevision>, ContentError> {
        Ok(self
            .revisions
            .lock()
            .unwrap()
            .iter()
            .find(|revision| {
                revision.article_id == article_id && revision.revision_no == revision_no
            })
            .cloned())
    }

    async fn restore_revision(
        &self,
        article_id: i64,
        restore: RevisionRestore,
    ) -> Result<Article, ContentError> {
        let revision = self
            .find_revision(article_id, restore.revision_no)
            .await?
            .ok_or(ContentError::RevisionNotFound)?;
        let mut articles = self.articles.lock().unwrap();
        let article = articles
            .iter_mut()
            .find(|article| article.id == article_id)
            .ok_or(ContentError::NotFound)?;
        if article.version != restore.expected_version {
            return Err(ContentError::Conflict);
        }
        self.revisions.lock().unwrap().push(snapshot(article));
        article.title = revision.metadata.title;
        article.slug = revision.metadata.slug;
        article.summary = revision.metadata.summary;
        article.category_id = revision.metadata.category_id;
        article.cover_url = revision.metadata.cover_url;
        article.seo_keywords = revision.metadata.seo_keywords;
        article.allow_comments = revision.metadata.allow_comments;
        article.is_pinned = revision.metadata.is_pinned;
        article.tag_ids = revision.metadata.tag_ids;
        article.password_protected = revision.metadata.access_password_hash.is_some();
        article.markdown_source = revision.markdown_source;
        article.rendered_html = restore.rendered_html;
        article.version += 1;
        article.updated_at = OffsetDateTime::now_utc();
        Ok(article.clone())
    }

    async fn list_categories(&self, _kind: CategoryKind) -> Result<Vec<Category>, ContentError> {
        Ok(self.categories.lock().unwrap().clone())
    }

    async fn create_category(&self, category: NewCategory) -> Result<Category, ContentError> {
        let mut categories = self.categories.lock().unwrap();
        let category = Category {
            id: i64::try_from(categories.len() + 1).map_err(|_| ContentError::StoreUnavailable)?,
            parent_id: category.parent_id,
            kind: category.kind,
            name: category.name,
            slug: category.slug,
            description: category.description,
        };
        categories.push(category.clone());
        Ok(category)
    }

    async fn update_category(
        &self,
        category_id: i64,
        _kind: CategoryKind,
        update: CategoryUpdate,
    ) -> Result<Category, ContentError> {
        let mut categories = self.categories.lock().unwrap();
        let category = categories
            .iter_mut()
            .find(|category| category.id == category_id)
            .ok_or(ContentError::NotFound)?;
        category.name = update.name;
        category.slug = update.slug;
        Ok(category.clone())
    }

    async fn delete_category(
        &self,
        category_id: i64,
        _kind: CategoryKind,
    ) -> Result<(), ContentError> {
        let references = self
            .articles
            .lock()
            .unwrap()
            .iter()
            .filter(|article| article.category_id == Some(category_id))
            .count();
        if references > 0 {
            return Err(ContentError::Referenced {
                count: i64::try_from(references).map_err(|_| ContentError::StoreUnavailable)?,
            });
        }
        let mut categories = self.categories.lock().unwrap();
        let position = categories
            .iter()
            .position(|category| category.id == category_id)
            .ok_or(ContentError::NotFound)?;
        categories.remove(position);
        Ok(())
    }

    async fn list_tags(&self) -> Result<Vec<Tag>, ContentError> {
        Ok(self.tags.lock().unwrap().clone())
    }

    async fn create_tag(&self, tag: NewTag) -> Result<Tag, ContentError> {
        let mut tags = self.tags.lock().unwrap();
        let tag = Tag {
            id: i64::try_from(tags.len() + 1).map_err(|_| ContentError::StoreUnavailable)?,
            name: tag.name,
            slug: tag.slug,
        };
        tags.push(tag.clone());
        Ok(tag)
    }

    async fn update_tag(&self, tag_id: i64, update: TagUpdate) -> Result<Tag, ContentError> {
        let mut tags = self.tags.lock().unwrap();
        let tag = tags
            .iter_mut()
            .find(|tag| tag.id == tag_id)
            .ok_or(ContentError::NotFound)?;
        tag.name = update.name;
        tag.slug = update.slug;
        Ok(tag.clone())
    }

    async fn delete_tag(&self, tag_id: i64) -> Result<(), ContentError> {
        let references = self
            .articles
            .lock()
            .unwrap()
            .iter()
            .filter(|article| article.tag_ids.contains(&tag_id))
            .count();
        if references > 0 {
            return Err(ContentError::Referenced {
                count: i64::try_from(references).map_err(|_| ContentError::StoreUnavailable)?,
            });
        }
        let mut tags = self.tags.lock().unwrap();
        let position = tags
            .iter()
            .position(|tag| tag.id == tag_id)
            .ok_or(ContentError::NotFound)?;
        tags.remove(position);
        Ok(())
    }
}

struct MockPasswordHasher;

impl PasswordHasher for MockPasswordHasher {
    fn hash(&self, password: &str) -> Result<String, AuthError> {
        Ok(format!("argon2:{password}"))
    }

    fn verify(&self, password: &str, password_hash: &str) -> Result<bool, AuthError> {
        Ok(password_hash == format!("argon2:{password}"))
    }
}

// ---- 媒体相关 Stub：本文件只测文章行为，媒体引用同步需要“无引用”的最小实现 ----

use aries_core::jobs::{
    BackgroundJob, JobError, JobListQuery, JobPage, JobRepository, NewBackgroundJob,
};
use aries_core::media::{
    MediaAsset, MediaBatchDeleteResult, MediaError, MediaListQuery, MediaPage, MediaRepository,
    MediaStorage, MediaUpdate, MediaUsage, NewMediaAsset, SiteSettings, SiteSettingsRepository,
    SiteSettingsUpdate,
};

struct StubMediaRepository;

#[async_trait]
impl MediaRepository for StubMediaRepository {
    async fn create(&self, _asset: NewMediaAsset) -> Result<MediaAsset, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn find(&self, _asset_id: i64) -> Result<Option<MediaAsset>, MediaError> {
        Ok(None)
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
        Ok(None)
    }

    async fn find_by_object_keys(&self, _keys: &[String]) -> Result<Vec<MediaAsset>, MediaError> {
        Ok(Vec::new())
    }

    async fn usages_of(&self, _asset_id: i64) -> Result<Vec<MediaUsage>, MediaError> {
        Ok(Vec::new())
    }

    async fn replace_article_usages(
        &self,
        _article_id: i64,
        _cover_asset_id: Option<i64>,
        _content_asset_ids: &[i64],
    ) -> Result<(), MediaError> {
        Ok(())
    }

    async fn count_usages(&self, _asset_id: i64) -> Result<i64, MediaError> {
        Ok(0)
    }

    async fn list_missing_dimensions(&self, _limit: i64) -> Result<Vec<MediaAsset>, MediaError> {
        Ok(Vec::new())
    }

    async fn update_dimensions(
        &self,
        _asset_id: i64,
        _width: i32,
        _height: i32,
    ) -> Result<(), MediaError> {
        Ok(())
    }

    async fn list_purgeable(&self, _limit: i64) -> Result<Vec<MediaAsset>, MediaError> {
        Ok(Vec::new())
    }

    async fn purge(&self, _asset_id: i64) -> Result<(), MediaError> {
        Ok(())
    }
}

struct StubMediaStorage;

#[async_trait]
impl MediaStorage for StubMediaStorage {
    async fn put(&self, _key: &str, _bytes: &[u8], _mime: &str) -> Result<String, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, MediaError> {
        Ok(None)
    }

    async fn delete(&self, _key: &str) -> Result<(), MediaError> {
        Ok(())
    }

    fn url_for(&self, key: &str) -> String {
        format!("/api/media/files/{key}")
    }

    fn serves_files_locally(&self) -> bool {
        true
    }
}

struct StubJobRepository;

#[async_trait]
impl JobRepository for StubJobRepository {
    async fn enqueue(&self, _job: NewBackgroundJob) -> Result<BackgroundJob, JobError> {
        Err(JobError::StoreUnavailable)
    }

    async fn find(&self, _job_id: i64) -> Result<Option<BackgroundJob>, JobError> {
        Ok(None)
    }

    async fn claim_next(&self) -> Result<Option<BackgroundJob>, JobError> {
        Ok(None)
    }

    async fn complete(&self, _job_id: i64) -> Result<(), JobError> {
        Ok(())
    }

    async fn fail(&self, _job_id: i64, _error: &str) -> Result<BackgroundJob, JobError> {
        Err(JobError::StoreUnavailable)
    }

    async fn list(&self, _query: JobListQuery) -> Result<JobPage, JobError> {
        Err(JobError::StoreUnavailable)
    }
}

struct StubSiteSettingsRepository;

#[async_trait]
impl SiteSettingsRepository for StubSiteSettingsRepository {
    async fn get(&self) -> Result<SiteSettings, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn update(&self, _update: SiteSettingsUpdate) -> Result<SiteSettings, MediaError> {
        Err(MediaError::StoreUnavailable)
    }
}

struct StubCommentRepository;

#[async_trait]
impl aries_core::comments::CommentRepository for StubCommentRepository {
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

fn test_app(role: Role) -> Router {
    let database = PgPoolOptions::new()
        .connect_lazy("postgres://test:test@127.0.0.1:1/test")
        .unwrap();
    build_app(AppState {
        database: database.clone(),
        auth: Arc::new(MockAuthRepository::new(role)),
        content: Arc::new(MockContentRepository::default()),
        markdown: Arc::new(ComrakMarkdownRenderer),
        passwords: Arc::new(MockPasswordHasher),
        media: Arc::new(StubMediaRepository),
        storage: Arc::new(StubMediaStorage),
        jobs: Arc::new(StubJobRepository),
        site_settings: Arc::new(StubSiteSettingsRepository),
        comments: Arc::new(StubCommentRepository),
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
        // AI 端点不在本测试覆盖范围内，复用真实 Repository（连接为 lazy，不会实际查询）。
        ai: Arc::new(aries_infra::DispatchingAiProvider::new()),
        ai_requests: Arc::new(aries_infra::PostgresAiRequestRepository::new(database)),
        config: Arc::new(ServerConfig {
            address: "127.0.0.1:0".parse().unwrap(),
            admin_origins: vec![ORIGIN.to_owned()],
            bootstrap_secret: "test-bootstrap-secret-2026".to_owned(),
            session_ttl: Duration::hours(12),
            cookie_secure: false,
            media_public_base_url: "/api/media/files".to_owned(),
            media_provider: "local".to_owned(),
            slow_request_ms: 0,
            log_error_spike_threshold: 0,
        }),
        rate_limiter: RateLimiter::default(),
        log_handle: Arc::new(aries_server::logging::LogHandle::noop()),
    })
    .unwrap()
}

#[tokio::test]
async fn preview_create_and_list_form_a_real_admin_contract() {
    let app = test_app(Role::Editor);
    let create_category = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/categories",
            serde_json::json!({ "name": "Rust" }),
        ))
        .await
        .unwrap();
    assert_eq!(create_category.status(), StatusCode::CREATED);
    assert!(response_body(create_category).await.contains("Rust"));

    let create_tag = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/tags",
            serde_json::json!({ "name": "Vue 3" }),
        ))
        .await
        .unwrap();
    assert_eq!(create_tag.status(), StatusCode::CREATED);
    assert!(response_body(create_tag).await.contains("Vue 3"));

    let categories = app
        .clone()
        .oneshot(get_request("/api/admin/categories"))
        .await
        .unwrap();
    assert_eq!(categories.status(), StatusCode::OK);
    assert!(response_body(categories).await.contains("Rust"));

    let tags = app
        .clone()
        .oneshot(get_request("/api/admin/tags"))
        .await
        .unwrap();
    assert_eq!(tags.status(), StatusCode::OK);
    assert!(response_body(tags).await.contains("Vue 3"));

    let preview = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles/preview",
            serde_json::json!({
                "markdown_source": "# Preview\n\n<script>alert(1)</script>"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let preview_body = response_body(preview).await;
    assert!(preview_body.contains("<h1>Preview</h1>"));
    assert!(!preview_body.contains("script"));

    let create = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles",
            serde_json::json!({
                "title": "First post",
                "summary": "Stored in PostgreSQL in production",
                "markdown_source": "# First post",
                "category_id": 1,
                "tag_ids": [1]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let create_body = response_body(create).await;
    assert!(create_body.contains("first-post"));
    assert!(create_body.contains("rendered_html"));
    assert!(create_body.contains("\"tag_ids\":[1]"));

    let detail = app
        .clone()
        .oneshot(get_request("/api/admin/articles/1"))
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    assert!(response_body(detail).await.contains("# First post"));

    let update = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/admin/articles/1",
            serde_json::json!({
                "title": "First post revised",
                "summary": "Updated summary",
                "markdown_source": "# First post revised",
                "category_id": 1,
                "tag_ids": [1],
                "expected_version": 1
            }),
        ))
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);
    let update_body = response_body(update).await;
    assert!(update_body.contains("First post revised"));
    assert!(update_body.contains("\"version\":2"));

    let publish = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/articles/1/status",
            serde_json::json!({ "command": "publish", "expected_version": 2 }),
        ))
        .await
        .unwrap();
    assert_eq!(publish.status(), StatusCode::OK);
    let publish_body = response_body(publish).await;
    assert!(publish_body.contains("\"status\":\"published\""));
    assert!(publish_body.contains("\"version\":3"));

    let recycle = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/articles/1/status",
            serde_json::json!({ "command": "recycle", "expected_version": 3 }),
        ))
        .await
        .unwrap();
    assert_eq!(recycle.status(), StatusCode::OK);
    assert!(
        response_body(recycle)
            .await
            .contains("\"status\":\"recycled\"")
    );

    let recover = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/articles/1/status",
            serde_json::json!({ "command": "recover", "expected_version": 4 }),
        ))
        .await
        .unwrap();
    assert_eq!(recover.status(), StatusCode::OK);
    assert!(
        response_body(recover)
            .await
            .contains("\"status\":\"draft\"")
    );

    let list = app
        .oneshot(get_request("/api/admin/articles?page=1&page_size=20"))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = response_body(list).await;
    assert!(list_body.contains("First post"));
    assert!(list_body.contains("\"total\":1"));
}

#[tokio::test]
async fn moderator_cannot_manage_articles() {
    let response = test_app(Role::Moderator)
        .oneshot(get_request("/api/admin/articles"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn revisions_can_be_listed_and_restored() {
    let app = test_app(Role::Editor);
    let create = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles",
            serde_json::json!({
                "title": "First post",
                "markdown_source": "# First post"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);

    let update = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/admin/articles/1",
            serde_json::json!({
                "title": "First post revised",
                "markdown_source": "# First post revised",
                "expected_version": 1
            }),
        ))
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);

    let revisions = app
        .clone()
        .oneshot(get_request("/api/admin/articles/1/revisions"))
        .await
        .unwrap();
    assert_eq!(revisions.status(), StatusCode::OK);
    let revisions_body = response_body(revisions).await;
    assert!(revisions_body.contains("\"revision_no\":1"));
    assert!(revisions_body.contains("First post"));
    assert!(revisions_body.contains("# First post"));

    let restore = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles/1/revisions/1/restore",
            serde_json::json!({ "expected_version": 2 }),
        ))
        .await
        .unwrap();
    assert_eq!(restore.status(), StatusCode::OK);
    let restore_body = response_body(restore).await;
    assert!(restore_body.contains("\"title\":\"First post\""));
    assert!(restore_body.contains("\"version\":3"));

    // 恢复动作本身也会留下当前版本的 Revision。
    let revisions = app
        .clone()
        .oneshot(get_request("/api/admin/articles/1/revisions"))
        .await
        .unwrap();
    let revisions_body = response_body(revisions).await;
    assert!(revisions_body.contains("\"revision_no\":2"));

    let stale_restore = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles/1/revisions/1/restore",
            serde_json::json!({ "expected_version": 2 }),
        ))
        .await
        .unwrap();
    assert_eq!(stale_restore.status(), StatusCode::CONFLICT);
    assert!(
        response_body(stale_restore)
            .await
            .contains("ARTICLE_CONFLICT")
    );

    let missing_revision = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles/1/revisions/99/restore",
            serde_json::json!({ "expected_version": 3 }),
        ))
        .await
        .unwrap();
    assert_eq!(missing_revision.status(), StatusCode::NOT_FOUND);
    assert!(
        response_body(missing_revision)
            .await
            .contains("REVISION_NOT_FOUND")
    );

    let missing_article = app
        .oneshot(get_request("/api/admin/articles/999/revisions"))
        .await
        .unwrap();
    assert_eq!(missing_article.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_requires_recycled_status_and_is_final() {
    let app = test_app(Role::Editor);
    let create = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles",
            serde_json::json!({ "title": "Doomed post" }),
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);

    let early_delete = app
        .clone()
        .oneshot(json_request(
            "DELETE",
            "/api/admin/articles/1",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(early_delete.status(), StatusCode::CONFLICT);
    assert!(
        response_body(early_delete)
            .await
            .contains("ARTICLE_NOT_RECYCLED")
    );

    let recycle = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/articles/1/status",
            serde_json::json!({ "command": "recycle", "expected_version": 1 }),
        ))
        .await
        .unwrap();
    assert_eq!(recycle.status(), StatusCode::OK);

    let delete = app
        .clone()
        .oneshot(json_request(
            "DELETE",
            "/api/admin/articles/1",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let detail = app
        .clone()
        .oneshot(get_request("/api/admin/articles/1"))
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::NOT_FOUND);

    let delete_again = app
        .oneshot(json_request(
            "DELETE",
            "/api/admin/articles/1",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(delete_again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn taxonomy_write_operations_enforce_reference_protection() {
    let app = test_app(Role::Editor);
    for (uri, payload) in [
        (
            "/api/admin/categories",
            serde_json::json!({ "name": "Backend" }),
        ),
        ("/api/admin/tags", serde_json::json!({ "name": "Rust" })),
    ] {
        let response = app
            .clone()
            .oneshot(json_request("POST", uri, payload))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }
    let create = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles",
            serde_json::json!({
                "title": "Referenced post",
                "category_id": 1,
                "tag_ids": [1]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);

    let update_category = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/admin/categories/1",
            serde_json::json!({ "name": "Backend Core" }),
        ))
        .await
        .unwrap();
    assert_eq!(update_category.status(), StatusCode::OK);
    assert!(
        response_body(update_category)
            .await
            .contains("Backend Core")
    );

    let update_tag = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/admin/tags/1",
            serde_json::json!({ "name": "Rust Lang" }),
        ))
        .await
        .unwrap();
    assert_eq!(update_tag.status(), StatusCode::OK);
    assert!(response_body(update_tag).await.contains("Rust Lang"));

    // 引用保护：409 且 detail 中带引用数。
    for uri in ["/api/admin/categories/1", "/api/admin/tags/1"] {
        let delete = app
            .clone()
            .oneshot(json_request("DELETE", uri, serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::CONFLICT);
        let body = response_body(delete).await;
        assert!(body.contains("TAXONOMY_IN_USE"));
        assert!(body.contains("\"reference_count\":1"));
    }

    let recycle = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/articles/1/status",
            serde_json::json!({ "command": "recycle", "expected_version": 1 }),
        ))
        .await
        .unwrap();
    assert_eq!(recycle.status(), StatusCode::OK);
    let delete_article = app
        .clone()
        .oneshot(json_request(
            "DELETE",
            "/api/admin/articles/1",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(delete_article.status(), StatusCode::NO_CONTENT);

    for uri in ["/api/admin/categories/1", "/api/admin/tags/1"] {
        let delete = app
            .clone()
            .oneshot(json_request("DELETE", uri, serde_json::json!({})))
            .await
            .unwrap();
        assert_eq!(delete.status(), StatusCode::NO_CONTENT);
    }

    let delete_missing = app
        .oneshot(json_request(
            "DELETE",
            "/api/admin/categories/1",
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(delete_missing.status(), StatusCode::NOT_FOUND);
    assert!(
        response_body(delete_missing)
            .await
            .contains("TAXONOMY_NOT_FOUND")
    );
}

#[tokio::test]
async fn access_password_is_hashed_and_never_echoed() {
    let app = test_app(Role::Editor);
    let create = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/articles",
            serde_json::json!({
                "title": "Protected post",
                "access_password": "secret123"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let create_body = response_body(create).await;
    assert!(create_body.contains("\"password_protected\":true"));
    // 响应不得回显明文或哈希。
    assert!(!create_body.contains("secret123"));
    assert!(!create_body.contains("argon2"));

    // 字段缺省：密码保持不变。
    let update_untouched = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/admin/articles/1",
            serde_json::json!({
                "title": "Protected post",
                "expected_version": 1
            }),
        ))
        .await
        .unwrap();
    assert_eq!(update_untouched.status(), StatusCode::OK);
    assert!(
        response_body(update_untouched)
            .await
            .contains("\"password_protected\":true")
    );

    // 显式 null：清除密码。
    let update_cleared = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/admin/articles/1",
            serde_json::json!({
                "title": "Protected post",
                "access_password": null,
                "expected_version": 2
            }),
        ))
        .await
        .unwrap();
    assert_eq!(update_cleared.status(), StatusCode::OK);
    assert!(
        response_body(update_cleared)
            .await
            .contains("\"password_protected\":false")
    );

    // 字符串：设置新密码。
    let update_set = app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/admin/articles/1",
            serde_json::json!({
                "title": "Protected post",
                "access_password": "new-secret-2",
                "expected_version": 3
            }),
        ))
        .await
        .unwrap();
    assert_eq!(update_set.status(), StatusCode::OK);
    let update_set_body = response_body(update_set).await;
    assert!(update_set_body.contains("\"password_protected\":true"));
    assert!(!update_set_body.contains("new-secret-2"));

    // 过短或空白密码直接 400。
    for payload in [
        serde_json::json!({ "title": "Short", "access_password": "12345" }),
        serde_json::json!({ "title": "Blank", "access_password": "   " }),
    ] {
        let rejected = app
            .clone()
            .oneshot(json_request("POST", "/api/admin/articles", payload))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        assert!(
            response_body(rejected)
                .await
                .contains("INVALID_ACCESS_PASSWORD")
        );
    }
}

#[tokio::test]
async fn list_supports_sort_parameters_and_pagination_total() {
    let app = test_app(Role::Editor);
    for title in ["Alpha", "Beta", "Gamma"] {
        let create = app
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/admin/articles",
                serde_json::json!({ "title": title }),
            ))
            .await
            .unwrap();
        assert_eq!(create.status(), StatusCode::CREATED);
    }

    // 白名单内参数与非法参数（静默回退默认）都返回 200。
    for uri in [
        "/api/admin/articles?sort=title&order=asc",
        "/api/admin/articles?sort=published_at&order=desc",
        "/api/admin/articles?sort=bogus&order=sideways",
    ] {
        let list = app.clone().oneshot(get_request(uri)).await.unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        assert!(response_body(list).await.contains("\"total\":3"));
    }

    let second_page = app
        .oneshot(get_request("/api/admin/articles?page=2&page_size=2"))
        .await
        .unwrap();
    assert_eq!(second_page.status(), StatusCode::OK);
    let body = response_body(second_page).await;
    assert!(body.contains("\"total\":3"));
    assert!(body.contains("\"page\":2"));
    assert_eq!(body.matches("\"id\":").count(), 1);
}

fn json_request(method: &str, uri: &str, payload: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ORIGIN, ORIGIN)
        .header(header::COOKIE, COOKIE)
        .body(Body::from(payload.to_string()))
        .unwrap()
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::COOKIE, COOKIE)
        .body(Body::empty())
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
