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

#[derive(Debug, Serialize)]
struct PublicCategoryResponse {
    id: i64,
    name: String,
    slug: String,
    description: String,
    article_count: i64,
}

#[derive(Debug, Serialize)]
struct PublicTagResponse {
    id: i64,
    name: String,
    slug: String,
    article_count: i64,
}

#[derive(Debug, Deserialize)]
struct TaxonomyArticlesParams {
    #[serde(default = "crate::http::default_page")]
    page: u32,
}

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
