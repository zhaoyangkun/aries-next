use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    content::{
        Article, ArticleListQuery, ArticleRevision, ArticleSort, ArticleStatus, ContentError,
        NewArticle, RevisionRestore, SortOrder,
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

use super::{
    auth::{CurrentUser, hash_password},
    error::ApiError,
};

const MAX_MARKDOWN_LENGTH: usize = 1_000_000;
const MIN_ACCESS_PASSWORD_LENGTH: usize = 6;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/articles", get(list_articles).post(create_article))
        .route("/articles/preview", post(preview_article))
        .route(
            "/articles/{id}",
            get(get_article).put(update_article).delete(delete_article),
        )
        .route("/articles/{id}/status", axum::routing::patch(change_status))
        .route("/articles/reorder", put(reorder_articles))
        .route("/articles/{id}/revisions", get(list_revisions))
        .route(
            "/articles/{id}/revisions/{rev}/restore",
            post(restore_revision),
        )
}

#[derive(Debug, Deserialize)]
struct ArticleListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    keyword: Option<String>,
    status: Option<String>,
    category_id: Option<i64>,
    tag_id: Option<i64>,
    sort: Option<String>,
    order: Option<String>,
}

/// 手写 double-option 反序列化：字段缺失走 `#[serde(default)]` 得到 `None`（不改动），
/// 显式 `null` 进入这里得到 `Some(None)`（清除），字符串得到 `Some(Some(_))`（设置）。
fn deserialize_nullable_string<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
struct CreateArticleRequest {
    title: String,
    slug: Option<String>,
    #[serde(default)]
    summary: String,
    ai_brief: Option<String>,
    #[serde(default)]
    markdown_source: String,
    category_id: Option<i64>,
    cover_url: Option<String>,
    #[serde(default)]
    seo_keywords: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    access_password: Option<Option<String>>,
    #[serde(default = "default_true")]
    allow_comments: bool,
    #[serde(default)]
    is_pinned: bool,
    #[serde(default)]
    tag_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
struct UpdateArticleRequest {
    title: String,
    slug: Option<String>,
    #[serde(default)]
    summary: String,
    /// 三层语义：缺省不改动、显式 null 清除、字符串设置（同 access_password）。
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    ai_brief: Option<Option<String>>,
    #[serde(default)]
    markdown_source: String,
    category_id: Option<i64>,
    cover_url: Option<String>,
    #[serde(default)]
    seo_keywords: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    access_password: Option<Option<String>>,
    #[serde(default = "default_true")]
    allow_comments: bool,
    #[serde(default)]
    is_pinned: bool,
    #[serde(default)]
    tag_ids: Vec<i64>,
    expected_version: i64,
}

#[derive(Debug, Deserialize)]
struct PreviewArticleRequest {
    markdown_source: String,
}

#[derive(Debug, Deserialize)]
struct RestoreRevisionRequest {
    expected_version: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArticleStatusCommand {
    Publish,
    Recycle,
    Recover,
}

impl ArticleStatusCommand {
    const fn target(&self) -> ArticleStatus {
        match self {
            Self::Publish => ArticleStatus::Published,
            Self::Recycle => ArticleStatus::Recycled,
            Self::Recover => ArticleStatus::Draft,
        }
    }

    const fn audit_action(&self) -> &'static str {
        match self {
            Self::Publish => "article.published",
            Self::Recycle => "article.recycled",
            Self::Recover => "article.recovered",
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChangeArticleStatusRequest {
    command: ArticleStatusCommand,
    expected_version: i64,
}

#[derive(Debug, Serialize)]
struct ArticlePageResponse {
    items: Vec<ArticleResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

#[derive(Debug, Serialize)]
struct ArticleResponse {
    id: i64,
    author_id: i64,
    category_id: Option<i64>,
    status: ArticleStatus,
    slug: String,
    title: String,
    summary: String,
    ai_brief: Option<String>,
    cover_url: Option<String>,
    markdown_source: String,
    rendered_html: String,
    seo_keywords: Vec<String>,
    tag_ids: Vec<i64>,
    password_protected: bool,
    allow_comments: bool,
    is_pinned: bool,
    version: i64,
    #[serde(with = "time::serde::rfc3339::option")]
    published_at: Option<time::OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: time::OffsetDateTime,
}

impl From<Article> for ArticleResponse {
    fn from(article: Article) -> Self {
        Self {
            id: article.id,
            author_id: article.author_id,
            category_id: article.category_id,
            status: article.status,
            slug: article.slug,
            title: article.title,
            summary: article.summary,
            ai_brief: article.ai_brief,
            cover_url: article.cover_url,
            markdown_source: article.markdown_source,
            rendered_html: article.rendered_html,
            seo_keywords: article.seo_keywords,
            tag_ids: article.tag_ids,
            password_protected: article.password_protected,
            allow_comments: article.allow_comments,
            is_pinned: article.is_pinned,
            version: article.version,
            published_at: article.published_at,
            created_at: article.created_at,
            updated_at: article.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct PreviewArticleResponse {
    rendered_html: String,
}

/// Revision 响应不回显 `access_password_hash`，只暴露是否受密码保护。
#[derive(Debug, Serialize)]
struct RevisionResponse {
    revision_no: i64,
    title: String,
    slug: String,
    summary: String,
    ai_brief: Option<String>,
    category_id: Option<i64>,
    cover_url: Option<String>,
    seo_keywords: Vec<String>,
    tag_ids: Vec<i64>,
    password_protected: bool,
    allow_comments: bool,
    is_pinned: bool,
    markdown_source: String,
    operator_id: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: time::OffsetDateTime,
}

impl From<ArticleRevision> for RevisionResponse {
    fn from(revision: ArticleRevision) -> Self {
        Self {
            revision_no: revision.revision_no,
            title: revision.metadata.title,
            slug: revision.metadata.slug,
            summary: revision.metadata.summary,
            ai_brief: revision.metadata.ai_brief,
            category_id: revision.metadata.category_id,
            cover_url: revision.metadata.cover_url,
            seo_keywords: revision.metadata.seo_keywords,
            tag_ids: revision.metadata.tag_ids,
            password_protected: revision.metadata.access_password_hash.is_some(),
            allow_comments: revision.metadata.allow_comments,
            is_pinned: revision.metadata.is_pinned,
            markdown_source: revision.markdown_source,
            operator_id: revision.operator_id,
            created_at: revision.created_at,
        }
    }
}

async fn list_articles(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<ArticleListParams>,
) -> Result<Json<ArticlePageResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let status = params
        .status
        .as_deref()
        .map(ArticleStatus::from_str)
        .transpose()?;
    let (sort, order) = parse_sort(params.sort.as_deref(), params.order.as_deref());
    let page = state
        .content
        .list_articles(ArticleListQuery {
            page: params.page,
            page_size: params.page_size,
            keyword: params.keyword,
            status,
            category_id: params.category_id,
            tag_id: params.tag_id,
            sort,
            order,
        })
        .await?;

    Ok(Json(ArticlePageResponse {
        items: page.items.into_iter().map(Into::into).collect(),
        total: page.total,
        page: page.page,
        page_size: page.page_size,
    }))
}

async fn get_article(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(article_id): Path<i64>,
) -> Result<Json<ArticleResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let article = state
        .content
        .find_article(article_id)
        .await?
        .ok_or(aries_core::content::ContentError::NotFound)?;
    Ok(Json(article.into()))
}

async fn create_article(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<CreateArticleRequest>,
) -> Result<(StatusCode, Json<ArticleResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let title = request.title.trim().to_owned();
    if title.is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_ARTICLE_TITLE",
            "Article title is required",
        ));
    }
    validate_markdown_length(&request.markdown_source)?;
    let rendered_html = render_markdown(&state, request.markdown_source.clone()).await?;
    // 新建时字段缺失或显式 null 都视为“无密码”，只有字符串才设置密码。
    let access_password_hash = match request.access_password.flatten() {
        Some(password) => Some(hash_access_password(&state, password).await?),
        None => None,
    };
    let slug = request
        .slug
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| title.clone());
    let article = state
        .content
        .create_article(NewArticle {
            author_id: current.user.id,
            category_id: request.category_id,
            slug,
            title,
            summary: request.summary.trim().to_owned(),
            ai_brief: request.ai_brief,
            cover_url: request.cover_url,
            markdown_source: request.markdown_source,
            rendered_html,
            seo_keywords: normalize_keywords(request.seo_keywords)?,
            access_password_hash,
            allow_comments: request.allow_comments,
            is_pinned: request.is_pinned,
            tag_ids: request.tag_ids,
        })
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "article.created".to_owned(),
            target_type: "article".to_owned(),
            target_id: Some(article.id.to_string()),
            metadata: serde_json::json!({ "status": article.status.as_str() }),
        })
        .await?;
    super::media::sync_article_media_usages(
        &state,
        article.id,
        &article.markdown_source,
        article.cover_url.as_deref(),
    )
    .await;

    Ok((StatusCode::CREATED, Json(article.into())))
}

async fn update_article(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(article_id): Path<i64>,
    Json(request): Json<UpdateArticleRequest>,
) -> Result<Json<ArticleResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let title = request.title.trim().to_owned();
    if title.is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_ARTICLE_TITLE",
            "Article title is required",
        ));
    }
    validate_markdown_length(&request.markdown_source)?;
    let existing = state
        .content
        .find_article(article_id)
        .await?
        .ok_or(aries_core::content::ContentError::NotFound)?;
    let rendered_html = render_markdown(&state, request.markdown_source.clone()).await?;
    // 三层语义：缺省不改动、显式 null 清除、字符串设置新密码（哈希后落库）。
    let access_password_hash = match request.access_password {
        Some(Some(password)) => Some(Some(hash_access_password(&state, password).await?)),
        Some(None) => Some(None),
        None => None,
    };
    let article = state
        .content
        .update_article(
            article_id,
            aries_core::content::ArticleUpdate {
                category_id: request.category_id,
                slug: request.slug.unwrap_or(existing.slug),
                title,
                summary: request.summary.trim().to_owned(),
                ai_brief: match request.ai_brief {
                    Some(value) => value,
                    None => existing.ai_brief.clone(),
                },
                cover_url: request.cover_url,
                markdown_source: request.markdown_source,
                rendered_html,
                seo_keywords: normalize_keywords(request.seo_keywords)?,
                access_password_hash,
                allow_comments: request.allow_comments,
                is_pinned: request.is_pinned,
                tag_ids: request.tag_ids,
                expected_version: request.expected_version,
                operator_id: current.user.id,
            },
        )
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "article.updated".to_owned(),
            target_type: "article".to_owned(),
            target_id: Some(article.id.to_string()),
            metadata: serde_json::json!({ "version": article.version }),
        })
        .await?;
    super::media::sync_article_media_usages(
        &state,
        article.id,
        &article.markdown_source,
        article.cover_url.as_deref(),
    )
    .await;
    enqueue_article_embed(&state, article.id).await;
    Ok(Json(article.into()))
}

/// 文章变更后入队 Embedding 任务：worker 内做开关与配置检查，
/// 未开启 smart_search 时任务静默完成，不入队反而会让开启后缺少存量数据。
async fn enqueue_article_embed(state: &AppState, article_id: i64) {
    if let Err(error) = state
        .jobs
        .enqueue(aries_core::jobs::NewBackgroundJob::new(
            aries_core::jobs::JobKind::ArticleEmbed,
            serde_json::json!({ "article_id": article_id }),
        ))
        .await
    {
        tracing::warn!(error = %error, article_id, "failed to enqueue article embed job");
    }
}

async fn change_status(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(article_id): Path<i64>,
    Json(request): Json<ChangeArticleStatusRequest>,
) -> Result<Json<ArticleResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let target = request.command.target();
    let article = state
        .content
        .transition_article(
            article_id,
            request.expected_version,
            target,
            current.user.id,
        )
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: request.command.audit_action().to_owned(),
            target_type: "article".to_owned(),
            target_id: Some(article.id.to_string()),
            metadata: serde_json::json!({
                "status": article.status.as_str(),
                "version": article.version,
            }),
        })
        .await?;
    enqueue_article_embed(&state, article.id).await;
    Ok(Json(article.into()))
}

async fn preview_article(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<PreviewArticleRequest>,
) -> Result<Json<PreviewArticleResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    validate_markdown_length(&request.markdown_source)?;
    let rendered_html = render_markdown(&state, request.markdown_source).await?;
    Ok(Json(PreviewArticleResponse { rendered_html }))
}

async fn delete_article(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(article_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    // 是否允许删除（仅 recycled 状态）由 Repository 在事务内判定。
    state.content.delete_article(article_id).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "article.deleted".to_owned(),
            target_type: "article".to_owned(),
            target_id: Some(article_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_revisions(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(article_id): Path<i64>,
) -> Result<Json<Vec<RevisionResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    state
        .content
        .find_article(article_id)
        .await?
        .ok_or(ContentError::NotFound)?;
    let revisions = state.content.list_revisions(article_id).await?;
    Ok(Json(revisions.into_iter().map(Into::into).collect()))
}

async fn restore_revision(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((article_id, revision_no)): Path<(i64, i64)>,
    Json(request): Json<RestoreRevisionRequest>,
) -> Result<Json<ArticleResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    current.require(Permission::ManageContent)?;
    state
        .content
        .find_article(article_id)
        .await?
        .ok_or(ContentError::NotFound)?;
    let revision = state
        .content
        .find_revision(article_id, revision_no)
        .await?
        .ok_or(ContentError::RevisionNotFound)?;
    // 与 update 流程一致：Markdown 渲染在 HTTP 层完成，Repository 只负责事务化写回。
    let rendered_html = render_markdown(&state, revision.markdown_source.clone()).await?;
    let article = state
        .content
        .restore_revision(
            article_id,
            RevisionRestore {
                revision_no,
                rendered_html,
                expected_version: request.expected_version,
                operator_id: current.user.id,
            },
        )
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "article.revision_restored".to_owned(),
            target_type: "article".to_owned(),
            target_id: Some(article.id.to_string()),
            metadata: serde_json::json!({
                "revision_no": revision_no,
                "version": article.version,
            }),
        })
        .await?;
    Ok(Json(article.into()))
}

#[derive(Debug, Deserialize)]
struct ReorderArticlesRequest {
    /// 期望的新顺序（当前可视列表按展示顺序全量提交）。
    article_ids: Vec<i64>,
}

/// 批量重排文章手动排序值（下锚语义）：这批文章整体落到原 sort_order 槽位区间正前方的
/// 连续新区块，块内顺序即入参顺序；未入参文章（其他页/被过滤）的相对位置不受影响。
/// 典型 payload 是「排序」模式下当前页的完整有序 id 列表；上下箭头等价于提交两两交换后的列表。
async fn reorder_articles(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(request): Json<ReorderArticlesRequest>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    let ids = request.article_ids;
    if ids.is_empty() || ids.len() > 500 {
        return Err(ApiError::bad_request(
            "INVALID_REORDER_INPUT",
            "Reorder list must contain 1 to 500 article ids",
        ));
    }
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != ids.len() {
        return Err(ApiError::bad_request(
            "INVALID_REORDER_INPUT",
            "Reorder list contains duplicate article ids",
        ));
    }
    state.content.reorder_articles(ids.clone()).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "article.reordered".to_owned(),
            target_type: "article".to_owned(),
            target_id: None,
            metadata: serde_json::json!({
                "count": ids.len(),
                "article_ids": ids,
            }),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// 非法排序参数静默回退默认值，列表接口不因排序参数报错。
fn parse_sort(sort: Option<&str>, order: Option<&str>) -> (ArticleSort, SortOrder) {
    let sort = match sort {
        Some("created_at") => ArticleSort::CreatedAt,
        Some("published_at") => ArticleSort::PublishedAt,
        Some("title") => ArticleSort::Title,
        Some("sort_order") => ArticleSort::SortOrder,
        _ => ArticleSort::UpdatedAt,
    };
    let order = match order {
        Some("asc") => SortOrder::Asc,
        _ => SortOrder::Desc,
    };
    (sort, order)
}

async fn hash_access_password(state: &AppState, password: String) -> Result<String, ApiError> {
    let password = password.trim().to_owned();
    // 空或过短密码直接报错，比静默当作“清除”更不容易误操作。
    if password.chars().count() < MIN_ACCESS_PASSWORD_LENGTH {
        return Err(ApiError::bad_request(
            "INVALID_ACCESS_PASSWORD",
            "Access password must contain at least 6 characters",
        ));
    }
    hash_password(state, password).await
}

pub(super) async fn render_markdown(state: &AppState, source: String) -> Result<String, ApiError> {
    let renderer = state.markdown.clone();
    tokio::task::spawn_blocking(move || renderer.render(&source))
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(Into::into)
}

fn validate_markdown_length(source: &str) -> Result<(), ApiError> {
    if source.chars().count() > MAX_MARKDOWN_LENGTH {
        return Err(ApiError::bad_request(
            "ARTICLE_CONTENT_TOO_LONG",
            "Article content is too long",
        ));
    }
    Ok(())
}

fn normalize_keywords(keywords: Vec<String>) -> Result<Vec<String>, ApiError> {
    let mut keywords = keywords
        .into_iter()
        .map(|keyword| keyword.trim().to_owned())
        .filter(|keyword| !keyword.is_empty())
        .collect::<Vec<_>>();
    keywords.sort();
    keywords.dedup();
    if keywords.len() > 20 || keywords.iter().any(|keyword| keyword.chars().count() > 50) {
        return Err(ApiError::bad_request(
            "INVALID_SEO_KEYWORDS",
            "SEO keywords are invalid",
        ));
    }
    Ok(keywords)
}

const fn default_true() -> bool {
    true
}
