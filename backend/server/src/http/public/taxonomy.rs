//! 公开分类与标签端点：列表带实时 Published 文章数，文章分页大小取自站点设置。

use aries_core::content::PublicArticleQuery;
use axum::{
    Router,
    extract::{Path, Query, State},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::http::error::ApiError;
use crate::state::AppState;

use super::articles::{PublicArticleListItem, PublicArticlePageResponse};
use super::{CACHE_AGGREGATE, json_with_cache, reject_out_of_range_page};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/categories", get(list_public_categories))
        .route("/categories/{slug}/articles", get(list_category_articles))
        .route("/tags", get(list_public_tags))
        .route("/tags/{slug}/articles", get(list_tag_articles))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "PublicCategoryResponse",
    description = "公开分类摘要，含实时 Published 文章数。"
)]
struct PublicCategoryResponse {
    id: i64,
    name: String,
    slug: String,
    description: String,
    /// 实时统计的 Published 文章数。
    article_count: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(
    title = "PublicTagResponse",
    description = "公开标签摘要，含实时 Published 文章数。"
)]
struct PublicTagResponse {
    id: i64,
    name: String,
    slug: String,
    /// 实时统计的 Published 文章数。
    article_count: i64,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct TaxonomyArticlesParams {
    /// 页码，从 1 开始。
    #[serde(default = "crate::http::default_page")]
    page: u32,
}

#[utoipa::path(
    get,
    path = "/api/public/categories",
    tag = "Public Taxonomy",
    operation_id = "listPublicCategories",
    summary = "获取公开分类列表",
    description = "kind 为 `article` 的分类列表，含实时 Published 文章数。",
    responses(
        (status = 200, description = "分类摘要列表", body = [PublicCategoryResponse]),
    )
)]
async fn list_public_categories(State(state): State<AppState>) -> Result<Response, ApiError> {
    let categories = state.content.list_public_categories().await?;
    let body: Vec<PublicCategoryResponse> = categories
        .into_iter()
        .map(|category| PublicCategoryResponse {
            id: category.id,
            name: category.name,
            slug: category.slug,
            description: category.description,
            article_count: category.article_count,
        })
        .collect();
    Ok(json_with_cache(&body, CACHE_AGGREGATE))
}

#[utoipa::path(
    get,
    path = "/api/public/tags",
    tag = "Public Taxonomy",
    operation_id = "listPublicTags",
    summary = "获取公开标签列表",
    description = "标签列表，含实时 Published 文章数。",
    responses(
        (status = 200, description = "标签摘要列表", body = [PublicTagResponse]),
    )
)]
async fn list_public_tags(State(state): State<AppState>) -> Result<Response, ApiError> {
    let tags = state.content.list_public_tags().await?;
    let body: Vec<PublicTagResponse> = tags
        .into_iter()
        .map(|tag| PublicTagResponse {
            id: tag.id,
            name: tag.name,
            slug: tag.slug,
            article_count: tag.article_count,
        })
        .collect();
    Ok(json_with_cache(&body, CACHE_AGGREGATE))
}

#[utoipa::path(
    get,
    path = "/api/public/categories/{slug}/articles",
    tag = "Public Taxonomy",
    operation_id = "listCategoryArticles",
    summary = "获取分类下的公开文章分页",
    description = "该分类下 Published 文章分页；分页大小取站点设置 `page_size_index`。`page > 1` 且当前页结果为空时返回 404 `PAGE_OUT_OF_RANGE`（page=1 空结果保持 200）。",
    params(
        ("slug" = String, Path, description = "分类 Slug"),
        TaxonomyArticlesParams,
    ),
    responses(
        (status = 200, description = "分页文章列表", body = PublicArticlePageResponse),
        (status = 404, description = "分类不存在（`CATEGORY_NOT_FOUND`）或页码越界（`PAGE_OUT_OF_RANGE`）", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_category_articles(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<TaxonomyArticlesParams>,
) -> Result<Response, ApiError> {
    let category = state
        .content
        .find_category_by_slug(aries_core::content::CategoryKind::Article, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found("CATEGORY_NOT_FOUND", "Category was not found"))?;
    // 分页大小来自站点设置（旧版分类页默认 3 是 Bug，不继承）。
    let settings = state.site_settings.get().await?;
    let page = state
        .content
        .list_public_articles(PublicArticleQuery {
            page: params.page,
            page_size: settings.page_size_index.max(1) as u32,
            category_id: Some(category.id),
            ..PublicArticleQuery::default()
        })
        .await?;
    reject_out_of_range_page(page.page, page.items.len())?;
    Ok(json_with_cache(
        &PublicArticlePageResponse {
            items: page.items.iter().map(PublicArticleListItem::from).collect(),
            total: page.total,
            page: page.page,
            page_size: page.page_size,
        },
        CACHE_AGGREGATE,
    ))
}

#[utoipa::path(
    get,
    path = "/api/public/tags/{slug}/articles",
    tag = "Public Taxonomy",
    operation_id = "listTagArticles",
    summary = "获取标签下的公开文章分页",
    description = "该标签下 Published 文章分页；分页大小取站点设置 `page_size_index`。`page > 1` 且当前页结果为空时返回 404 `PAGE_OUT_OF_RANGE`（page=1 空结果保持 200）。",
    params(
        ("slug" = String, Path, description = "标签 Slug"),
        TaxonomyArticlesParams,
    ),
    responses(
        (status = 200, description = "分页文章列表", body = PublicArticlePageResponse),
        (status = 404, description = "标签不存在（`TAG_NOT_FOUND`）或页码越界（`PAGE_OUT_OF_RANGE`）", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_tag_articles(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(params): Query<TaxonomyArticlesParams>,
) -> Result<Response, ApiError> {
    let tag = state
        .content
        .find_tag_by_slug(&slug)
        .await?
        .ok_or_else(|| ApiError::not_found("TAG_NOT_FOUND", "Tag was not found"))?;
    let settings = state.site_settings.get().await?;
    let page = state
        .content
        .list_public_articles(PublicArticleQuery {
            page: params.page,
            page_size: settings.page_size_index.max(1) as u32,
            tag_id: Some(tag.id),
            ..PublicArticleQuery::default()
        })
        .await?;
    reject_out_of_range_page(page.page, page.items.len())?;
    Ok(json_with_cache(
        &PublicArticlePageResponse {
            items: page.items.iter().map(PublicArticleListItem::from).collect(),
            total: page.total,
            page: page.page,
            page_size: page.page_size,
        },
        CACHE_AGGREGATE,
    ))
}
