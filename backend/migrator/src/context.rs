//! Migrate 子命令的共享上下文与跨表公共逻辑。

use std::path::PathBuf;

use aries_infra::ComrakMarkdownRenderer;
use sqlx::{MySqlPool, PgPool};
use time::UtcOffset;

use crate::{archive::ArchiveWriter, report::RunReport};

/// 媒体下载上限（20MB）：超出视为失败，保留 legacy_url 外链记录。
pub const MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;
/// 媒体下载超时（秒）。
pub const DOWNLOAD_TIMEOUT_SECS: u64 = 15;
/// 媒体下载最大重试次数（不含首次尝试）。
pub const DOWNLOAD_MAX_RETRIES: u32 = 2;

pub struct MediaOptions {
    pub skip_download: bool,
    /// 下载成功后的写盘目录（对应 server 的 MEDIA_LOCAL_DIR）。
    pub media_dir: PathBuf,
    /// 旧站地址，用于把相对 URL 拼接为绝对 URL 后再下载。
    pub source_base_url: Option<String>,
    /// 目标站媒体公开前缀（对应 server 的 MEDIA_PUBLIC_BASE_URL）。
    pub public_base_url: String,
    pub client: reqwest::Client,
}

pub struct MigrateCtx {
    pub mysql: MySqlPool,
    pub pg: PgPool,
    pub renderer: ComrakMarkdownRenderer,
    pub offset: UtcOffset,
    pub truncate_violations: bool,
    pub batch_size: usize,
    pub archive: ArchiveWriter,
    pub report: RunReport,
    /// `--owner-ids` 显式指定的 owner 账号；为空时取最小 id 账号。
    pub owner_ids: Vec<i64>,
    pub(crate) owner_id: Option<i64>,
    pub media: MediaOptions,
}

impl MigrateCtx {
    /// 记录一行 Archive，同时累计 Report 的 archived 计数（两个去向保持同步）。
    /// `whole_row` 为 true 时同时累计 archived_rows（整行未迁入，参与对账）。
    pub fn archive_row(
        &mut self,
        table: &str,
        id: Option<i64>,
        fields: serde_json::Value,
        reason: &str,
        whole_row: bool,
    ) -> anyhow::Result<()> {
        self.archive.record(table, id, fields, reason, whole_row)?;
        let report = self.report.table(table);
        report.archived += 1;
        if whole_row {
            report.archived_rows += 1;
        }
        Ok(())
    }

    pub fn set_owner_id(&mut self, owner_id: i64) {
        self.owner_id = Some(owner_id);
    }

    /// media_assets.uploaded_by 需要一个已迁移用户；`--only` 跳过 users 时回退到目标库最小 id。
    pub async fn owner_id(&mut self) -> anyhow::Result<i64> {
        if let Some(id) = self.owner_id {
            return Ok(id);
        }
        let id: Option<i64> = sqlx::query_scalar("SELECT id FROM users ORDER BY id LIMIT 1")
            .fetch_optional(&self.pg)
            .await?;
        let id = id.ok_or_else(|| {
            anyhow::anyhow!(
                "no migrated user found; run the users step first or include it in --only"
            )
        })?;
        self.owner_id = Some(id);
        Ok(id)
    }
}

/// 把绝对/相对 URL 归一化为可下载的绝对 URL；无 base 时返回 None。
pub fn resolve_source_url(url: &str, source_base_url: Option<&str>) -> Option<String> {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(url.to_owned());
    }
    let base = source_base_url?.trim_end_matches('/');
    if url.starts_with('/') {
        Some(format!("{base}{url}"))
    } else {
        Some(format!("{base}/{url}"))
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_source_url;

    #[test]
    fn resolves_absolute_and_relative_urls() {
        assert_eq!(
            resolve_source_url("https://a.com/x.png", None).as_deref(),
            Some("https://a.com/x.png")
        );
        assert_eq!(
            resolve_source_url("/upload/x.png", Some("https://a.com/")).as_deref(),
            Some("https://a.com/upload/x.png")
        );
        assert_eq!(
            resolve_source_url("upload/x.png", Some("https://a.com")).as_deref(),
            Some("https://a.com/upload/x.png")
        );
        assert_eq!(resolve_source_url("/x.png", None), None);
    }
}
