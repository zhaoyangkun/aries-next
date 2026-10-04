//! Public 扩展内容只读端点：Page / Journal / Gallery / Link / Navigation。
//! 只返回 published / public / visible 数据；Private Journal 永不外泄（Phase 06 §5）。

use aries_core::{
    content::ArticleStatus,
    galleries::GalleryError,
    journals::JournalPage,
    navigation::{NavigationItem, NavigationTargetType},
    pages::PageError,
};
use axum::{
    Router,
    extract::{Path, Query, State},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::http::error::ApiError;
use crate::state::AppState;

use super::{CACHE_AGGREGATE, CACHE_ARTICLE, json_with_cache, reject_out_of_range_page};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/{slug}", get(get_page))
        .route("/journals", get(list_journals))
        .route("/galleries", get(list_galleries))
        .route("/galleries/{slug}", get(get_gallery))
        .route("/photos", get(list_photos))
        .route("/links", get(list_links))
        .route("/navigation", get(list_navigation))
}

// ============================================================
// Page
// ============================================================

/// 公开页面 DTO：只回渲染后的 HTML 与展示字段，不回 Markdown 源与内部字段。
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicPageResponse {
    /// 页面 Slug。
    slug: String,
    /// 页面标题。
    title: String,
    /// 服务端渲染并 Sanitize 后的页面 HTML。
    content_html: String,
    /// 最近更新时间。
    updated_at: OffsetDateTime,
}

/// 已发布页面的公开详情。
#[utoipa::path(
    get,
    path = "/api/public/pages/{slug}",
    tag = "Public Pages",
    operation_id = "getPublicPage",
    summary = "按 Slug 获取已发布页面详情",
    description = "匿名可访问；只返回 `published` 页面。草稿、已删除或不存在的 Slug 一律返回 404 `PAGE_NOT_FOUND`。响应不含 Markdown 原文。",
    params(
        ("slug" = String, Path, description = "页面 Slug")
    ),
    responses(
        (status = 200, description = "已发布页面的公开详情", body = PublicPageResponse),
        (status = 404, description = "页面不存在或未发布（PAGE_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    let page = state
        .pages
        .find_published_by_slug(&slug)
        .await?
        .ok_or(PageError::NotFound)?;
    Ok(json_with_cache(
        &PublicPageResponse {
            slug: page.slug,
            title: page.title,
            content_html: page.content_html,
            updated_at: page.updated_at,
        },
        CACHE_ARTICLE,
    ))
}

// ============================================================
// Journal
// ============================================================

#[derive(Debug, Deserialize, utoipa::IntoParams)]
struct PublicJournalParams {
    /// 页码，从 1 开始。
    #[serde(default = "crate::http::default_page")]
    page: u32,
    /// 每页条数，1–100。
    #[serde(default = "crate::http::default_page_size")]
    page_size: u32,
}

/// 公开日志条目：只含渲染后的 HTML。
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicJournalResponse {
    /// 日志 ID。
    id: i64,
    /// 服务端渲染并 Sanitize 后的日志 HTML。
    content_html: String,
    /// 创建时间。
    created_at: OffsetDateTime,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicJournalPageResponse {
    /// 当前页条目。
    items: Vec<PublicJournalResponse>,
    /// 总条目数。
    total: i64,
    /// 当前页码。
    page: u32,
    /// 每页条数。
    page_size: u32,
}

/// 公开日志分页列表。
#[utoipa::path(
    get,
    path = "/api/public/journals",
    tag = "Public Journals",
    operation_id = "listPublicJournals",
    summary = "分页获取公开日志列表",
    description = "匿名可访问；只返回 `visibility = public` 的日志，`private` 内容永不进入结果集。稳定排序 `created_at DESC, id DESC`。`page > 1` 且当前页结果为空时返回 404 `PAGE_OUT_OF_RANGE`（page=1 空结果保持 200）。",
    params(PublicJournalParams),
    responses(
        (status = 200, description = "公开日志分页；列表项只含渲染后的 HTML", body = PublicJournalPageResponse),
        (status = 404, description = "越界分页（PAGE_OUT_OF_RANGE）", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_journals(
    State(state): State<AppState>,
    Query(params): Query<PublicJournalParams>,
) -> Result<Response, ApiError> {
    // list_public 在 Repository 固定 visibility = 'public'，Private 不进入结果集。
    let result: JournalPage = state
        .journals
        .list_public(params.page.max(1), params.page_size.clamp(1, 100))
        .await?;
    reject_out_of_range_page(result.page, result.items.len())?;
    Ok(json_with_cache(
        &PublicJournalPageResponse {
            items: result
                .items
                .into_iter()
                .map(|journal| PublicJournalResponse {
                    id: journal.id,
                    content_html: journal.content_html,
                    created_at: journal.created_at,
                })
                .collect(),
            total: result.total,
            page: result.page,
            page_size: result.page_size,
        },
        CACHE_AGGREGATE,
    ))
}

// ============================================================
// Gallery
// ============================================================

#[derive(Debug, Deserialize, utoipa::IntoParams)]
struct PublicGalleryParams {
    /// 页码，从 1 开始。
    #[serde(default = "crate::http::default_page")]
    page: u32,
    /// 每页条数，1–100。
    #[serde(default = "crate::http::default_page_size")]
    page_size: u32,
}

/// 公开图库摘要。
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicGallerySummary {
    /// 图库 Slug。
    slug: String,
    /// 图库标题。
    title: String,
    /// 图库描述。
    description: String,
    /// 封面媒体公开 URL；无封面为 null。
    cover_url: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicGalleryPageResponse {
    /// 当前页图库摘要。
    items: Vec<PublicGallerySummary>,
    /// 总条目数。
    total: i64,
    /// 当前页码。
    page: u32,
    /// 每页条数。
    page_size: u32,
}

/// 公开图库摘要分页列表。
#[utoipa::path(
    get,
    path = "/api/public/galleries",
    tag = "Public Galleries",
    operation_id = "listPublicGalleries",
    summary = "分页获取公开图库摘要列表",
    description = "匿名可访问；只返回 `published` 图库摘要，`cover_url` 取封面媒体的公开 URL。`page > 1` 且当前页结果为空时返回 404 `PAGE_OUT_OF_RANGE`（page=1 空结果保持 200）。",
    params(PublicGalleryParams),
    responses(
        (status = 200, description = "公开图库摘要分页", body = PublicGalleryPageResponse),
        (status = 404, description = "越界分页（PAGE_OUT_OF_RANGE）", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_galleries(
    State(state): State<AppState>,
    Query(params): Query<PublicGalleryParams>,
) -> Result<Response, ApiError> {
    let result = state
        .galleries
        .list_public(params.page.max(1), params.page_size.clamp(1, 100))
        .await?;
    reject_out_of_range_page(result.page, result.items.len())?;
    let mut items = Vec::with_capacity(result.items.len());
    for gallery in result.items {
        let cover_url = match gallery.cover_media_id {
            Some(media_id) => state.media.find(media_id).await?.map(|asset| asset.url),
            None => None,
        };
        items.push(PublicGallerySummary {
            slug: gallery.slug,
            title: gallery.title,
            description: gallery.description,
            cover_url,
        });
    }
    Ok(json_with_cache(
        &PublicGalleryPageResponse {
            items,
            total: result.total,
            page: result.page,
            page_size: result.page_size,
        },
        CACHE_AGGREGATE,
    ))
}

/// 公开图库条目：只回展示字段与媒体 URL/尺寸，不回内部 asset 元数据。
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicGalleryItemResponse {
    /// 媒体公开 URL。
    url: String,
    /// 替代文本；条目未填时回退媒体资产自身的 `alt`。
    alt: String,
    /// 拍摄地点。
    location: String,
    /// 媒体宽度（像素）；未知为 null。
    width: Option<i32>,
    /// 媒体高度（像素）；未知为 null。
    height: Option<i32>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicGalleryResponse {
    /// 图库 Slug。
    slug: String,
    /// 图库标题。
    title: String,
    /// 图库描述。
    description: String,
    /// 图库条目列表；媒体缺失（软删除）的条目已被跳过。
    items: Vec<PublicGalleryItemResponse>,
}

/// 已发布图库的公开详情（含条目）。
#[utoipa::path(
    get,
    path = "/api/public/galleries/{slug}",
    tag = "Public Galleries",
    operation_id = "getPublicGallery",
    summary = "按 Slug 获取已发布图库详情",
    description = "匿名可访问；只返回 `published` 图库，草稿、已删除或不存在的 Slug 一律返回 404 `GALLERY_NOT_FOUND`。条目携带媒体公开 URL 与尺寸；条目未填 `alt` 时回退媒体资产自身的 `alt`。",
    params(
        ("slug" = String, Path, description = "图库 Slug")
    ),
    responses(
        (status = 200, description = "公开图库详情（含条目）", body = PublicGalleryResponse),
        (status = 404, description = "图库不存在或未发布（GALLERY_NOT_FOUND）", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_gallery(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Response, ApiError> {
    let gallery = state
        .galleries
        .find_published_by_slug(&slug)
        .await?
        .ok_or(GalleryError::NotFound)?;
    // 媒体摘要由 Repository 一次 JOIN 带回（消除逐条 media.find 的 N+1）；
    // 媒体缺失（软删除）时跳过该条目，公开站不出坏图。
    let items = state.galleries.list_items(gallery.id).await?;
    let mut responses = Vec::with_capacity(items.len());
    for detail in items {
        let Some(media) = detail.media else {
            continue;
        };
        let item = detail.item;
        responses.push(PublicGalleryItemResponse {
            url: media.url,
            alt: if item.alt.is_empty() {
                media.alt
            } else {
                item.alt
            },
            location: item.location,
            width: media.width,
            height: media.height,
        });
    }
    Ok(json_with_cache(
        &PublicGalleryResponse {
            slug: gallery.slug,
            title: gallery.title,
            description: gallery.description,
            items: responses,
        },
        CACHE_ARTICLE,
    ))
}

// ============================================================
// Photos（照片墙）
// ============================================================

/// 公开照片墙条目：跨相册平铺，字段已含解析后的媒体 URL/尺寸与分类名。
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicPhotoResponse {
    /// 媒体公开 URL。
    url: String,
    /// 替代文本。
    alt: String,
    /// 拍摄地点。
    location: String,
    /// 媒体宽度（像素）；未知为 null。
    width: Option<i32>,
    /// 媒体高度（像素）；未知为 null。
    height: Option<i32>,
    /// 所属图库 Slug。
    gallery_slug: String,
    /// 所属图库标题。
    gallery_title: String,
    /// 所属图库分类名；无分类为 null。
    category_name: Option<String>,
}

/// 公开照片墙列表。
#[utoipa::path(
    get,
    path = "/api/public/photos",
    tag = "Public Galleries",
    operation_id = "listPhotos",
    summary = "获取公开照片墙列表",
    description = "匿名可访问的照片墙：跨相册平铺所有 `published` 且未删除相册的条目，不分页（对齐旧版 xue 主题的 GetAll 行为）。稳定排序 `galleries.sort_order ASC, galleries.id ASC, gallery_items.sort_order ASC`。",
    responses(
        (status = 200, description = "公开照片墙列表", body = [PublicPhotoResponse]),
    )
)]
async fn list_photos(State(state): State<AppState>) -> Result<Response, ApiError> {
    // 不分页：个人博客照片量级一次拉完，对齐旧版 xue 的 GetAll 行为；
    // published/软删除过滤与排序规则在 Repository 的 JOIN 查询中固定。
    let photos = state.galleries.list_public_photos().await?;
    let body: Vec<PublicPhotoResponse> = photos
        .into_iter()
        .map(|photo| PublicPhotoResponse {
            url: photo.url,
            alt: photo.alt,
            location: photo.location,
            width: photo.width,
            height: photo.height,
            gallery_slug: photo.gallery_slug,
            gallery_title: photo.gallery_title,
            category_name: photo.category_name,
        })
        .collect();
    Ok(json_with_cache(&body, CACHE_AGGREGATE))
}

// ============================================================
// Link
// ============================================================

/// 公开友链 DTO：不回 status/sort_order 等管理字段；
/// `category_name` 供前端按分类分组展示（对齐旧版 xue 主题的分组标题）。
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct PublicLinkResponse {
    /// 友链标题。
    title: String,
    /// 友链 URL。
    url: String,
    /// 图标 URL；无图标为 null。
    icon_url: Option<String>,
    /// 友链描述。
    description: String,
    /// 所属分类 ID；无分类为 null。
    category_id: Option<i64>,
    /// 所属分类名（供前端分组展示）；无分类或解析失败为 null。
    category_name: Option<String>,
}

/// 公开友情链接列表。
#[utoipa::path(
    get,
    path = "/api/public/links",
    tag = "Public Links",
    operation_id = "listPublicLinks",
    summary = "获取公开友情链接列表",
    description = "匿名可访问；只返回 `status = active` 的友情链接，不含管理字段。稳定排序 `sort_order ASC, id ASC`。",
    responses(
        (status = 200, description = "公开友情链接列表", body = [PublicLinkResponse]),
    )
)]
async fn list_links(State(state): State<AppState>) -> Result<Response, ApiError> {
    // list_public 在 Repository 固定 status = 'active'。
    let links = state.links.list_public().await?;
    let mut body = Vec::with_capacity(links.len());
    for link in links {
        // 分类名解析失败降级为 None，不让单个坏分类拖垮整个友链列表。
        let category_name = match link.category_id {
            Some(category_id) => state
                .content
                .find_category(category_id)
                .await
                .ok()
                .flatten()
                .map(|category| category.name),
            None => None,
        };
        body.push(PublicLinkResponse {
            title: link.title,
            url: link.url,
            icon_url: link.icon_url,
            description: link.description,
            category_id: link.category_id,
            category_name,
        });
    }
    Ok(json_with_cache(&body, CACHE_AGGREGATE))
}

// ============================================================
// Navigation
// ============================================================

/// 公开导航节点：两级树，children 为一级节点的子菜单。
/// `href` 为服务端解析后的路由地址，前端直接用于 NuxtLink，无需关心 target 拼路由规则。
#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(no_recursion)]
struct PublicNavigationNode {
    /// 菜单显示名。
    label: String,
    /// 目标类型：`article` / `page` / `category` / `url`。
    target_type: String,
    /// 内部目标 ID（article/page/category 时存在）。
    target_id: Option<i64>,
    /// 外链地址；非 url 类型为 null。
    url: Option<String>,
    /// 服务端解析后的可访问路由地址。
    href: Option<String>,
    /// 是否新标签页打开。
    open_in_new_tab: bool,
    /// 子菜单（仅一级节点可能有子节点）。
    children: Vec<PublicNavigationNode>,
}

/// 解析节点的可访问 href：url 原样保留；内部目标按 slug 拼路由，
/// 目标不存在或未发布时返回 None（调用方将剔除该节点，死链接不进公开导航）。
async fn resolve_href(state: &AppState, item: &NavigationItem) -> Result<Option<String>, ApiError> {
    match item.target_type {
        NavigationTargetType::Url => Ok(item.url.clone()),
        NavigationTargetType::Article => match item.target_id {
            Some(article_id) => {
                let article = state.content.find_article(article_id).await?;
                Ok(match article {
                    Some(article) if article.status == ArticleStatus::Published => {
                        Some(format!("/articles/{}", article.slug))
                    }
                    _ => None,
                })
            }
            None => Ok(None),
        },
        NavigationTargetType::Page => match item.target_id {
            Some(page_id) => {
                let page = state.pages.find(page_id).await?;
                // 对齐旧版 xue 主题的 /custom/:url 路由。
                Ok(match page {
                    Some(page) if page.status == aries_core::pages::PageStatus::Published => {
                        Some(format!("/custom/{}", page.slug))
                    }
                    _ => None,
                })
            }
            None => Ok(None),
        },
        NavigationTargetType::Category => match item.target_id {
            Some(category_id) => {
                let category = state.content.find_category(category_id).await?;
                Ok(category.map(|category| format!("/categories/{}", category.slug)))
            }
            None => Ok(None),
        },
    }
}

/// 构建公开节点：href 解析失败的非 url 节点返回 None（调用方连同子树剔除）。
/// 菜单最多两级，子节点不再有 children，无需递归。
async fn to_public_node(
    state: &AppState,
    item: &NavigationItem,
    children: Vec<PublicNavigationNode>,
) -> Result<Option<PublicNavigationNode>, ApiError> {
    let Some(href) = resolve_href(state, item).await? else {
        return Ok(None);
    };
    Ok(Some(PublicNavigationNode {
        label: item.label.clone(),
        target_type: item.target_type.as_str().to_owned(),
        target_id: item.target_id,
        url: item.url.clone(),
        href: Some(href),
        open_in_new_tab: item.open_in_new_tab,
        children,
    }))
}

/// 公开导航两级树。
#[utoipa::path(
    get,
    path = "/api/public/navigation",
    tag = "Public Navigation",
    operation_id = "listPublicNavigation",
    summary = "获取公开导航两级树",
    description = "匿名可访问；只返回 `visible = true` 的节点，组织为两级树（一级节点的 `children` 为其子菜单）。隐藏节点不出现在响应中。",
    responses(
        (status = 200, description = "公开导航两级树", body = [PublicNavigationNode]),
    )
)]
async fn list_navigation(State(state): State<AppState>) -> Result<Response, ApiError> {
    // list_visible 仅返回 visible = true 的项，隐藏菜单不出现在公开响应中。
    let items = state.navigation.list_visible().await?;
    let mut roots: Vec<PublicNavigationNode> = Vec::new();
    for item in items.iter().filter(|item| item.parent_id.is_none()) {
        let mut children = Vec::new();
        for child in items
            .iter()
            .filter(|child| child.parent_id == Some(item.id))
        {
            if let Some(node) = to_public_node(&state, child, Vec::new()).await? {
                children.push(node);
            }
        }
        // 死链接节点连同已解析的子树一并剔除。
        if let Some(node) = to_public_node(&state, item, children).await? {
            roots.push(node);
        }
    }
    Ok(json_with_cache(&roots, CACHE_AGGREGATE))
}
