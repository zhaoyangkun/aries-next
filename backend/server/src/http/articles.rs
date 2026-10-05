use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    content::{
        Article, ArticleListQuery, ArticleRevision, ArticleSort, ArticleStatus, ContentError,
        MoveDirection, NewArticle, RevisionRestore, SortOrder,
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
    extract::ApiJson,
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
        .route("/articles/{id}/move", put(move_article))
        .route("/articles/{id}/revisions", get(list_revisions))
        .route(
            "/articles/{id}/revisions/{rev}/restore",
            post(restore_revision),
        )
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
struct ArticleListParams {
    /// 页码，从 1 开始。
    #[serde(default = "super::default_page")]
    page: u32,
    /// 每页条数。
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 标题/摘要关键字过滤。
    keyword: Option<String>,
    /// 按状态过滤：`draft` / `published` / `recycled`。
    status: Option<String>,
    /// 按分类 ID 过滤。
    category_id: Option<i64>,
    /// 按标签 ID 过滤。
    tag_id: Option<i64>,
    /// 排序字段白名单；非法值静默回退 `updated_at`。`sort_order` 为手动排序值（「排序」模式的拖拽/箭头调整）。
    sort: Option<String>,
    /// 排序方向；非法值静默回退 `desc`。稳定 Tie-breaker 恒为 `id DESC`。
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct CreateArticleRequest {
    /// 文章标题（Trim 后非空，最长 200 字符）。
    title: String,
    /// URL 别名；缺省回退为标题。
    slug: Option<String>,
    /// 摘要，最长 500 字符。
    #[serde(default)]
    summary: String,
    /// AI 导读（TL;DR），由 `/api/admin/ai/editor/brief` 生成；不超过 500 字符。
    ai_brief: Option<String>,
    /// Markdown 原文，最长 1,000,000 字符。
    #[serde(default)]
    markdown_source: String,
    /// 所属分类 ID。
    category_id: Option<i64>,
    /// 封面图 URL。
    cover_url: Option<String>,
    /// SEO 关键字（Trim、排序、去重，最多 20 个，单个最长 50 字符）。
    #[serde(default)]
    seo_keywords: Vec<String>,
    /// 文章访问密码（明文，服务端以 Argon2id 哈希后存储，任何响应都不回显）。
    /// 新建时字段缺省或显式 `null` 都视为无密码；字符串表示设置新密码，
    /// Trim 后少于 6 个字符返回 400 `INVALID_ACCESS_PASSWORD`。
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    access_password: Option<Option<String>>,
    /// 是否允许评论，默认 `true`。
    #[serde(default = "default_true")]
    allow_comments: bool,
    /// 是否置顶，默认 `false`。
    #[serde(default)]
    is_pinned: bool,
    /// 关联标签 ID 列表。
    #[serde(default)]
    tag_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct UpdateArticleRequest {
    /// 文章标题（Trim 后非空，最长 200 字符）。
    title: String,
    /// URL 别名；缺省时保持现有值。
    slug: Option<String>,
    /// 摘要，最长 500 字符。
    #[serde(default)]
    summary: String,
    /// AI 导读（TL;DR）；三层语义：缺省不改动、显式 null 清除、字符串设置。
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    ai_brief: Option<Option<String>>,
    /// Markdown 原文，最长 1,000,000 字符。
    #[serde(default)]
    markdown_source: String,
    /// 所属分类 ID。
    category_id: Option<i64>,
    /// 封面图 URL。
    cover_url: Option<String>,
    /// SEO 关键字（Trim、排序、去重，最多 20 个，单个最长 50 字符）。
    #[serde(default)]
    seo_keywords: Vec<String>,
    /// 文章访问密码（明文，服务端以 Argon2id 哈希后存储，任何响应都不回显）。
    /// 三层语义：缺省不改动、显式 `null` 清除、字符串设置新密码（Trim 后少于 6 个字符返回 400）。
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
    access_password: Option<Option<String>>,
    /// 是否允许评论，默认 `true`。
    #[serde(default = "default_true")]
    allow_comments: bool,
    /// 是否置顶，默认 `false`。
    #[serde(default)]
    is_pinned: bool,
    /// 关联标签 ID 列表。
    #[serde(default)]
    tag_ids: Vec<i64>,
    /// 乐观锁版本号：必须等于当前 `version`，否则返回 409 `ARTICLE_CONFLICT`。
    expected_version: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct PreviewArticleRequest {
    /// Markdown 原文，最长 1,000,000 字符。
    markdown_source: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct RestoreRevisionRequest {
    /// 乐观锁版本号：必须等于当前 `version`，否则返回 409 `ARTICLE_CONFLICT`。
    expected_version: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct ChangeArticleStatusRequest {
    /// 状态流转命令：`publish`（发布，draft → published）、`recycle`（回收）、`recover`（还原为草稿）。
    command: ArticleStatusCommand,
    /// 乐观锁版本号：必须等于当前 `version`，否则返回 409 `ARTICLE_CONFLICT`。
    expected_version: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct ArticlePageResponse {
    /// 当前页文章列表。
    items: Vec<ArticleResponse>,
    /// 命中总数（稳定 Total 语义，与分页无关）。
    total: i64,
    /// 当前页码，从 1 开始。
    page: u32,
    /// 每页条数。
    page_size: u32,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct ArticleResponse {
    /// 文章 ID。
    id: i64,
    /// 作者用户 ID。
    author_id: i64,
    /// 所属分类 ID。
    category_id: Option<i64>,
    /// 文章状态：`draft` / `published` / `recycled`。
    #[schema(value_type = String, example = "published")]
    status: ArticleStatus,
    /// URL 别名。
    slug: String,
    /// 文章标题。
    title: String,
    /// 摘要。
    summary: String,
    /// AI 导读（TL;DR）；更新时字段缺省表示不改动，显式 null 清除。
    ai_brief: Option<String>,
    /// 封面图 URL。
    cover_url: Option<String>,
    /// Markdown 原文。
    markdown_source: String,
    /// 服务端渲染并 Sanitization 后的 HTML。
    rendered_html: String,
    /// SEO 关键字列表。
    seo_keywords: Vec<String>,
    /// 关联标签 ID 列表。
    tag_ids: Vec<i64>,
    /// 是否设置了访问密码（任何响应都不回显密码或哈希）。
    password_protected: bool,
    /// 是否允许评论。
    allow_comments: bool,
    /// 是否置顶。
    is_pinned: bool,
    /// 乐观锁版本号：每次更新递增。
    version: i64,
    /// 发布时间；草稿为 `null`。
    #[serde(with = "time::serde::rfc3339::option")]
    published_at: Option<time::OffsetDateTime>,
    /// 创建时间（RFC 3339）。
    #[serde(with = "time::serde::rfc3339")]
    created_at: time::OffsetDateTime,
    /// 更新时间（RFC 3339）。
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

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PreviewArticleResponse {
    /// 经过 `comrak` 渲染和 `ammonia` Sanitization 的 HTML。
    rendered_html: String,
}

/// Revision 响应不回显 `access_password`，只暴露是否受密码保护。
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct RevisionResponse {
    /// 版本号，从 1 开始递增。
    revision_no: i64,
    /// 当时标题。
    title: String,
    /// 当时 URL 别名。
    slug: String,
    /// 当时摘要。
    summary: String,
    /// 当时 AI 导读（TL;DR）。
    ai_brief: Option<String>,
    /// 当时分类 ID。
    category_id: Option<i64>,
    /// 当时封面图 URL。
    cover_url: Option<String>,
    /// 当时 SEO 关键字列表。
    seo_keywords: Vec<String>,
    /// 当时关联标签 ID 列表。
    tag_ids: Vec<i64>,
    /// 当时是否设置了访问密码（不回显密码哈希）。
    password_protected: bool,
    /// 当时是否允许评论。
    allow_comments: bool,
    /// 当时是否置顶。
    is_pinned: bool,
    /// 当时 Markdown 原文。
    markdown_source: String,
    /// 操作者用户 ID。
    operator_id: i64,
    /// 版本创建时间（RFC 3339）。
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

#[utoipa::path(
    get,
    path = "/api/admin/articles",
    tag = "Admin Articles",
    operation_id = "listArticles",
    summary = "分页查询文章列表",
    description = "返回具备稳定 Total 语义的 Article Page。非法排序参数静默回退默认值，列表接口不因排序参数报错。",
    security(("cookieAuth" = [])),
    params(ArticleListParams),
    responses(
        (status = 200, description = "返回具备稳定 Total 语义的 Article Page", body = ArticlePageResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
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

#[utoipa::path(
    get,
    path = "/api/admin/articles/{id}",
    tag = "Admin Articles",
    operation_id = "getArticle",
    summary = "获取文章详情",
    description = "返回 Article 的 Markdown 原文和当前版本。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "文章 ID")),
    responses(
        (status = 200, description = "返回 Article 的 Markdown 原文和当前版本", body = ArticleResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "文章不存在（ARTICLE_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
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

#[utoipa::path(
    post,
    path = "/api/admin/articles",
    tag = "Admin Articles",
    operation_id = "createArticle",
    summary = "创建文章（草稿）",
    description = "在 HTTP 层完成 Markdown 渲染，Draft 写入 PostgreSQL 并自动同步媒体引用。",
    security(("cookieAuth" = [])),
    request_body(content = CreateArticleRequest, content_type = "application/json"),
    responses(
        (status = 201, description = "Draft 已写入 PostgreSQL", body = ArticleResponse),
        (status = 400, description = "标题为空、内容超长、SEO 关键字非法或访问密码过短", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 409, description = "Slug 冲突", body = crate::openapi::ErrorResponse),
    )
)]
async fn create_article(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<CreateArticleRequest>,
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

#[utoipa::path(
    put,
    path = "/api/admin/articles/{id}",
    tag = "Admin Articles",
    operation_id = "updateArticle",
    summary = "更新文章",
    description = "乐观锁更新：expected_version 必须等于当前 version。`access_password` 与 `ai_brief` 三层语义：缺省不改动、显式 null 清除、字符串设置。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "文章 ID")),
    request_body(content = UpdateArticleRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "返回保存后的 Article 和新版本号", body = ArticleResponse),
        (status = 400, description = "标题为空、内容超长、SEO 关键字非法或访问密码过短", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "文章不存在（ARTICLE_NOT_FOUND）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "版本冲突或 Slug 冲突（ARTICLE_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_article(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(article_id): Path<i64>,
    ApiJson(request): ApiJson<UpdateArticleRequest>,
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

#[utoipa::path(
    patch,
    path = "/api/admin/articles/{id}/status",
    tag = "Admin Articles",
    operation_id = "changeStatus",
    summary = "流转文章状态",
    description = "通过 `publish` / `recycle` / `recover` 命令做状态机流转；乐观锁校验 expected_version。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "文章 ID")),
    request_body(content = ChangeArticleStatusRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "返回状态流转后的 Article 和新版本号", body = ArticleResponse),
        (status = 400, description = "非法状态迁移", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "文章不存在（ARTICLE_NOT_FOUND）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "版本冲突或非法状态迁移（ARTICLE_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
async fn change_status(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(article_id): Path<i64>,
    ApiJson(request): ApiJson<ChangeArticleStatusRequest>,
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

#[utoipa::path(
    post,
    path = "/api/admin/articles/preview",
    tag = "Admin Articles",
    operation_id = "previewArticle",
    summary = "预览 Markdown 渲染结果",
    description = "返回经过 `comrak` 渲染和 `ammonia` Sanitization 的 HTML，不落库。",
    security(("cookieAuth" = [])),
    request_body(content = PreviewArticleRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "返回渲染并 Sanitization 后的 HTML", body = PreviewArticleResponse),
        (status = 400, description = "内容超长（ARTICLE_CONTENT_TOO_LONG）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn preview_article(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<PreviewArticleRequest>,
) -> Result<Json<PreviewArticleResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    validate_markdown_length(&request.markdown_source)?;
    let rendered_html = render_markdown(&state, request.markdown_source).await?;
    Ok(Json(PreviewArticleResponse { rendered_html }))
}

#[utoipa::path(
    delete,
    path = "/api/admin/articles/{id}",
    tag = "Admin Articles",
    operation_id = "deleteArticle",
    summary = "物理删除文章",
    description = "物理删除文章及其 Revision 与 Tag 关联；仅 `recycled` 状态允许删除，否则返回 409 `ARTICLE_NOT_RECYCLED`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "文章 ID")),
    responses(
        (status = 204, description = "Article 已物理删除（无响应体）"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "文章不存在（ARTICLE_NOT_FOUND）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "文章非 recycled 状态（ARTICLE_NOT_RECYCLED）", body = crate::openapi::ErrorResponse),
    )
)]
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

#[utoipa::path(
    get,
    path = "/api/admin/articles/{id}/revisions",
    tag = "Admin Articles",
    operation_id = "listRevisions",
    summary = "查询文章 Revision 列表",
    description = "按 `revision_no` 倒序返回 Revision 列表；响应不含密码哈希，只暴露 `password_protected`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "文章 ID")),
    responses(
        (status = 200, description = "返回 Article 的 Revision 列表", body = Vec<RevisionResponse>),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "文章不存在（ARTICLE_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
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

#[utoipa::path(
    post,
    path = "/api/admin/articles/{id}/revisions/{rev}/restore",
    tag = "Admin Articles",
    operation_id = "restoreRevision",
    summary = "恢复指定 Revision",
    description = "将指定 Revision 的 Markdown 与元数据写回文章（`status`/`published_at` 不回滚）；恢复前会自动为当前版本留档新 Revision。版本冲突返回 409 `ARTICLE_CONFLICT`。",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "文章 ID"),
        ("rev" = i64, Path, description = "Revision 版本号，从 1 开始"),
    ),
    request_body(content = RestoreRevisionRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "返回恢复后的 Article 和新版本号", body = ArticleResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "Article 或 Revision 不存在（ARTICLE_NOT_FOUND / REVISION_NOT_FOUND）", body = crate::openapi::ErrorResponse),
        (status = 409, description = "版本冲突（ARTICLE_CONFLICT）", body = crate::openapi::ErrorResponse),
    )
)]
async fn restore_revision(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((article_id, revision_no)): Path<(i64, i64)>,
    ApiJson(request): ApiJson<RestoreRevisionRequest>,
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct ReorderArticlesRequest {
    /// 期望的新顺序（当前可视列表按展示顺序全量提交）。
    article_ids: Vec<i64>,
}

/// 批量重排文章手动排序值（下锚语义）：这批文章整体落到原 sort_order 槽位区间正前方的
/// 连续新区块，块内顺序即入参顺序；未入参文章（其他页/被过滤）的相对位置不受影响。
/// 典型 payload 是「排序」模式下当前页的完整有序 id 列表；上下箭头等价于提交两两交换后的列表。
#[utoipa::path(
    put,
    path = "/api/admin/articles/reorder",
    tag = "Admin Articles",
    operation_id = "reorderArticles",
    summary = "批量重排文章手动排序值",
    description = "批量重排文章手动排序值（`sort_order`，下锚语义）：这批文章整体落到原 `sort_order` 槽位区间正前方的连续新区块，块内顺序即入参顺序；未入参文章的相对位置不受影响。典型 payload 是「排序」模式下当前页的完整有序 id 列表（1–500 个，不得重复）；幂等，重复提交同一列表结果一致。",
    security(("cookieAuth" = [])),
    request_body(content = ReorderArticlesRequest, content_type = "application/json"),
    responses(
        (status = 204, description = "排序已更新（无响应体）"),
        (status = 400, description = "入参为空、含重复 id 或超过 500 个（INVALID_REORDER_INPUT）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "文章不存在（ARTICLE_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
async fn reorder_articles(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<ReorderArticlesRequest>,
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
enum MoveDirectionPayload {
    Up,
    Down,
    Top,
    Bottom,
}

impl MoveDirectionPayload {
    const fn to_domain(&self) -> MoveDirection {
        match self {
            Self::Up => MoveDirection::Up,
            Self::Down => MoveDirection::Down,
            Self::Top => MoveDirection::Top,
            Self::Bottom => MoveDirection::Bottom,
        }
    }

    const fn as_str(&self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct MoveArticleRequest {
    /// 移动方向：`up`/`down` 逐位移动，`top`/`bottom` 移至所在置顶分组的最前/最后。
    direction: MoveDirectionPayload,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct MoveArticleResponse {
    /// 是否实际发生了位置变化；`false` 表示文章已在置顶分组边界，位置未变。
    moved: bool,
}

/// 调整文章排序位置：在同一置顶分组内逐位交换（up/down）或直接移至分组
/// 最前/最后（top/bottom），适合「排序」模式的箭头跨页调整（reorder 只覆盖当前可视页）。
/// 已在分组边界时不做改动，响应体 `moved` 为 `false`，前端据此给出提示。
#[utoipa::path(
    put,
    path = "/api/admin/articles/{id}/move",
    tag = "Admin Articles",
    operation_id = "moveArticle",
    summary = "调整文章排序位置",
    description = "在同一置顶分组（置顶组/非置顶组）内调整排序位置：`up`/`down` 与相邻文章逐位交换，`top`/`bottom` 直接移至分组最前/最后（置顶组内 top 即全站最前，非置顶组 top 紧接置顶组之后）；已在分组边界时不做改动，响应体 `moved` 为 `false`。",
    security(("cookieAuth" = [])),
    params(("id" = i64, Path, description = "文章 ID")),
    request_body(content = MoveArticleRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "返回是否实际发生位置变化（分组边界 no-op 时 `moved` 为 false）", body = MoveArticleResponse),
        (status = 400, description = "请求体缺失或格式非法（direction 仅接受 up / down / top / bottom，INVALID_REQUEST_BODY）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无 content:manage 权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "文章不存在（ARTICLE_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
async fn move_article(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(article_id): Path<i64>,
    ApiJson(request): ApiJson<MoveArticleRequest>,
) -> Result<Json<MoveArticleResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let moved = state
        .content
        .move_article(article_id, request.direction.to_domain())
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "article.moved".to_owned(),
            target_type: "article".to_owned(),
            target_id: Some(article_id.to_string()),
            metadata: serde_json::json!({
                "direction": request.direction.as_str(),
                "moved": moved,
            }),
        })
        .await?;
    Ok(Json(MoveArticleResponse { moved }))
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
