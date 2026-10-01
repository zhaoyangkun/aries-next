//! `pictures` → `media_assets`：默认尝试下载，成功写盘 + sha256 + provider='local'；
//! 失败保留 provider='legacy_url' 外链记录并在 Report 逐条列出。`size` KB → bytes。

use anyhow::Context;
use sqlx::{FromRow, MySqlPool, PgPool};
use time::OffsetDateTime;

use aries_infra::upload::{ImageKind, generate_object_key, probe_dimensions, sha256_hex};

use super::{optional_time, required_time};
use crate::context::{
    DOWNLOAD_MAX_RETRIES, MAX_DOWNLOAD_BYTES, MediaOptions, MigrateCtx, resolve_source_url,
};

pub const TABLE: &str = "media_assets";

#[derive(Debug, Clone, FromRow)]
pub struct LegacyPicture {
    pub id: i64,
    pub storage_type: String,
    pub hash: String,
    pub file_name: String,
    pub url: String,
    /// 旧库单位 KB。
    pub size: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewMediaAsset {
    pub id: i64,
    pub provider: String,
    pub object_key: String,
    pub url: String,
    pub original_name: String,
    pub mime: String,
    pub size_bytes: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub sha256: String,
    pub status: String,
    pub uploaded_by: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyPicture>> {
    sqlx::query_as::<_, LegacyPicture>(
        "SELECT CAST(id AS SIGNED) AS id, storage_type, hash, file_name, url, \
         CAST(size AS SIGNED) AS size, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM pictures ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy pictures")
}

/// 从文件名扩展名猜 MIME；失败回退 application/octet-stream。
pub fn guess_mime(file_name: &str) -> String {
    file_name
        .rsplit('.')
        .next()
        .filter(|ext| ext.len() < file_name.len())
        .and_then(ImageKind::from_extension)
        .map_or_else(
            || "application/octet-stream".to_owned(),
            |kind| kind.mime().to_owned(),
        )
}

/// 下载图片：超时由 client 控制，限制大小，最多重试 DOWNLOAD_MAX_RETRIES 次。
async fn download_image(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let mut last_error = None;
    for attempt in 0..=DOWNLOAD_MAX_RETRIES {
        match try_download_once(client, url).await {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                tracing::warn!(url, attempt, error = %error, "media download attempt failed");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("download failed")))
}

async fn try_download_once(client: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let response = client.get(url).send().await?.error_for_status()?;
    if let Some(length) = response.content_length()
        && length as usize > MAX_DOWNLOAD_BYTES
    {
        anyhow::bail!("content-length {length} exceeds limit {MAX_DOWNLOAD_BYTES}");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        anyhow::bail!(
            "body {} bytes exceeds limit {MAX_DOWNLOAD_BYTES}",
            bytes.len()
        );
    }
    Ok(bytes.to_vec())
}

/// legacy_url 外链记录的 sha256：优先用旧库 hash（64 位十六进制），否则对 URL 取摘要占位。
pub fn legacy_sha256(legacy_hash: &str, url: &str) -> String {
    let valid = legacy_hash.len() == 64 && legacy_hash.bytes().all(|b| b.is_ascii_hexdigit());
    if valid {
        legacy_hash.to_ascii_lowercase()
    } else {
        sha256_hex(url.as_bytes())
    }
}

/// build_media_asset 的输入参数（避免超长参数列表）。
pub struct MediaAssetInput<'a> {
    pub id: i64,
    pub legacy_object_prefix: &'a str,
    pub source_url: &'a str,
    pub file_name: &'a str,
    pub size_kb: i64,
    pub legacy_hash: &'a str,
    pub uploaded_by: i64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

/// 构造一条 media_assets 行：能下载则落盘为 local，否则保留 legacy_url 外链。
/// 返回（行，失败/降级备注）。
pub async fn build_media_asset(
    media: &MediaOptions,
    input: MediaAssetInput<'_>,
) -> (NewMediaAsset, Option<String>) {
    let MediaAssetInput {
        id,
        legacy_object_prefix,
        source_url,
        file_name,
        size_kb,
        legacy_hash,
        uploaded_by,
        created_at,
        updated_at,
        deleted_at,
    } = input;
    let status = if deleted_at.is_some() {
        "deleted"
    } else {
        "active"
    };
    let original_name = if file_name.trim().is_empty() {
        format!("{legacy_object_prefix}-{id}")
    } else {
        file_name.to_owned()
    };
    let absolute_url = resolve_source_url(source_url, media.source_base_url.as_deref());

    if !media.skip_download
        && let Some(url) = absolute_url.as_deref()
        && let Ok(bytes) = download_image(&media.client, url).await
        && let Some(kind) = ImageKind::detect(&bytes)
    {
        let object_key = generate_object_key(kind.extension());
        let path = media.media_dir.join(&object_key);
        let write_result = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &bytes)
        })();
        if write_result.is_ok() {
            let (width, height) =
                probe_dimensions(&bytes).map_or((None, None), |(w, h)| (Some(w), Some(h)));
            let public_url = format!(
                "{}/{}",
                media.public_base_url.trim_end_matches('/'),
                object_key
            );
            return (
                NewMediaAsset {
                    id,
                    provider: "local".to_owned(),
                    object_key,
                    url: public_url,
                    original_name,
                    mime: kind.mime().to_owned(),
                    size_bytes: bytes.len() as i64,
                    width,
                    height,
                    sha256: sha256_hex(&bytes),
                    status: status.to_owned(),
                    uploaded_by,
                    created_at,
                    updated_at,
                    deleted_at,
                },
                None,
            );
        }
    }

    // 失败/跳过路径：保留外链记录，url 尽量用绝对形式。
    let url = absolute_url.unwrap_or_else(|| source_url.to_owned());
    let note = if media.skip_download {
        format!("#{id}: download disabled (--skip-media-download), kept as legacy_url: {url}")
    } else {
        format!("#{id}: download failed or not an image, kept as legacy_url: {url}")
    };
    (
        NewMediaAsset {
            id,
            provider: "legacy_url".to_owned(),
            object_key: format!("legacy/{legacy_object_prefix}/{id}"),
            url,
            original_name,
            mime: guess_mime(file_name),
            size_bytes: size_kb.max(0) * 1024,
            width: None,
            height: None,
            sha256: legacy_sha256(legacy_hash, source_url),
            status: status.to_owned(),
            uploaded_by,
            created_at,
            updated_at,
            deleted_at,
        },
        Some(note),
    )
}

/// 分批插入 media_assets（幂等）；galleries 步骤产出的媒体行也走这里。
pub async fn insert_media_assets(
    pg: &PgPool,
    rows: &[NewMediaAsset],
    batch_size: usize,
) -> anyhow::Result<(i64, i64)> {
    let mut migrated = 0i64;
    let mut skipped = 0i64;
    for chunk in rows.chunks(batch_size.max(1)) {
        let mut tx = pg.begin().await?;
        for row in chunk {
            let affected = sqlx::query(
                "INSERT INTO media_assets (id, provider, object_key, url, original_name, mime, \
                 size_bytes, width, height, sha256, status, uploaded_by, created_at, updated_at, deleted_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(&row.provider)
            .bind(&row.object_key)
            .bind(&row.url)
            .bind(&row.original_name)
            .bind(&row.mime)
            .bind(row.size_bytes)
            .bind(row.width)
            .bind(row.height)
            .bind(&row.sha256)
            .bind(&row.status)
            .bind(row.uploaded_by)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert media_asset #{}", row.id))?
            .rows_affected();
            if affected == 1 {
                migrated += 1;
            } else {
                skipped += 1;
            }
        }
        tx.commit().await?;
    }
    Ok((migrated, skipped))
}

pub async fn migrate(ctx: &mut MigrateCtx) -> anyhow::Result<()> {
    let rows = extract(&ctx.mysql).await?;
    ctx.report.table(TABLE).source_rows = rows.len() as i64;
    let owner_id = ctx.owner_id().await?;

    let mut new_rows = Vec::with_capacity(rows.len());
    let mut legacy_notes = Vec::new();
    for row in &rows {
        let created_at = required_time(
            row.created_at.as_deref(),
            ctx.offset,
            TABLE,
            row.id,
            "created_at",
        )?;
        let updated_at = required_time(
            row.updated_at.as_deref(),
            ctx.offset,
            TABLE,
            row.id,
            "updated_at",
        )?;
        let deleted_at = optional_time(
            row.deleted_at.as_deref(),
            ctx.offset,
            TABLE,
            row.id,
            "deleted_at",
        )?;
        let (asset, note) = build_media_asset(
            &ctx.media,
            MediaAssetInput {
                id: row.id,
                legacy_object_prefix: "pictures",
                source_url: &row.url,
                file_name: &row.file_name,
                size_kb: row.size,
                legacy_hash: &row.hash,
                uploaded_by: owner_id,
                created_at,
                updated_at,
                deleted_at,
            },
        )
        .await;
        if let Some(note) = note {
            legacy_notes.push(note);
        }
        new_rows.push(asset);
    }

    let (migrated, skipped) = insert_media_assets(&ctx.pg, &new_rows, ctx.batch_size).await?;

    let legacy_count = new_rows
        .iter()
        .filter(|row| row.provider == "legacy_url")
        .count() as i64;
    // 旧图床类型分布（storage_type 只作溯源信息，不影响目标映射）。
    let mut storage_types: std::collections::BTreeMap<String, i64> = Default::default();
    for row in &rows {
        *storage_types.entry(row.storage_type.clone()).or_default() += 1;
    }
    let report = ctx.report.table(TABLE);
    report.migrated = migrated;
    report.skipped = skipped;
    // legacy_url 降级视为修复（数据保留为外链而非落盘）。
    report.repaired += legacy_count;
    report.note(format!(
        "legacy storage_type distribution: {storage_types:?}"
    ));
    for note in legacy_notes {
        report.note(note);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sha256_prefers_valid_legacy_hash() {
        let hash = "a".repeat(64);
        assert_eq!(legacy_sha256(&hash, "https://a.com/x.png"), hash);
        // 非 64 位十六进制回退为 URL 摘要。
        let fallback = legacy_sha256("not-a-hash", "https://a.com/x.png");
        assert_eq!(fallback.len(), 64);
        assert_eq!(fallback, sha256_hex(b"https://a.com/x.png"));
    }

    #[test]
    fn guess_mime_from_extension() {
        assert_eq!(guess_mime("a.jpg"), "image/jpeg");
        assert_eq!(guess_mime("a.PNG"), "image/png");
        assert_eq!(guess_mime("no-ext"), "application/octet-stream");
        assert_eq!(guess_mime("a.svg"), "application/octet-stream");
    }

    /// 最小 HTTP/1.1 静态响应服务器：验证下载成功路径（local provider + 落盘）。
    struct StaticServer {
        url: String,
        handle: std::thread::JoinHandle<()>,
    }

    impl StaticServer {
        fn serve_once(body: &'static [u8], content_type: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let handle = std::thread::spawn(move || {
                if let Ok((mut stream, _)) = listener.accept() {
                    use std::io::{Read, Write};
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf);
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                }
            });
            Self {
                url: format!("http://127.0.0.1:{port}/img.png"),
                handle,
            }
        }
    }

    fn media_options(dir: &std::path::Path, skip: bool) -> MediaOptions {
        MediaOptions {
            skip_download: skip,
            media_dir: dir.to_path_buf(),
            source_base_url: None,
            public_base_url: "/api/media/files".to_owned(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
        }
    }

    fn input<'a>(url: &'a str) -> MediaAssetInput<'a> {
        MediaAssetInput {
            id: 42,
            legacy_object_prefix: "pictures",
            source_url: url,
            file_name: "img.png",
            size_kb: 1,
            legacy_hash: "",
            uploaded_by: 1,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
            deleted_at: None,
        }
    }

    const PNG_BYTES: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG magic + 填充
        0x00, 0x00, 0x00, 0x0D,
    ];

    #[tokio::test]
    async fn successful_download_writes_local_asset() {
        let server = StaticServer::serve_once(PNG_BYTES, "image/png");
        let dir =
            std::env::temp_dir().join(format!("aries-migrator-test-{}", uuid::Uuid::now_v7()));
        let options = media_options(&dir, false);

        let (asset, note) = build_media_asset(&options, input(&server.url)).await;
        server.handle.join().unwrap();

        assert_eq!(asset.provider, "local");
        assert!(note.is_none());
        assert_eq!(asset.mime, "image/png");
        assert_eq!(asset.size_bytes, PNG_BYTES.len() as i64);
        assert_eq!(asset.sha256, sha256_hex(PNG_BYTES));
        assert!(asset.url.starts_with("/api/media/files/"));
        // 文件按 object_key 约定落盘。
        let written = dir.join(&asset.object_key);
        assert_eq!(std::fs::read(&written).unwrap(), PNG_BYTES);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn non_image_response_falls_back_to_legacy_url() {
        let server = StaticServer::serve_once(b"not an image", "text/plain");
        let dir =
            std::env::temp_dir().join(format!("aries-migrator-test-{}", uuid::Uuid::now_v7()));
        let options = media_options(&dir, false);

        let (asset, note) = build_media_asset(&options, input(&server.url)).await;
        server.handle.join().unwrap();

        assert_eq!(asset.provider, "legacy_url");
        assert!(note.is_some());
        assert_eq!(asset.object_key, "legacy/pictures/42");
        assert_eq!(asset.url, server.url);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn skip_download_never_calls_network() {
        let dir =
            std::env::temp_dir().join(format!("aries-migrator-test-{}", uuid::Uuid::now_v7()));
        let options = media_options(&dir, true);
        let (asset, note) =
            build_media_asset(&options, input("http://127.0.0.1:1/unreachable.png")).await;
        assert_eq!(asset.provider, "legacy_url");
        assert!(note.unwrap().contains("--skip-media-download"));
    }
}
