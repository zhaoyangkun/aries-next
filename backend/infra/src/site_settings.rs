use crate::logged::logged_query_as;
use aries_core::media::{MediaError, SiteSettings, SiteSettingsRepository, SiteSettingsUpdate};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresSiteSettingsRepository {
    pool: PgPool,
}

impl PostgresSiteSettingsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, FromRow)]
struct SiteSettingsRow {
    site_name: String,
    site_description: String,
    site_url: String,
    logo_url: String,
    icp_text: String,
    default_cover_url: String,
    page_size_index: i32,
    page_size_archive: i32,
    page_size_search: i32,
    comment_policy: String,
    comments_per_page: i32,
    updated_at: OffsetDateTime,
}

impl TryFrom<SiteSettingsRow> for SiteSettings {
    type Error = MediaError;

    fn try_from(row: SiteSettingsRow) -> Result<Self, Self::Error> {
        Ok(Self {
            site_name: row.site_name,
            site_description: row.site_description,
            site_url: row.site_url,
            logo_url: row.logo_url,
            icp_text: row.icp_text,
            default_cover_url: row.default_cover_url,
            page_size_index: row.page_size_index,
            page_size_archive: row.page_size_archive,
            page_size_search: row.page_size_search,
            comment_policy: row
                .comment_policy
                .parse()
                .map_err(|_| MediaError::InvalidCommentPolicy)?,
            comments_per_page: row.comments_per_page,
            updated_at: row.updated_at,
        })
    }
}

const SETTINGS_COLUMNS: &str = "site_name, site_description, site_url, logo_url, icp_text, \
    default_cover_url, page_size_index, page_size_archive, page_size_search, \
    comment_policy, comments_per_page, updated_at";

#[async_trait]
impl SiteSettingsRepository for PostgresSiteSettingsRepository {
    async fn get(&self) -> Result<SiteSettings, MediaError> {
        // 设置表由 Migration 保证恰好一行（id = 1），缺失属于部署问题而非客户端错误。
        let query = format!("SELECT {SETTINGS_COLUMNS} FROM site_settings WHERE id = 1");
        let row = logged_query_as::<SiteSettingsRow>(&query)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(MediaError::StoreUnavailable)?;
        row.try_into()
    }

    async fn update(&self, update: SiteSettingsUpdate) -> Result<SiteSettings, MediaError> {
        let query = format!(
            "UPDATE site_settings SET site_name = $1, site_description = $2, site_url = $3, \
             logo_url = $4, icp_text = $5, default_cover_url = $6, page_size_index = $7, \
             page_size_archive = $8, page_size_search = $9, comment_policy = $10, \
             comments_per_page = $11, updated_at = now() \
             WHERE id = 1 RETURNING {SETTINGS_COLUMNS}"
        );
        let row = logged_query_as::<SiteSettingsRow>(&query)
            .bind(update.site_name)
            .bind(update.site_description)
            .bind(update.site_url)
            .bind(update.logo_url)
            .bind(update.icp_text)
            .bind(update.default_cover_url)
            .bind(update.page_size_index)
            .bind(update.page_size_archive)
            .bind(update.page_size_search)
            .bind(update.comment_policy.as_str())
            .bind(update.comments_per_page)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.try_into()
    }
}

fn map_sqlx(error: sqlx::Error) -> MediaError {
    tracing::error!(error = %error, "site settings repository operation failed");
    MediaError::StoreUnavailable
}
