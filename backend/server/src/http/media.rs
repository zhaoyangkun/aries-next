//! 媒体库 Admin 端点：本地上传、远端抓取（SSRF 防护）、引用维护、Markdown 批量导入。
//! 静态文件服务入口（`serve_media_file`）也在这里，因为它与上传共享同一套 Key 校验与存储抽象。

use std::collections::HashSet;
use std::net::IpAddr;
use std::str::FromStr;

use aries_core::{
    auth::{AuditEvent, Permission},
    content::{ContentError, NewArticle, NewTag, normalize_slug},
    jobs::{JobKind, NewBackgroundJob},
    media::{
        MediaAsset, MediaListQuery, MediaProvider, MediaStatus, MediaUpdate, NewMediaAsset,
        extract_media_usage_refs,
    },
};
use aries_infra::upload::{
    self, MAX_BATCH_FILES, MAX_FILE_BYTES, generate_object_key, validate_upload,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

use super::{articles::render_markdown, auth::CurrentUser, error::ApiError, extract::ApiJson};

/// 上传请求体上限：5 个 5MB 文件 + Multipart 边界开销。
const MAX_UPLOAD_REQUEST_BYTES: usize = MAX_BATCH_FILES * MAX_FILE_BYTES + 1024 * 1024;
/// Markdown 导入：单批最多 10 个文件，单文件 2MB。
const MAX_IMPORT_FILES: usize = 10;
const MAX_IMPORT_BYTES: usize = 2 * 1024 * 1024;
/// 批量删除单次上限：避免一次请求携带过多 ID 拉长事务与探测耗时。
const MAX_BATCH_DELETE: usize = 100;
const MAX_REMOTE_REDIRECTS: u8 = 3;
const REMOTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/media", get(list_media).post(upload_media))
        .route("/media/remote", post(upload_remote))
        .route("/media/batch-delete", post(batch_delete_media))
        .route(
            "/media/{id}",
            get(get_media).put(update_media).delete(delete_media),
        )
        .route("/media/{id}/usages", get(list_media_usages))
        .route("/articles/imports", post(import_markdown))
        .route("/imports/{id}", get(get_import))
        .route("/imports/{id}/commit", post(commit_import))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_REQUEST_BYTES))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct MediaAssetResponse {
    id: i64,
    /// 存储 Provider：`local` / `s3` / `legacy_url`。
    provider: &'static str,
    /// 服务端生成的不可预测 Key（`yyyy/mm/<uuid>.<ext>`），不含用户文件名。
    object_key: String,
    url: String,
    original_name: String,
    mime: String,
    size_bytes: i64,
    /// 尺寸探测失败时为 `null`，由 `metadata_probe` 后台任务补探测。
    width: Option<i32>,
    height: Option<i32>,
    sha256: String,
    alt: String,
    /// `active` / `deleted`；列表与详情只返回 `active`。
    status: &'static str,
    uploaded_by: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: time::OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: time::OffsetDateTime,
    /// 同内容 Hash 的已存在资产 ID；仅在本次上传命中重复时返回。
    #[serde(skip_serializing_if = "Option::is_none")]
    duplicate_of: Option<i64>,
}

impl MediaAssetResponse {
    fn from_asset(asset: MediaAsset, duplicate_of: Option<i64>) -> Self {
        Self {
            id: asset.id,
            provider: asset.provider.as_str(),
            object_key: asset.object_key,
            url: asset.url,
            original_name: asset.original_name,
            mime: asset.mime,
            size_bytes: asset.size_bytes,
            width: asset.width,
            height: asset.height,
            sha256: asset.sha256,
            alt: asset.alt,
            status: asset.status.as_str(),
            uploaded_by: asset.uploaded_by,
            created_at: asset.created_at,
            updated_at: asset.updated_at,
            duplicate_of,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct MediaPageResponse {
    items: Vec<MediaAssetResponse>,
    total: i64,
    page: u32,
    page_size: u32,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct MediaUsageResponse {
    id: i64,
    asset_id: i64,
    /// 引用方类型：`article_cover` / `article_content` / `gallery_item`。
    target_type: &'static str,
    target_id: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: time::OffsetDateTime,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct MediaListParams {
    #[serde(default = "super::default_page")]
    page: u32,
    #[serde(default = "super::default_page_size")]
    page_size: u32,
    /// 按 `original_name` 模糊匹配。
    keyword: Option<String>,
    /// 按存储 Provider 过滤：`local` / `s3` / `legacy_url`。
    provider: Option<String>,
    mime: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct UpdateMediaRequest {
    /// 替换说明文字；缺省为空串。
    #[serde(default)]
    alt: String,
    /// 替换原始文件名；缺省或空串表示不改动。
    original_name: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct RemoteUploadRequest {
    /// 待抓取的远端图片 URL（仅 http/https）。
    url: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct BatchDeleteMediaRequest {
    /// 待删除的资产 ID 列表，单次 1–100 个（服务端去重）。
    #[serde(default)]
    ids: Vec<i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(description = "批量删除的部分成功结果；三个列表均按 ID 升序，且互不相交。")]
struct BatchDeleteMediaResponse {
    /// 成功软删除的 ID。
    deleted: Vec<i64>,
    /// 仍被内容引用、保持 active 的 ID。
    referenced: Vec<i64>,
    /// 不存在或已删除的 ID。
    not_found: Vec<i64>,
}

/// `POST /api/admin/media` 的 Multipart 表单；实际字段名为 `file[]`（兼容 `file`），
/// 每批 1–5 个文件，单文件 ≤ 5MB。
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
struct UploadMediaForm {
    /// 待上传图片文件（jpg/jpeg/png/gif/bmp/webp）。
    #[schema(value_type = Vec<String>, format = Binary)]
    file: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/api/admin/media",
    tag = "Admin Media",
    operation_id = "listMedia",
    summary = "分页查询媒体资产列表",
    description = "仅返回 `active` 资产；稳定排序 `created_at DESC, id DESC`。",
    security(("cookieAuth" = [])),
    params(MediaListParams),
    responses(
        (status = 200, description = "媒体资产分页列表", body = MediaPageResponse),
        (status = 400, description = "Provider 过滤值非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_media(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<MediaListParams>,
) -> Result<Json<MediaPageResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let provider = params
        .provider
        .as_deref()
        .map(|value| {
            MediaProvider::from_str(value).map_err(|_| {
                ApiError::bad_request("INVALID_MEDIA_PROVIDER", "Media provider filter is invalid")
            })
        })
        .transpose()?;
    let page = state
        .media
        .list(MediaListQuery {
            page: params.page,
            page_size: params.page_size,
            keyword: params.keyword,
            provider,
            mime: params.mime,
        })
        .await?;
    Ok(Json(MediaPageResponse {
        items: page
            .items
            .into_iter()
            .map(|asset| MediaAssetResponse::from_asset(asset, None))
            .collect(),
        total: page.total,
        page: page.page,
        page_size: page.page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/api/admin/media/{id}",
    tag = "Admin Media",
    operation_id = "getMedia",
    summary = "获取单个媒体资产详情",
    description = "已软删除的资产返回 404。",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "媒体资产 ID"),
    ),
    responses(
        (status = 200, description = "资产详情", body = MediaAssetResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "资产不存在或已删除", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_media(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(asset_id): Path<i64>,
) -> Result<Json<MediaAssetResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let asset = find_active_asset(&state, asset_id).await?;
    Ok(Json(MediaAssetResponse::from_asset(asset, None)))
}

#[utoipa::path(
    put,
    path = "/api/admin/media/{id}",
    tag = "Admin Media",
    operation_id = "updateMedia",
    summary = "更新媒体资产元数据",
    description = "仅允许更新 `alt` 与 `original_name`；文件内容、Hash、URL 创建后不可变。",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "媒体资产 ID"),
    ),
    request_body(description = "待更新的元数据", content = UpdateMediaRequest),
    responses(
        (status = 200, description = "资产已更新", body = MediaAssetResponse),
        (status = 400, description = "字段超长", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "资产不存在或已删除", body = crate::openapi::ErrorResponse),
    )
)]
async fn update_media(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(asset_id): Path<i64>,
    ApiJson(request): ApiJson<UpdateMediaRequest>,
) -> Result<Json<MediaAssetResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let existing = find_active_asset(&state, asset_id).await?;
    let original_name = request
        .original_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or(existing.original_name);
    if original_name.chars().count() > 255 || request.alt.chars().count() > 500 {
        return Err(ApiError::bad_request(
            "INVALID_MEDIA_FIELDS",
            "Media alt or original name is too long",
        ));
    }
    let asset = state
        .media
        .update(
            asset_id,
            MediaUpdate {
                alt: request.alt.trim().to_owned(),
                original_name,
            },
        )
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "media.update".to_owned(),
            target_type: "media".to_owned(),
            target_id: Some(asset_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(Json(MediaAssetResponse::from_asset(asset, None)))
}

#[utoipa::path(
    delete,
    path = "/api/admin/media/{id}",
    tag = "Admin Media",
    operation_id = "deleteMedia",
    summary = "软删除媒体资产",
    description = "仍存在引用时返回 409 并携带 Usages 摘要。物理删除由 `media_cleanup` 后台任务在零引用后执行。",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "媒体资产 ID"),
    ),
    responses(
        (status = 204, description = "已软删除"),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "资产不存在或已删除", body = crate::openapi::ErrorResponse),
        (status = 409, description = "资产仍被内容引用（MEDIA_IN_USE）", body = crate::openapi::ErrorResponse),
    )
)]
async fn delete_media(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(asset_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require(Permission::ManageContent)?;
    find_active_asset(&state, asset_id).await?;
    let usages = state.media.usages_of(asset_id).await?;
    if !usages.is_empty() {
        // 409 携带引用摘要，前端可直接展示“哪些内容还在用这张图”。
        return Err(ApiError::conflict_with_details(
            "MEDIA_IN_USE",
            "Media asset is still referenced",
            serde_json::json!({
                "reference_count": usages.len(),
                "usages": usages
                    .iter()
                    .map(|usage| serde_json::json!({
                        "target_type": usage.target_type.as_str(),
                        "target_id": usage.target_id,
                    }))
                    .collect::<Vec<_>>(),
            }),
        ));
    }
    state.media.soft_delete(asset_id).await?;
    // 物理删除交给后台任务，软删除先保证内容可恢复。
    state
        .jobs
        .enqueue(NewBackgroundJob::new(
            JobKind::MediaCleanup,
            serde_json::json!({}),
        ))
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "media.delete".to_owned(),
            target_type: "media".to_owned(),
            target_id: Some(asset_id.to_string()),
            metadata: serde_json::json!({}),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/admin/media/batch-delete",
    tag = "Admin Media",
    operation_id = "batchDeleteMedia",
    summary = "批量软删除媒体资产",
    description = "部分成功语义：被引用的资产保持 active 并归入 `referenced`；不存在或已删除的 ID 归入 `not_found`。单次 1–100 个 ID（服务端去重）。有实际删除时自动入队 `media_cleanup` 任务。",
    security(("cookieAuth" = [])),
    request_body(description = "待删除的资产 ID 列表", content = BatchDeleteMediaRequest),
    responses(
        (status = 200, description = "逐项归类结果（deleted / referenced / not_found 均按 ID 升序）", body = BatchDeleteMediaResponse),
        (status = 400, description = "空批次或超过 100 个 ID（INVALID_BATCH_DELETE）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn batch_delete_media(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<BatchDeleteMediaRequest>,
) -> Result<Json<BatchDeleteMediaResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    if request.ids.is_empty() || request.ids.len() > MAX_BATCH_DELETE {
        return Err(ApiError::bad_request_with_details(
            "INVALID_BATCH_DELETE",
            "Batch delete requires 1 to 100 media ids",
            serde_json::json!({ "min": 1, "max": MAX_BATCH_DELETE }),
        ));
    }
    // 部分成功语义：被引用项保持 active，其余成功软删除，逐项结果随响应返回。
    let result = state.media.batch_delete(&request.ids).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "media.batch_delete".to_owned(),
            target_type: "media".to_owned(),
            target_id: None,
            metadata: serde_json::json!({
                "requested": request.ids.len(),
                "deleted": result.deleted,
                "referenced": result.referenced,
                "not_found": result.not_found,
            }),
        })
        .await?;
    // 有实际删除时才需要清理任务；清理任务自身会按零引用过滤，幂等安全。
    if !result.deleted.is_empty() {
        state
            .jobs
            .enqueue(NewBackgroundJob::new(
                JobKind::MediaCleanup,
                serde_json::json!({}),
            ))
            .await?;
    }
    Ok(Json(BatchDeleteMediaResponse {
        deleted: result.deleted,
        referenced: result.referenced,
        not_found: result.not_found,
    }))
}

#[utoipa::path(
    get,
    path = "/api/admin/media/{id}/usages",
    tag = "Admin Media",
    operation_id = "listMediaUsages",
    summary = "列出媒体资产的引用",
    description = "返回引用该资产的内容列表（封面 / 正文 / 图库条目）。",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "媒体资产 ID"),
    ),
    responses(
        (status = 200, description = "资产的引用列表", body = Vec<MediaUsageResponse>),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "资产不存在或已删除", body = crate::openapi::ErrorResponse),
    )
)]
async fn list_media_usages(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(asset_id): Path<i64>,
) -> Result<Json<Vec<MediaUsageResponse>>, ApiError> {
    current.require(Permission::ManageContent)?;
    find_active_asset(&state, asset_id).await?;
    let usages = state.media.usages_of(asset_id).await?;
    Ok(Json(
        usages
            .into_iter()
            .map(|usage| MediaUsageResponse {
                id: usage.id,
                asset_id: usage.asset_id,
                target_type: usage.target_type.as_str(),
                target_id: usage.target_id,
                created_at: usage.created_at,
            })
            .collect(),
    ))
}

/// 校验通过后待入库的文件；本地与远端上传共用同一入口。
struct IncomingFile {
    original_name: String,
    declared_mime: Option<String>,
    bytes: Bytes,
}

#[utoipa::path(
    post,
    path = "/api/admin/media",
    tag = "Admin Media",
    operation_id = "uploadMedia",
    summary = "本地上传媒体文件",
    description = "Multipart 字段名 `file[]`，每批 1–5 个文件，单文件 ≤ 5MB；扩展名白名单 jpg/jpeg/png/gif/bmp/webp，并校验 Magic Bytes 与声明 MIME。响应恒为数组。",
    security(("cookieAuth" = [])),
    request_body(content = UploadMediaForm, content_type = "multipart/form-data", description = "待上传文件批次"),
    responses(
        (status = 201, description = "全部文件已入库；同内容 Hash 的资产带 `duplicate_of` 提示", body = Vec<MediaAssetResponse>),
        (status = 400, description = "文件为空/超限/类型不允许或 Multipart 非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn upload_media(
    State(state): State<AppState>,
    current: CurrentUser,
    multipart: Multipart,
) -> Result<(StatusCode, Json<Vec<MediaAssetResponse>>), ApiError> {
    current.require(Permission::ManageContent)?;
    let files = collect_multipart_files(multipart, MAX_BATCH_FILES).await?;
    // 先完成全部校验再写存储，避免校验失败留下半批文件。
    let mut responses = Vec::with_capacity(files.len());
    for file in files {
        responses.push(store_incoming(&state, current.user.id, file).await?);
    }
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "media.upload".to_owned(),
            target_type: "media".to_owned(),
            target_id: None,
            metadata: serde_json::json!({
                "count": responses.len(),
                "asset_ids": responses.iter().map(|asset| asset.id).collect::<Vec<_>>(),
            }),
        })
        .await?;
    Ok((StatusCode::CREATED, Json(responses)))
}

#[utoipa::path(
    post,
    path = "/api/admin/media/remote",
    tag = "Admin Media",
    operation_id = "uploadRemoteMedia",
    summary = "抓取远端图片并入库",
    description = "SSRF 防护：仅 http/https，拒绝内网与环回地址，Redirect 最多 3 次且逐跳重校验，Body ≤ 5MB，超时 10s。",
    security(("cookieAuth" = [])),
    request_body(description = "远端图片 URL", content = RemoteUploadRequest),
    responses(
        (status = 201, description = "远端文件已下载并入库", body = MediaAssetResponse),
        (status = 400, description = "URL 非法、地址被禁止或抓取失败", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn upload_remote(
    State(state): State<AppState>,
    current: CurrentUser,
    ApiJson(request): ApiJson<RemoteUploadRequest>,
) -> Result<(StatusCode, Json<MediaAssetResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let file = fetch_remote(&request.url).await?;
    let response = store_incoming(&state, current.user.id, file).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "media.upload".to_owned(),
            target_type: "media".to_owned(),
            target_id: Some(response.id.to_string()),
            metadata: serde_json::json!({ "source": "remote" }),
        })
        .await?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// 校验、写入存储、入库的共用管线；本地与远端上传行为必须一致。
async fn store_incoming(
    state: &AppState,
    actor_id: i64,
    file: IncomingFile,
) -> Result<MediaAssetResponse, ApiError> {
    let validated = validate_upload(
        &file.original_name,
        file.declared_mime.as_deref(),
        &file.bytes,
    )
    .map_err(upload_error_to_api)?;
    let object_key = generate_object_key(validated.kind.extension());
    let url = state
        .storage
        .put(&object_key, &file.bytes, &validated.mime)
        .await?;
    let duplicate_of = state
        .media
        .find_by_hash(&validated.sha256)
        .await?
        .map(|asset| asset.id);
    let provider =
        MediaProvider::from_str(&state.config.media_provider).map_err(|_| ApiError::internal())?;
    let asset = state
        .media
        .create(NewMediaAsset {
            provider,
            object_key,
            url,
            original_name: file.original_name,
            mime: validated.mime,
            size_bytes: validated.size_bytes,
            width: validated.width,
            height: validated.height,
            sha256: validated.sha256,
            alt: String::new(),
            uploaded_by: actor_id,
        })
        .await?;
    tracing::info!(
        media_id = asset.id,
        original_name = %asset.original_name,
        size_bytes = asset.size_bytes,
        provider = %state.config.media_provider,
        uploaded_by = actor_id,
        "media upload stored"
    );
    Ok(MediaAssetResponse::from_asset(asset, duplicate_of))
}

async fn find_active_asset(state: &AppState, asset_id: i64) -> Result<MediaAsset, ApiError> {
    let asset = state
        .media
        .find(asset_id)
        .await?
        .ok_or_else(|| ApiError::not_found("MEDIA_NOT_FOUND", "Media asset was not found"))?;
    // 已软删除的资产对管理端不可见，与列表语义保持一致。
    if asset.status != MediaStatus::Active {
        return Err(ApiError::not_found(
            "MEDIA_NOT_FOUND",
            "Media asset was not found",
        ));
    }
    Ok(asset)
}

fn upload_error_to_api(error: upload::UploadError) -> ApiError {
    let (code, message) = match error {
        upload::UploadError::Empty => ("EMPTY_FILE", "Uploaded file is empty"),
        upload::UploadError::TooLarge => ("FILE_TOO_LARGE", "File exceeds the 5MB size limit"),
        upload::UploadError::UnsupportedExtension => {
            ("UNSUPPORTED_FILE_TYPE", "File extension is not allowed")
        }
        upload::UploadError::UnsupportedContent => (
            "UNSUPPORTED_FILE_TYPE",
            "File content is not a supported image",
        ),
        upload::UploadError::ContentMismatch | upload::UploadError::MimeMismatch => (
            "FILE_TYPE_MISMATCH",
            "File content does not match its declared type",
        ),
    };
    ApiError::bad_request(code, message)
}

async fn collect_multipart_files(
    mut multipart: Multipart,
    max_files: usize,
) -> Result<Vec<IncomingFile>, ApiError> {
    let mut files = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(|error| {
        ApiError::bad_request_with_details(
            "INVALID_MULTIPART",
            "Multipart body is invalid",
            serde_json::json!({ "reason": error.body_text() }),
        )
    })? {
        // 对齐旧版字段名 file[]，同时兼容 file；其他字段忽略。
        let name = field.name().unwrap_or_default().to_owned();
        if name != "file[]" && name != "file" {
            continue;
        }
        let original_name = field
            .file_name()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("upload")
            .to_owned();
        let declared_mime = field.content_type().map(str::to_owned);
        let bytes = field
            .bytes()
            .await
            .map_err(|_| ApiError::bad_request("FILE_TOO_LARGE", "File exceeds the size limit"))?;
        files.push(IncomingFile {
            original_name,
            declared_mime,
            bytes,
        });
        if files.len() > max_files {
            return Err(ApiError::bad_request(
                "TOO_MANY_FILES",
                "Too many files in one batch",
            ));
        }
    }
    if files.is_empty() {
        return Err(ApiError::bad_request(
            "NO_FILES",
            "At least one file is required",
        ));
    }
    Ok(files)
}

/// 抓取远端图片并回落到与本地相同的校验与入库流程。
/// SSRF 防护：仅 http/https、拒绝内网与环回地址、手动跟随 Redirect（每跳重校验）、
/// Body 上限 5MB、整体超时 10s。
async fn fetch_remote(raw_url: &str) -> Result<IncomingFile, ApiError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REMOTE_TIMEOUT)
        .build()
        .map_err(|_| ApiError::internal())?;

    let mut url = validate_remote_url(raw_url)?;
    let mut redirects = 0_u8;
    loop {
        ensure_host_allowed(&url).await?;
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| ApiError::bad_request("REMOTE_FETCH_FAILED", "Remote fetch failed"))?;
        if response.status().is_redirection() {
            if redirects >= MAX_REMOTE_REDIRECTS {
                return Err(ApiError::bad_request(
                    "REMOTE_FETCH_FAILED",
                    "Remote URL redirects too many times",
                ));
            }
            redirects += 1;
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    ApiError::bad_request("REMOTE_FETCH_FAILED", "Remote redirect is invalid")
                })?;
            // 相对 Redirect 以当前 URL 为基准解析，每一跳都重新走 SSRF 校验。
            let next = url.join(location).map_err(|_| {
                ApiError::bad_request("REMOTE_URL_INVALID", "Remote URL is invalid")
            })?;
            url = validate_remote_url(next.as_str())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(ApiError::bad_request(
                "REMOTE_FETCH_FAILED",
                "Remote server did not return a success status",
            ));
        }
        let declared_mime = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_owned()
            });
        let original_name = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|segment| !segment.is_empty())
            .unwrap_or("remote-image")
            .to_owned();
        let bytes = read_limited(response).await?;
        return Ok(IncomingFile {
            original_name,
            declared_mime,
            bytes,
        });
    }
}

async fn read_limited(mut response: reqwest::Response) -> Result<Bytes, ApiError> {
    let mut buffer = Vec::new();
    // 逐块读取并限制总量：不信任 Content-Length，防止远端声明小体积但持续吐数据。
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        ApiError::bad_request("REMOTE_FETCH_FAILED", "Failed to read remote response body")
    })? {
        if buffer.len() + chunk.len() > MAX_FILE_BYTES {
            return Err(ApiError::bad_request(
                "FILE_TOO_LARGE",
                "File exceeds the 5MB size limit",
            ));
        }
        buffer.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(buffer))
}

/// URL 静态校验：协议白名单 + 字面量 IP / localhost 直接拒绝。
/// 域名对应的解析结果在 `ensure_host_allowed` 中检查。
fn validate_remote_url(raw: &str) -> Result<reqwest::Url, ApiError> {
    let url = reqwest::Url::parse(raw.trim())
        .map_err(|_| ApiError::bad_request("REMOTE_URL_INVALID", "Remote URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(
            "REMOTE_URL_INVALID",
            "Remote URL must use http or https",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::bad_request("REMOTE_URL_INVALID", "Remote URL is invalid"))?;
    if is_forbidden_host(host) {
        return Err(ApiError::bad_request(
            "REMOTE_URL_FORBIDDEN",
            "Remote URL points to a forbidden address",
        ));
    }
    Ok(url)
}

/// 解析域名并检查所有解析结果；任一地址落入内网/环回即拒绝（防 DNS Rebinding 双答）。
async fn ensure_host_allowed(url: &reqwest::Url) -> Result<(), ApiError> {
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::bad_request("REMOTE_URL_INVALID", "Remote URL is invalid"))?;
    // 字面量 IP 在静态校验已处理，这里只对域名做解析。
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let port = url.port_or_known_default().unwrap_or(80);
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| {
            ApiError::bad_request("REMOTE_FETCH_FAILED", "Remote host cannot be resolved")
        })?
        .map(|address| address.ip())
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|ip| is_forbidden_ip(*ip)) {
        return Err(ApiError::bad_request(
            "REMOTE_URL_FORBIDDEN",
            "Remote URL points to a forbidden address",
        ));
    }
    Ok(())
}

fn is_forbidden_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    // URL Crate 的 IPv6 Host 带方括号（如 `[::1]`），先剥掉再解析。
    let bare = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse::<IpAddr>().is_ok_and(is_forbidden_ip)
}

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 100 // CGNAT 100.64.0.0/10 起始于 100
                    && (64..=127).contains(&v4.octets()[1])
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_forbidden_ip(IpAddr::V4(v4)))
        }
    }
}

// ---------------------------------------------------------------------------
// Markdown 批量导入
// ---------------------------------------------------------------------------

/// 导入预览项完整存入 Job Payload（含 Markdown 正文），Commit 阶段不再依赖原始文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportItem {
    index: usize,
    file_name: String,
    title: String,
    slug: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
    category: Option<String>,
    markdown_source: String,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    slug_conflict: bool,
}

/// 预览响应剥离正文，避免大字段反复传输。
#[derive(Debug, Serialize, utoipa::ToSchema)]
struct ImportPreview {
    index: usize,
    file_name: String,
    title: String,
    slug: String,
    summary: String,
    tags: Vec<String>,
    category: Option<String>,
    warnings: Vec<String>,
    /// Slug 已被现有文章占用时需要在 Commit 时给出 skip/rename 策略。
    slug_conflict: bool,
}

impl From<&ImportItem> for ImportPreview {
    fn from(item: &ImportItem) -> Self {
        Self {
            index: item.index,
            file_name: item.file_name.clone(),
            title: item.title.clone(),
            slug: item.slug.clone(),
            summary: item.summary.clone(),
            tags: item.tags.clone(),
            category: item.category.clone(),
            warnings: item.warnings.clone(),
            slug_conflict: item.slug_conflict,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct ImportJobResponse {
    job_id: i64,
    /// `pending` / `running` / `done` / `failed`。
    status: &'static str,
    items: Vec<ImportPreview>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct CommitImportRequest {
    /// 仅冲突项需要显式策略；未列出的冲突项默认 skip。
    #[serde(default)]
    items: Vec<CommitStrategy>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
struct CommitStrategy {
    /// 导入预览项的序号。
    index: usize,
    /// `skip`（跳过）或 `rename`（自动改名落库）。
    strategy: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct CommitResultResponse {
    created: Vec<CreatedArticleSummary>,
    /// 因 Slug 冲突被跳过的 Slug。
    skipped: Vec<String>,
    /// 因冲突自动改名的 Slug 映射。
    renamed: Vec<RenamedSlug>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct CreatedArticleSummary {
    id: i64,
    title: String,
    slug: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
struct RenamedSlug {
    from: String,
    to: String,
}

/// `POST /api/admin/articles/imports` 的 Multipart 表单；实际字段名为 `file[]`（兼容 `file`），
/// 1–10 个 `.md` 文件，单文件 ≤ 2MB。
#[derive(utoipa::ToSchema)]
#[allow(dead_code)]
struct ImportMarkdownForm {
    /// 待导入的 Markdown 文件（.md / .markdown）。
    #[schema(value_type = Vec<String>, format = Binary)]
    file: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/api/admin/articles/imports",
    tag = "Admin Media",
    operation_id = "importMarkdown",
    summary = "批量导入 Markdown 文章",
    description = "Multipart 字段名 `file[]`，1–10 个 `.md` 文件，单文件 ≤ 2MB；解析 YAML Front Matter（title/slug/tags/category/summary）并生成 Slug 冲突预览，结果存入 Background Job。",
    security(("cookieAuth" = [])),
    request_body(content = ImportMarkdownForm, content_type = "multipart/form-data", description = "待导入的 Markdown 文件批次"),
    responses(
        (status = 201, description = "导入预览已生成", body = ImportJobResponse),
        (status = 400, description = "文件类型/大小/编码不合法或 Multipart 非法", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
    )
)]
async fn import_markdown(
    State(state): State<AppState>,
    current: CurrentUser,
    multipart: Multipart,
) -> Result<(StatusCode, Json<ImportJobResponse>), ApiError> {
    current.require(Permission::ManageContent)?;
    let files = collect_multipart_files(multipart, MAX_IMPORT_FILES).await?;
    let mut items = Vec::with_capacity(files.len());
    for (index, file) in files.into_iter().enumerate() {
        if file.bytes.len() > MAX_IMPORT_BYTES {
            return Err(ApiError::bad_request(
                "FILE_TOO_LARGE",
                "Markdown file exceeds the 2MB size limit",
            ));
        }
        let mut item = parse_import_file(index, &file.original_name, &file.bytes)?;
        item.slug_conflict = state
            .content
            .find_article_by_slug(&item.slug)
            .await?
            .is_some();
        items.push(item);
    }
    let job = state
        .jobs
        .enqueue(NewBackgroundJob::new(
            JobKind::ImportMarkdown,
            serde_json::json!({ "items": items }),
        ))
        .await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "import.created".to_owned(),
            target_type: "import".to_owned(),
            target_id: Some(job.id.to_string()),
            metadata: serde_json::json!({ "count": items.len() }),
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ImportJobResponse {
            job_id: job.id,
            status: job.status.as_str(),
            items: items.iter().map(ImportPreview::from).collect(),
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/api/admin/imports/{id}",
    tag = "Admin Media",
    operation_id = "getImport",
    summary = "获取 Markdown 导入预览",
    description = "返回导入预览（标题、Slug、警告、冲突标记）；非导入类型的 Job 返回 404。",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "导入 Job ID"),
    ),
    responses(
        (status = 200, description = "导入预览", body = ImportJobResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "导入 Job 不存在", body = crate::openapi::ErrorResponse),
    )
)]
async fn get_import(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(job_id): Path<i64>,
) -> Result<Json<ImportJobResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let job = find_import_job(&state, job_id).await?;
    let items = parse_job_items(&job.payload)?;
    Ok(Json(ImportJobResponse {
        job_id: job.id,
        status: job.status.as_str(),
        items: items.iter().map(ImportPreview::from).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/admin/imports/{id}/commit",
    tag = "Admin Media",
    operation_id = "commitImport",
    summary = "提交 Markdown 导入",
    description = "按逐篇策略落库；冲突项未指定策略时一律 skip，绝不静默覆盖已有文章。Job 只允许 Commit 一次。",
    security(("cookieAuth" = [])),
    params(
        ("id" = i64, Path, description = "导入 Job ID"),
    ),
    request_body(description = "冲突项的逐篇处理策略", content = CommitImportRequest),
    responses(
        (status = 200, description = "导入完成，Job 置为 Done", body = CommitResultResponse),
        (status = 400, description = "策略非法（INVALID_IMPORT_STRATEGY）", body = crate::openapi::ErrorResponse),
        (status = 401, description = "未认证", body = crate::openapi::ErrorResponse),
        (status = 403, description = "无内容管理权限", body = crate::openapi::ErrorResponse),
        (status = 404, description = "导入 Job 不存在", body = crate::openapi::ErrorResponse),
        (status = 409, description = "Job 已处理过（IMPORT_ALREADY_COMMITTED / IMPORT_SLUG_EXHAUSTED）", body = crate::openapi::ErrorResponse),
    )
)]
async fn commit_import(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(job_id): Path<i64>,
    ApiJson(request): ApiJson<CommitImportRequest>,
) -> Result<Json<CommitResultResponse>, ApiError> {
    current.require(Permission::ManageContent)?;
    let job = find_import_job(&state, job_id).await?;
    if job.status != aries_core::jobs::JobStatus::Pending {
        return Err(ApiError::conflict(
            "IMPORT_ALREADY_COMMITTED",
            "Import job was already processed",
        ));
    }
    let mut strategies = std::collections::HashMap::new();
    for entry in request.items {
        if entry.strategy != "skip" && entry.strategy != "rename" {
            return Err(ApiError::bad_request(
                "INVALID_IMPORT_STRATEGY",
                "Import strategy must be skip or rename",
            ));
        }
        strategies.insert(entry.index, entry.strategy);
    }
    let items = parse_job_items(&job.payload)?;

    // 本次批次内已占用的 Slug 也要计入冲突，避免批内互撞。
    let mut taken_slugs: HashSet<String> = HashSet::new();
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    let mut renamed = Vec::new();
    for item in &items {
        let mut slug = item.slug.clone();
        let conflicts = taken_slugs.contains(&slug.to_lowercase())
            || state.content.find_article_by_slug(&slug).await?.is_some();
        if conflicts {
            match strategies.get(&item.index).map(String::as_str) {
                Some("rename") => {
                    let renamed_slug = find_free_slug(&state, &taken_slugs, &slug).await?;
                    renamed.push(RenamedSlug {
                        from: slug,
                        to: renamed_slug.clone(),
                    });
                    slug = renamed_slug;
                }
                // 未显式指定策略的冲突项一律跳过，绝不静默覆盖已有文章。
                _ => {
                    skipped.push(slug);
                    continue;
                }
            }
        }
        taken_slugs.insert(slug.to_lowercase());

        let category_id = match item.category.as_deref() {
            Some(name) => resolve_category(&state, name).await?,
            None => None,
        };
        let tag_ids = resolve_tag_ids(&state, &item.tags).await?;
        let rendered_html = render_markdown(&state, item.markdown_source.clone()).await?;
        let article = state
            .content
            .create_article(NewArticle {
                author_id: current.user.id,
                category_id,
                slug: slug.clone(),
                title: item.title.clone(),
                summary: item.summary.clone(),
                ai_brief: None,
                cover_url: None,
                markdown_source: item.markdown_source.clone(),
                rendered_html,
                seo_keywords: Vec::new(),
                access_password_hash: None,
                allow_comments: true,
                is_pinned: false,
                tag_ids,
            })
            .await?;
        // 导入的正文可能引用已上传的媒体，引用关系与正常编辑流程一致重建。
        sync_article_media_usages(&state, article.id, &article.markdown_source, None).await;
        created.push(CreatedArticleSummary {
            id: article.id,
            title: article.title,
            slug,
        });
    }

    state.jobs.complete(job.id).await?;
    state
        .auth
        .write_audit(AuditEvent {
            actor_user_id: Some(current.user.id),
            action: "import.commit".to_owned(),
            target_type: "import".to_owned(),
            target_id: Some(job.id.to_string()),
            metadata: serde_json::json!({
                "created": created.len(),
                "skipped": skipped.len(),
                "renamed": renamed.len(),
            }),
        })
        .await?;
    Ok(Json(CommitResultResponse {
        created,
        skipped,
        renamed,
    }))
}

async fn find_import_job(
    state: &AppState,
    job_id: i64,
) -> Result<aries_core::jobs::BackgroundJob, ApiError> {
    let job = state
        .jobs
        .find(job_id)
        .await?
        .ok_or_else(|| ApiError::not_found("IMPORT_NOT_FOUND", "Import job was not found"))?;
    if job.kind != JobKind::ImportMarkdown {
        return Err(ApiError::not_found(
            "IMPORT_NOT_FOUND",
            "Import job was not found",
        ));
    }
    Ok(job)
}

fn parse_job_items(payload: &serde_json::Value) -> Result<Vec<ImportItem>, ApiError> {
    serde_json::from_value(
        payload
            .get("items")
            .cloned()
            .ok_or_else(ApiError::internal)?,
    )
    .map_err(|_| ApiError::internal())
}

/// 解析单个 Markdown 导入文件：YAML Front Matter 可选，缺失字段回退默认值并记 Warning。
fn parse_import_file(index: usize, file_name: &str, bytes: &[u8]) -> Result<ImportItem, ApiError> {
    let extension_ok = file_name
        .rsplit('.')
        .next()
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown"));
    if !extension_ok {
        return Err(ApiError::bad_request_with_details(
            "UNSUPPORTED_FILE_TYPE",
            "Only Markdown files can be imported",
            serde_json::json!({ "file": file_name }),
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ApiError::bad_request_with_details(
            "INVALID_MARKDOWN",
            "Markdown file is not valid UTF-8",
            serde_json::json!({ "file": file_name }),
        )
    })?;

    let (front_matter, body) = split_front_matter(text);
    let mut warnings = Vec::new();
    let mut title = None;
    let mut slug = None;
    let mut summary = String::new();
    let mut tags = Vec::new();
    let mut category = None;

    if let Some(yaml) = front_matter {
        match serde_yaml::from_str::<serde_yaml::Value>(yaml) {
            Ok(serde_yaml::Value::Mapping(mapping)) => {
                title = yaml_string(mapping.get("title"));
                slug = yaml_string(mapping.get("slug"));
                summary = yaml_string(mapping.get("summary")).unwrap_or_default();
                category = yaml_string(mapping.get("category"));
                tags = yaml_string_list(mapping.get("tags"));
            }
            // Front Matter 解析失败不阻断导入，按无元数据处理并记 Warning。
            _ => warnings.push("front matter could not be parsed".to_owned()),
        }
    }

    let fallback_title = file_name
        .rsplit('.')
        .next_back()
        .unwrap_or(file_name)
        .to_owned();
    let title = title
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            warnings.push("title missing, file name used".to_owned());
            fallback_title
        });
    let slug = slug
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| title.clone());
    let slug = normalize_slug(&slug).unwrap_or_else(|_| {
        warnings.push("slug invalid, a random slug was generated".to_owned());
        format!("imported-{}", Uuid::now_v7().simple())
    });

    Ok(ImportItem {
        index,
        file_name: file_name.to_owned(),
        title: title.trim().to_owned(),
        slug,
        summary: summary.trim().to_owned(),
        tags,
        category,
        markdown_source: body.to_owned(),
        warnings,
        slug_conflict: false,
    })
}

/// 拆分 YAML Front Matter 与正文；无 Front Matter 时整个文本视为正文。
fn split_front_matter(text: &str) -> (Option<&str>, &str) {
    let text = text.strip_prefix("\u{feff}").unwrap_or(text);
    let Some(rest) = text.strip_prefix("---") else {
        return (None, text);
    };
    // 起始分隔符后必须是换行，否则不是 Front Matter（如水平线）。
    let Some(rest) = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
    else {
        return (None, text);
    };
    for (offset, _) in rest.match_indices("\n---") {
        let end = offset;
        let after = &rest[end + 4..];
        if after.is_empty() || after.starts_with('\n') || after.starts_with("\r\n") {
            let body = after
                .strip_prefix("\r\n")
                .or_else(|| after.strip_prefix('\n'))
                .unwrap_or(after);
            // CRLF 文档中分隔行是 "\r\n---"，元数据末尾的 '\r' 要剥掉。
            let meta = rest[..end].strip_suffix('\r').unwrap_or(&rest[..end]);
            return (Some(meta), body);
        }
    }
    (None, text)
}

fn yaml_string(value: Option<&serde_yaml::Value>) -> Option<String> {
    match value {
        Some(serde_yaml::Value::String(text)) => Some(text.trim().to_owned()),
        _ => None,
    }
}

/// tags 同时容忍 YAML 列表与逗号分隔字符串两种写法（旧版导出两种都有）。
fn yaml_string_list(value: Option<&serde_yaml::Value>) -> Vec<String> {
    match value {
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::trim))
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(serde_yaml::Value::String(text)) => text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// 追加 -2、-3… 直到 Slug 可用；批内占用与库内占用都检查。
async fn find_free_slug(
    state: &AppState,
    taken: &HashSet<String>,
    base: &str,
) -> Result<String, ApiError> {
    for suffix in 2..=100 {
        let candidate = format!("{base}-{suffix}");
        if taken.contains(&candidate.to_lowercase()) {
            continue;
        }
        if state
            .content
            .find_article_by_slug(&candidate)
            .await?
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(ApiError::conflict(
        "IMPORT_SLUG_EXHAUSTED",
        "Could not find a free slug for the imported article",
    ))
}

async fn resolve_category(state: &AppState, name: &str) -> Result<Option<i64>, ApiError> {
    // Markdown 导入只解析文章分类。
    let categories = state
        .content
        .list_categories(aries_core::content::CategoryKind::Article)
        .await?;
    // 分类不自动创建：层级结构由管理员维护，找不到就置空。
    Ok(categories
        .into_iter()
        .find(|category| category.name.eq_ignore_ascii_case(name.trim()))
        .map(|category| category.id))
}

async fn resolve_tag_ids(state: &AppState, names: &[String]) -> Result<Vec<i64>, ApiError> {
    let mut existing = state.content.list_tags().await?;
    let mut ids = Vec::new();
    for name in names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
    {
        if let Some(tag) = existing
            .iter()
            .find(|tag| tag.name.eq_ignore_ascii_case(name))
        {
            ids.push(tag.id);
            continue;
        }
        // 标签是扁平结构，导入时自动创建缺失标签比静默丢弃更贴近旧版行为。
        let slug =
            normalize_slug(name).unwrap_or_else(|_| format!("tag-{}", Uuid::now_v7().simple()));
        match state
            .content
            .create_tag(NewTag {
                name: name.to_owned(),
                slug,
            })
            .await
        {
            Ok(tag) => {
                ids.push(tag.id);
                existing.push(tag);
            }
            // 并发或 Slug 撞车：重新拉取后按名称兜底匹配。
            Err(ContentError::Conflict) => {
                existing = state.content.list_tags().await?;
                if let Some(tag) = existing
                    .iter()
                    .find(|tag| tag.name.eq_ignore_ascii_case(name))
                {
                    ids.push(tag.id);
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

// ---------------------------------------------------------------------------
// 文章引用维护与静态文件服务
// ---------------------------------------------------------------------------

/// 文章创建/更新后全量重建该文章的媒体引用。
/// 失败只记录日志不阻塞文章写入：引用表是派生数据，可由后续保存或后台任务修复。
pub async fn sync_article_media_usages(
    state: &AppState,
    article_id: i64,
    markdown_source: &str,
    cover_url: Option<&str>,
) {
    let base_url = state.config.media_public_base_url.clone();
    let mut refs = extract_media_usage_refs(markdown_source, cover_url, &base_url);
    // 配置为绝对 URL（如 CDN）时，站内相对前缀也要覆盖，保证两种写法都被追踪。
    if base_url != aries_infra::storage::DEFAULT_PUBLIC_BASE_URL {
        let relative = extract_media_usage_refs(
            markdown_source,
            cover_url,
            aries_infra::storage::DEFAULT_PUBLIC_BASE_URL,
        );
        if refs.cover.is_none() {
            refs.cover = relative.cover;
        }
        for key in relative.content {
            if !refs.content.contains(&key) {
                refs.content.push(key);
            }
        }
    }

    let mut keys = refs.content.clone();
    if let Some(cover) = &refs.cover {
        keys.push(cover.clone());
    }
    let assets = match state.media.find_by_object_keys(&keys).await {
        Ok(assets) => assets,
        Err(error) => {
            tracing::error!(error = %error, article_id, "failed to resolve media usage assets");
            return;
        }
    };
    let cover_asset_id = refs
        .cover
        .as_ref()
        .and_then(|key| assets.iter().find(|asset| &asset.object_key == key))
        .map(|asset| asset.id);
    let content_asset_ids = refs
        .content
        .iter()
        .filter_map(|key| assets.iter().find(|asset| &asset.object_key == key))
        .map(|asset| asset.id)
        .collect::<Vec<_>>();
    if let Err(error) = state
        .media
        .replace_article_usages(article_id, cover_asset_id, &content_asset_ids)
        .await
    {
        tracing::error!(error = %error, article_id, "failed to sync article media usages");
    }
}

/// `GET /api/media/files/{*path}`：本地存储直接发文件，远端存储 Redirect 到公开 URL。
/// `?w=<width>` 请求缩略图（16–1200）：仅本地存储的 jpeg/png/webp 生效，
/// 首次请求生成并落盘缓存（见 `aries_infra::thumbnail`）；其余情况静默回退原图。
#[utoipa::path(
    get,
    path = "/api/media/files/{path}",
    tag = "Public Media",
    operation_id = "serveMediaFile",
    summary = "匿名访问媒体文件",
    description = "Local Provider 直接发文件（`Cache-Control: public, max-age=31536000, immutable`），S3 Provider 307 Redirect 到公开 URL。Path 做路径穿越校验。`?w=<width>` 请求按需缩略图（16–1200，仅 Local Provider 的 jpeg/png/webp，首次生成后落盘缓存且同样 immutable）；缺省、非法值或不支持的类型静默回退原图。",
    params(
        ("path" = String, Path, description = "媒体 Object Key（如 `2026/09/<uuid>.png`）"),
        MediaFileParams,
    ),
    responses(
        (status = 200, description = "文件内容；Content-Type 按入库时的 MIME", content_type = "application/octet-stream"),
        (status = 307, description = "S3 Provider 时 Redirect 到公开 URL"),
        (status = 404, description = "文件不存在或路径非法", body = crate::openapi::ErrorResponse),
    )
)]
pub async fn serve_media_file(
    State(state): State<AppState>,
    Path(path): Path<String>,
    Query(params): Query<MediaFileParams>,
) -> Result<Response, ApiError> {
    // 拒绝路径穿越与绝对路径；对客户端统一表现为 404，不暴露路径细节。
    let not_found = || ApiError::not_found("MEDIA_NOT_FOUND", "Media file was not found");
    aries_infra::storage::validate_object_key(&path).map_err(|_| not_found())?;
    let asset = state
        .media
        .find_by_object_keys(std::slice::from_ref(&path))
        .await?
        .into_iter()
        .next()
        .ok_or_else(not_found)?;
    if !state.storage.serves_files_locally() {
        return Ok(Redirect::temporary(&asset.url).into_response());
    }
    let bytes = state.storage.get(&path).await?.ok_or_else(not_found)?;
    if let Some(width) = aries_infra::thumbnail::parse_width(params.w.as_deref()) {
        if aries_infra::thumbnail::is_resizable_image(&asset.mime) {
            let original = bytes.to_vec();
            let mime = asset.mime.clone();
            let object_key = path.clone();
            // 解码/缩放是 CPU 密集操作，放阻塞线程池，避免拖慢 async executor
            let thumbnail = tokio::task::spawn_blocking(move || {
                aries_infra::thumbnail::get_or_create(
                    &aries_infra::thumbnail::cache_root(),
                    &object_key,
                    &original,
                    &mime,
                    width,
                )
            })
            .await
            .map_err(ApiError::internal_logged)?
            .map_err(ApiError::internal_logged)?;
            // Object Key 不可变且宽度固定，缩略图缓存条目同样不可变，适合长缓存。
            return Ok((
                [
                    (header::CONTENT_TYPE, thumbnail.mime.to_owned()),
                    (
                        header::CACHE_CONTROL,
                        "public, max-age=31536000, immutable".to_owned(),
                    ),
                ],
                thumbnail.bytes,
            )
                .into_response());
        }
    }
    // Object Key 不可变且内容不变，适合长缓存。
    Ok((
        [
            (header::CONTENT_TYPE, asset.mime),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_owned(),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MediaFileParams {
    /// 缩略图目标宽度（16–1200 的整数）；缺省或非法时返回原图。
    pub w: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_url_rejects_non_http_schemes_and_internal_addresses() {
        assert!(validate_remote_url("https://example.com/a.png").is_ok());
        assert!(validate_remote_url("file:///etc/passwd").is_err());
        assert!(validate_remote_url("ftp://example.com/a.png").is_err());
        assert!(validate_remote_url("http://127.0.0.1/a.png").is_err());
        assert!(validate_remote_url("http://10.0.0.8/a.png").is_err());
        assert!(validate_remote_url("http://172.16.3.1/a.png").is_err());
        assert!(validate_remote_url("http://192.168.1.1/a.png").is_err());
        assert!(validate_remote_url("http://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_remote_url("http://[::1]/a.png").is_err());
        assert!(validate_remote_url("http://localhost/admin").is_err());
        assert!(validate_remote_url("not a url").is_err());
    }

    #[test]
    fn forbidden_ip_covers_v4_ranges_and_v6_mapped() {
        assert!(is_forbidden_ip("127.0.0.1".parse().unwrap()));
        assert!(is_forbidden_ip("10.1.2.3".parse().unwrap()));
        assert!(is_forbidden_ip("172.31.255.1".parse().unwrap()));
        assert!(is_forbidden_ip("192.168.0.1".parse().unwrap()));
        assert!(is_forbidden_ip("169.254.1.1".parse().unwrap()));
        assert!(is_forbidden_ip("0.0.0.0".parse().unwrap()));
        assert!(is_forbidden_ip("::1".parse().unwrap()));
        assert!(is_forbidden_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_forbidden_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_forbidden_ip("172.15.0.1".parse().unwrap()));
    }

    #[test]
    fn front_matter_split_handles_crlf_and_missing_markers() {
        let (meta, body) = split_front_matter("---\ntitle: Hello\n---\nbody text");
        assert_eq!(meta, Some("title: Hello"));
        assert_eq!(body, "body text");

        let (meta, body) = split_front_matter("---\r\ntitle: A\r\n---\r\nbody");
        assert_eq!(meta, Some("title: A"));
        assert_eq!(body, "body");

        assert_eq!(
            split_front_matter("no front matter"),
            (None, "no front matter")
        );
        // 水平线不是 Front Matter。
        assert_eq!(
            split_front_matter("---\nnot yaml\n正文"),
            (None, "---\nnot yaml\n正文")
        );
    }

    #[test]
    fn import_file_parses_front_matter_and_falls_back_to_file_name() {
        let bytes = b"---\ntitle: Hello\nslug: hello-world\ntags:\n  - rust\n  - vue\ncategory: Tech\nsummary: hi\n---\n# Content";
        let item = parse_import_file(0, "post.md", bytes).unwrap();
        assert_eq!(item.title, "Hello");
        assert_eq!(item.slug, "hello-world");
        assert_eq!(item.tags, vec!["rust".to_owned(), "vue".to_owned()]);
        assert_eq!(item.category.as_deref(), Some("Tech"));
        assert_eq!(item.summary, "hi");
        assert_eq!(item.markdown_source, "# Content");

        let fallback = parse_import_file(1, "我的笔记.md", b"plain body").unwrap();
        assert_eq!(fallback.title, "我的笔记");
        assert!(
            fallback
                .warnings
                .iter()
                .any(|w| w.contains("title missing"))
        );
        assert!(parse_import_file(2, "note.txt", b"x").is_err());
    }
}
