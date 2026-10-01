//! 公开文章端点：列表、详情（密码解锁）、浏览计数、全文搜索与归档聚合。

use std::time::Duration;

use aries_core::content::{ArchiveMonth, Article, ArticleNeighbors, PublicArticleQuery};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_extra::extract::cookie::CookieJar;
use cookie::{Cookie, SameSite, time::Duration as CookieDuration};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::http::error::ApiError;
use crate::security::secrets_equal;
use crate::state::AppState;

use super::{
    CACHE_AGGREGATE, CACHE_ARTICLE, CACHE_NO_STORE, client_fingerprint, json_with_cache,
    reject_out_of_range_page,
};

/// 访问密码解锁 Cookie 有效期：短于 Session，降低凭据外泄后的暴露窗口。
const ACCESS_COOKIE_TTL: Duration = Duration::from_secs(2 * 60 * 60);
/// 解锁尝试限流：同一客户端对同一文章 5 次/分钟，防在线爆破。
const ACCESS_RATE_LIMIT: usize = 5;
const ACCESS_RATE_WINDOW: Duration = Duration::from_secs(60);
/// 浏览量去重窗口：同一客户端 30 分钟内重复访问只计一次（修正旧版每次 +1 的行为）。
const VIEW_DEDUP_WINDOW: Duration = Duration::from_secs(30 * 60);

type HmacSha256 = Hmac<Sha256>;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/articles", get(list_public_articles))
        .route("/articles/{slug}", get(get_public_article))
        .route("/articles/{slug}/access", post(unlock_article))
        .route("/articles/{slug}/views", post(record_article_view))
        .route("/search", get(search_public_articles))
        .route("/search/suggest", get(search_suggest))
        .route("/search/ask", post(search_ask))
        .route("/articles/{slug}/related", get(related_articles))
        .route("/archives", get(list_public_archives))
}

#[derive(Debug, Deserialize)]
pub struct PublicArticleListParams {
    #[serde(default = "crate::http::default_page")]
    page: u32,
    /// 缺省时取站点设置的 `page_size_index`。
    page_size: Option<u32>,
    keyword: Option<String>,
    category_id: Option<i64>,
    tag_id: Option<i64>,
    /// 按 Slug 过滤（新版统一 Slug；旧版标签按 name 的行为不继承）。
    category: Option<String>,
    tag: Option<String>,
}

/// Public 列表项剥离 `markdown_source`、`author_id`、`version` 等内部字段。
#[derive(Debug, Serialize)]
pub struct PublicArticleListItem {
    id: i64,
    slug: String,
    title: String,
    summary: String,
    cover_url: Option<String>,
    category_id: Option<i64>,
    tag_ids: Vec<i64>,
    is_pinned: bool,
    password_protected: bool,
    /// 搜索命中正文的纯文本片段（仅搜索接口在摘要未覆盖命中时返回；其他列表接口不出现该字段）。
    #[serde(skip_serializing_if = "Option::is_none")]
    matched_excerpt: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    published_at: Option<time::OffsetDateTime>,
}

impl From<&Article> for PublicArticleListItem {
    fn from(article: &Article) -> Self {
        Self {
            id: article.id,
            slug: article.slug.clone(),
            title: article.title.clone(),
            summary: article.summary.clone(),
            cover_url: article.cover_url.clone(),
            category_id: article.category_id,
            tag_ids: article.tag_ids.clone(),
            is_pinned: article.is_pinned,
            password_protected: article.password_protected,
            matched_excerpt: None,
            published_at: article.published_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PublicArticlePageResponse {
    pub items: Vec<PublicArticleListItem>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Serialize)]
struct TaxonomyRef {
    id: i64,
    name: String,
    slug: String,
}

#[derive(Debug, Serialize)]
struct NeighborRef {
    slug: String,
    title: String,
}

/// 公开详情：受密码保护且未解锁时 `rendered_html` 为 `null`；
/// `markdown_source` 绝不出现在公开响应中。
#[derive(Debug, Serialize)]
pub struct PublicArticleDetail {
    id: i64,
    slug: String,
    title: String,
    /// 摘要由作者显式撰写，视为公开元数据；正文 HTML 才需要解锁。
    summary: String,
    /// AI 导读（TL;DR）：发布时在管理端生成并随文保存；未生成时不出现该字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    ai_brief: Option<String>,
    cover_url: Option<String>,
    category: Option<TaxonomyRef>,
    tags: Vec<TaxonomyRef>,
    is_pinned: bool,
    password_protected: bool,
    allow_comments: bool,
    seo_keywords: Vec<String>,
    rendered_html: Option<String>,
    visit_count: i64,
    comment_count: i64,
    previous: Option<NeighborRef>,
    next: Option<NeighborRef>,
    #[serde(with = "time::serde::rfc3339::option")]
    published_at: Option<time::OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: time::OffsetDateTime,
}

#[derive(Debug, Deserialize)]
struct UnlockRequest {
    password: String,
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: Option<String>,
    #[serde(default = "crate::http::default_page")]
    page: u32,
}

#[derive(Debug, Deserialize)]
struct SuggestParams {
    q: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct SearchSuggestionResponse {
    slug: String,
    title: String,
}

#[derive(Debug, Serialize)]
struct ArchiveArticleResponse {
    slug: String,
    title: String,
    #[serde(with = "time::serde::rfc3339")]
    published_at: time::OffsetDateTime,
}

#[derive(Debug, Serialize)]
struct ArchiveMonthResponse {
    year: i32,
    month: i32,
    count: i64,
    articles: Vec<ArchiveArticleResponse>,
}

impl From<ArchiveMonth> for ArchiveMonthResponse {
    fn from(month: ArchiveMonth) -> Self {
        Self {
            year: month.year,
            month: month.month,
            count: month.count,
            articles: month
                .articles
                .into_iter()
                .map(|article| ArchiveArticleResponse {
                    slug: article.slug,
                    title: article.title,
                    published_at: article.published_at,
                })
                .collect(),
        }
    }
}

async fn list_public_articles(
    State(state): State<AppState>,
    Query(params): Query<PublicArticleListParams>,
) -> Result<Response, ApiError> {
    let page_size = match params.page_size {
        Some(value) => value.clamp(1, 100),
        None => {
            let settings = state.site_settings.get().await?;
            u32::try_from(settings.page_size_index.clamp(1, 100)).unwrap_or(10)
        }
    };
    let page = state
        .content
        .list_public_articles(PublicArticleQuery {
            page: params.page,
            page_size,
            keyword: params.keyword,
            category_id: params.category_id,
            tag_id: params.tag_id,
            category_slug: params.category,
            tag_slug: params.tag,
        })
        .await?;
    reject_out_of_range_page(page.page, page.items.len())?;
    Ok(json_with_cache(
        &PublicArticlePageResponse {
            items: page.items.iter().map(Into::into).collect(),
            total: page.total,
            page: page.page,
            page_size: page.page_size,
        },
        CACHE_ARTICLE,
    ))
}

async fn get_public_article(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    jar: CookieJar,
) -> Result<Response, ApiError> {
    let article = state
        .content
        .find_published_article_by_slug(&slug)
        .await?
        .ok_or_else(|| ApiError::not_found("ARTICLE_NOT_FOUND", "Article was not found"))?;

    // 密码文章只有携带有效解锁 Cookie 时才返回正文；Cookie 是 HMAC 凭据，无需服务端存储。
    let unlocked = if article.password_protected {
        let hash = state.content.article_password_hash(article.id).await?;
        hash.is_some_and(|hash| {
            jar.get(&access_cookie_name(article.id))
                .is_some_and(|cookie| {
                    secrets_equal(
                        cookie.value(),
                        &access_credential(&state.config.bootstrap_secret, article.id, &hash),
                    )
                })
        })
    } else {
        true
    };

    let category = match article.category_id {
        Some(category_id) => state.content.find_category(category_id).await?,
        None => None,
    };
    let tags = state.content.list_tags_of_article(article.id).await?;
    let neighbors = state.content.public_article_neighbors(article.id).await?;

    // 密码文章响应可能因 Cookie 不同而不同，必须禁止缓存。
    let cache = if article.password_protected {
        CACHE_NO_STORE_PRIVATE
    } else {
        CACHE_ARTICLE
    };
    let detail = PublicArticleDetail {
        rendered_html: unlocked.then(|| article.rendered_html.clone()),
        category: category.map(|category| TaxonomyRef {
            id: category.id,
            name: category.name,
            slug: category.slug,
        }),
        tags: tags
            .into_iter()
            .map(|tag| TaxonomyRef {
                id: tag.id,
                name: tag.name,
                slug: tag.slug,
            })
            .collect(),
        previous: neighbor_ref(&neighbors, true),
        next: neighbor_ref(&neighbors, false),
        id: article.id,
        slug: article.slug,
        title: article.title,
        summary: article.summary,
        ai_brief: article.ai_brief,
        cover_url: article.cover_url,
        is_pinned: article.is_pinned,
        password_protected: article.password_protected,
        allow_comments: article.allow_comments,
        seo_keywords: article.seo_keywords,
        visit_count: article.visit_count,
        comment_count: article.comment_count,
        published_at: article.published_at,
        created_at: article.created_at,
        updated_at: article.updated_at,
    };
    Ok(json_with_cache(&detail, cache))
}

/// 密码文章详情专用缓存策略：private, no-store。
const CACHE_NO_STORE_PRIVATE: &str = "private, no-store";

// ============================================================
// AI 检索：相关文章与对话式搜索（features.smart_search 开关，默认关闭）
// ============================================================

/// 对话式搜索限流：同一客户端 5 次/分钟 + 50 次/天（内存滑动窗口）。
const ASK_RATE_LIMIT_PER_MINUTE: usize = 5;
const ASK_RATE_WINDOW: Duration = Duration::from_secs(60);
const ASK_RATE_LIMIT_PER_DAY: usize = 50;
const ASK_DAY_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);
const ASK_MAX_QUESTION_CHARS: usize = 300;
const ASK_CHUNK_LIMIT: usize = 8;

#[derive(Debug, Deserialize)]
struct RelatedParams {
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct RelatedArticleResponse {
    slug: String,
    title: String,
    cover_url: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    published_at: Option<time::OffsetDateTime>,
}

impl From<aries_core::retrieval::RelatedArticle> for RelatedArticleResponse {
    fn from(article: aries_core::retrieval::RelatedArticle) -> Self {
        Self {
            slug: article.slug,
            title: article.title,
            cover_url: article.cover_url,
            published_at: article.published_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AskRequest {
    question: String,
}

/// 对话式搜索引用的来源文章。
#[derive(Debug, Clone, Serialize)]
struct AskSource {
    slug: String,
    title: String,
}

/// 加载 AI 设置并做检索门禁：全局未启用或未开 smart_search 一律 404
/// （开关默认关闭，未开启时不暴露 AI 能力的存在）。
async fn smart_search_settings(state: &AppState) -> Result<aries_core::ai::AiSettings, ApiError> {
    let record = state
        .settings
        .get_group(aries_core::settings::SettingGroup::Ai)
        .await?;
    let settings: aries_core::ai::AiSettings =
        serde_json::from_value(record.payload).map_err(|_| ApiError::internal())?;
    if !settings.enabled || !settings.features.smart_search {
        return Err(ApiError::not_found(
            "AI_RETRIEVAL_DISABLED",
            "AI retrieval is not enabled",
        ));
    }
    Ok(settings)
}

/// 相关文章：以文章标题 + 摘要为查询向量，取相近的已发布文章。
/// 文章无已发布状态或 AI 检索未开启时返回 404，空结果返回 200 空数组。
async fn related_articles(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<RelatedParams>,
) -> Result<Response, ApiError> {
    let settings = smart_search_settings(&state).await?;
    if !settings.is_embedding_configured() {
        return Err(ApiError::not_found(
            "AI_RETRIEVAL_DISABLED",
            "AI retrieval is not enabled",
        ));
    }
    let article = state
        .content
        .find_published_article_by_slug(&slug)
        .await?
        .ok_or_else(|| ApiError::not_found("ARTICLE_NOT_FOUND", "Article was not found"))?;
    let limit = params.limit.unwrap_or(6).clamp(1, 12);

    let query = format!("{}\n{}", article.title, article.summary);
    let vectors = state
        .ai
        .embed(&settings, &[query])
        .await
        .map_err(|_| ApiError::bad_gateway("AI_PROVIDER_FAILED", "AI provider error"))?;
    let Some(vector) = vectors.into_iter().next() else {
        return Err(ApiError::bad_gateway(
            "AI_PROVIDER_FAILED",
            "AI provider error",
        ));
    };
    let related = state
        .chunks
        .nearest_articles(&vector, article.id, limit)
        .await
        .map_err(|_| ApiError::internal())?;
    let body: Vec<RelatedArticleResponse> = related.into_iter().map(Into::into).collect();
    Ok(json_with_cache(&body, CACHE_ARTICLE))
}

// ------------------------------------------------------------
// 对话式搜索（SSE）
// ------------------------------------------------------------

/// ask 流事件：与编辑器助手同一套事件名（start/delta/usage/done/error），
/// 额外新增 `sources` 事件（done 之前发出，携带引用文章列表）。
enum AskOut {
    Start,
    Delta(String),
    Sources(Vec<AskSource>),
    Usage(aries_core::ai::AiUsage),
    Done,
    Error(&'static str),
}

impl AskOut {
    fn into_event(self, model: &str) -> axum::response::sse::Event {
        use axum::response::sse::Event;
        match self {
            Self::Start => Event::default().event("start").data(
                serde_json::json!({
                    "feature": aries_core::ai::AiFeature::SearchAsk.as_str(),
                    "model": model,
                    "prompt_version": aries_core::ai_prompts::PROMPT_VERSION,
                })
                .to_string(),
            ),
            Self::Delta(text) => Event::default()
                .event("delta")
                .data(serde_json::json!({ "text": text }).to_string()),
            Self::Sources(items) => Event::default()
                .event("sources")
                .data(serde_json::json!({ "items": items }).to_string()),
            Self::Usage(usage) => Event::default()
                .event("usage")
                .data(serde_json::to_string(&usage).unwrap_or_default()),
            Self::Done => Event::default().event("done").data("{}"),
            Self::Error(code) => Event::default()
                .event("error")
                .data(serde_json::json!({ "code": code }).to_string()),
        }
    }
}

async fn search_ask(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AskRequest>,
) -> Result<Response, ApiError> {
    let question = request.question.trim().to_owned();
    if question.is_empty() || question.chars().count() > ASK_MAX_QUESTION_CHARS {
        return Err(ApiError::bad_request(
            "INVALID_AI_INPUT",
            "Question length is invalid",
        ));
    }
    let settings = smart_search_settings(&state).await?;
    if !settings.is_configured() {
        return Err(ApiError::not_found(
            "AI_RETRIEVAL_DISABLED",
            "AI retrieval is not enabled",
        ));
    }
    // 限流先于 Provider 调用：匿名端点按客户端指纹计数，避免被刷 Token 费用。
    let fingerprint = client_fingerprint(&headers);
    for (limit, window) in [
        (ASK_RATE_LIMIT_PER_MINUTE, ASK_RATE_WINDOW),
        (ASK_RATE_LIMIT_PER_DAY, ASK_DAY_WINDOW),
    ] {
        let decision = state
            .rate_limiter
            .check(&format!("ai-ask:{fingerprint}"), limit, window);
        if !decision.allowed() {
            return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
        }
    }

    // 检索：配置 embedding 时取向量近邻内容块；未配置时降级为关键词搜索命中文章摘要。
    let (context, sources) = if settings.is_embedding_configured() {
        let vectors = state
            .ai
            .embed(&settings, std::slice::from_ref(&question))
            .await
            .map_err(|_| ApiError::bad_gateway("AI_PROVIDER_FAILED", "AI provider error"))?;
        let Some(vector) = vectors.into_iter().next() else {
            return Err(ApiError::bad_gateway(
                "AI_PROVIDER_FAILED",
                "AI provider error",
            ));
        };
        let chunks = state
            .chunks
            .nearest_chunks(&vector, ASK_CHUNK_LIMIT)
            .await
            .map_err(|_| ApiError::internal())?;
        let mut context = String::new();
        let mut sources: Vec<AskSource> = Vec::new();
        for (index, chunk) in chunks.iter().enumerate() {
            context.push_str(&format!(
                "[{}] 《{}》{}：{}\n\n",
                index + 1,
                chunk.article_title,
                if chunk.heading.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", chunk.heading)
                },
                chunk.content,
            ));
            if !sources.iter().any(|s| s.slug == chunk.article_slug) {
                sources.push(AskSource {
                    slug: chunk.article_slug.clone(),
                    title: chunk.article_title.clone(),
                });
            }
        }
        (context, sources)
    } else {
        let hits = state
            .content
            .search_public_articles(&question, 1, 4)
            .await?;
        let mut context = String::new();
        let mut sources = Vec::new();
        for (index, hit) in hits.items.iter().enumerate() {
            context.push_str(&format!(
                "[{}] 《{}》摘要：{}\n\n",
                index + 1,
                hit.article.title,
                hit.article.summary,
            ));
            sources.push(AskSource {
                slug: hit.article.slug.clone(),
                title: hit.article.title.clone(),
            });
        }
        (context, sources)
    };

    let messages = aries_core::ai_prompts::search_ask_messages(&question, &context);
    let model = settings.model.clone().unwrap_or_default();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(64);
    tokio::spawn(run_ask_stream(
        state, settings, messages, sources, model, tx,
    ));
    Ok(
        axum::response::sse::Sse::new(futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx)))
            .into_response(),
    )
}

/// 驱动 ask 流：start → delta 转发 → sources → usage/done → 落审计。
async fn run_ask_stream(
    state: AppState,
    settings: aries_core::ai::AiSettings,
    messages: Vec<aries_core::ai::AiMessage>,
    sources: Vec<AskSource>,
    model: String,
    sender: tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
) {
    let started = std::time::Instant::now();
    let send = |event: AskOut| {
        let sender = sender.clone();
        let model = model.clone();
        async move { sender.send(Ok(event.into_event(&model))).await.is_ok() }
    };

    if !send(AskOut::Start).await {
        return;
    }

    let (inner_tx, mut inner_rx) = tokio::sync::mpsc::channel::<aries_core::ai::AiStreamEvent>(64);
    let provider = state.ai.clone();
    let provider_handle = tokio::spawn(async move {
        provider
            .chat_stream(
                &settings,
                aries_core::ai::AiChatRequest {
                    messages,
                    ..aries_core::ai::AiChatRequest::default()
                },
                inner_tx,
            )
            .await
    });

    let mut usage: Option<aries_core::ai::AiUsage> = None;
    let mut failure: Option<aries_core::ai::AiError> = None;
    let mut cancelled = false;
    while let Some(event) = inner_rx.recv().await {
        let out = match event {
            aries_core::ai::AiStreamEvent::Delta(delta) => AskOut::Delta(delta),
            aries_core::ai::AiStreamEvent::Usage(value) => {
                usage = Some(value);
                AskOut::Usage(value)
            }
            aries_core::ai::AiStreamEvent::Done => break,
        };
        if !send(out).await {
            cancelled = true;
            provider_handle.abort();
            break;
        }
    }
    if !cancelled && failure.is_none() {
        match provider_handle.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failure = Some(error),
            Err(_) => failure = Some(aries_core::ai::AiError::ProviderFailed),
        }
    }

    let (status, terminal) = if cancelled {
        (aries_core::ai::AiRequestStatus::Cancelled, None)
    } else if let Some(error) = &failure {
        let code = match error {
            aries_core::ai::AiError::Timeout => "AI_PROVIDER_TIMEOUT",
            aries_core::ai::AiError::RateLimited => "AI_RATE_LIMITED",
            _ => "AI_PROVIDER_FAILED",
        };
        (
            aries_core::ai::AiRequestStatus::Failed,
            Some(AskOut::Error(code)),
        )
    } else {
        (aries_core::ai::AiRequestStatus::Success, Some(AskOut::Done))
    };
    // 来源列表在成功/失败的收尾前统一发出（done 之前），前端据此渲染引用。
    if !send(AskOut::Sources(sources)).await {
        return;
    }
    if let Some(event) = terminal {
        let _ = send(event).await;
    }

    let usage = usage.unwrap_or_default();
    if let Err(error) = state
        .ai_requests
        .record(aries_core::ai::NewAiRequest {
            feature: aries_core::ai::AiFeature::SearchAsk,
            operator_user_id: None,
            model,
            status,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            latency_ms: i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX),
            error_category: failure.as_ref().map(|error| error.category().to_owned()),
        })
        .await
    {
        tracing::warn!(error = %error, "failed to record ai request");
    }
}

fn neighbor_ref(neighbors: &ArticleNeighbors, previous: bool) -> Option<NeighborRef> {
    let neighbor = if previous {
        neighbors.previous.as_ref()
    } else {
        neighbors.next.as_ref()
    };
    neighbor.map(|value| NeighborRef {
        slug: value.slug.clone(),
        title: value.title.clone(),
    })
}

async fn unlock_article(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UnlockRequest>,
) -> Result<Response, ApiError> {
    let article = state
        .content
        .find_published_article_by_slug(&slug)
        .await?
        .ok_or_else(|| ApiError::not_found("ARTICLE_NOT_FOUND", "Article was not found"))?;

    // 限流在密码校验之前：错误与正确尝试都计数，避免通过响应差异探测限流边界。
    let rate_key = format!(
        "article-access:{}:{}",
        article.id,
        client_fingerprint(&headers)
    );
    let decision = state
        .rate_limiter
        .check(&rate_key, ACCESS_RATE_LIMIT, ACCESS_RATE_WINDOW);
    if !decision.allowed() {
        return Err(ApiError::rate_limited_retry_after(decision.retry_after()));
    }

    let Some(password_hash) = state.content.article_password_hash(article.id).await? else {
        return Err(ApiError::bad_request(
            "ARTICLE_NOT_PROTECTED",
            "Article is not password protected",
        ));
    };
    let verified = state
        .passwords
        .verify(request.password.trim(), &password_hash)
        .map_err(|_| ApiError::internal())?;
    if !verified {
        return Err(ApiError::unauthorized_with(
            "INVALID_ARTICLE_PASSWORD",
            "Article access password is incorrect",
        ));
    }

    let credential = access_credential(&state.config.bootstrap_secret, article.id, &password_hash);
    let cookie = Cookie::build((access_cookie_name(article.id), credential))
        .path(format!("/api/public/articles/{slug}"))
        .http_only(true)
        .secure(state.config.cookie_secure)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::seconds(ACCESS_COOKIE_TTL.as_secs() as i64))
        .build();
    let mut response = Json(serde_json::json!({ "unlocked": true })).into_response();
    if let Ok(value) = cookie.to_string().parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static(CACHE_NO_STORE),
    );
    Ok(response)
}

async fn record_article_view(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let article = state
        .content
        .find_published_article_by_slug(&slug)
        .await?
        .ok_or_else(|| ApiError::not_found("ARTICLE_NOT_FOUND", "Article was not found"))?;
    // 窗口内重复访问直接返回当前计数，不落库。
    let dedup_key = format!(
        "article-view:{}:{}",
        article.id,
        client_fingerprint(&headers)
    );
    let count = if state
        .rate_limiter
        .check(&dedup_key, 1, VIEW_DEDUP_WINDOW)
        .allowed()
    {
        state.content.increment_visit_count(article.id).await?
    } else {
        article.visit_count
    };
    Ok(json_with_cache(
        &serde_json::json!({ "visit_count": count }),
        CACHE_NO_STORE,
    ))
}

async fn search_public_articles(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Response, ApiError> {
    let keyword = params.q.unwrap_or_default().trim().to_owned();
    if keyword.is_empty() || keyword.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "INVALID_SEARCH_KEYWORD",
            "Search keyword is required",
        ));
    }
    let settings = state.site_settings.get().await?;
    let page_size = u32::try_from(settings.page_size_search.clamp(1, 100)).unwrap_or(10);
    let page = state
        .content
        .search_public_articles(&keyword, params.page, page_size)
        .await?;
    reject_out_of_range_page(page.page, page.items.len())?;
    Ok(json_with_cache(
        &PublicArticlePageResponse {
            items: page
                .items
                .iter()
                .map(|hit| {
                    let mut item = PublicArticleListItem::from(&hit.article);
                    item.matched_excerpt = hit.snippet.clone();
                    item
                })
                .collect(),
            total: page.total,
            page: page.page,
            page_size: page.page_size,
        },
        CACHE_ARTICLE,
    ))
}

async fn search_suggest(
    State(state): State<AppState>,
    Query(params): Query<SuggestParams>,
) -> Result<Response, ApiError> {
    let keyword = params.q.unwrap_or_default().trim().to_owned();
    if keyword.is_empty() || keyword.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "INVALID_SEARCH_KEYWORD",
            "Search keyword is required",
        ));
    }
    let limit = params.limit.unwrap_or(8).clamp(1, 20);
    // 输入即搜随 keystroke 高频触发，响应不可缓存
    let suggestions = state.content.search_suggest(&keyword, limit).await?;
    let body: Vec<SearchSuggestionResponse> = suggestions
        .into_iter()
        .map(|suggestion| SearchSuggestionResponse {
            slug: suggestion.slug,
            title: suggestion.title,
        })
        .collect();
    Ok(json_with_cache(&body, CACHE_NO_STORE))
}

async fn list_public_archives(State(state): State<AppState>) -> Result<Response, ApiError> {
    let months = state.content.list_public_archives().await?;
    let body: Vec<ArchiveMonthResponse> = months.into_iter().map(Into::into).collect();
    Ok(json_with_cache(&body, CACHE_AGGREGATE))
}

fn access_cookie_name(article_id: i64) -> String {
    format!("aries_article_access_{article_id}")
}

/// 解锁凭据 = HMAC-SHA256(bootstrap_secret, article_id + "." + password_hash)。
/// 密码修改后 Hash 变化，旧凭据自动失效，无需服务端存储或撤销列表。
fn access_credential(secret: &str, article_id: i64, password_hash: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(article_id.to_string().as_bytes());
    mac.update(b".");
    mac.update(password_hash.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_credential_is_stable_and_binds_article_and_password() {
        let first = access_credential("secret-0123456789", 1, "hash-a");
        assert_eq!(first.len(), 64);
        assert_eq!(first, access_credential("secret-0123456789", 1, "hash-a"));
        assert_ne!(first, access_credential("secret-0123456789", 2, "hash-a"));
        assert_ne!(first, access_credential("secret-0123456789", 1, "hash-b"));
        assert_ne!(first, access_credential("other-secret", 1, "hash-a"));
    }
}
