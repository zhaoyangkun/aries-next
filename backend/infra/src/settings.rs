//! 设置分组 Repository 的 PostgreSQL 实现。
//! 更新走乐观锁（WHERE version = 期望值），payload 为原始 jsonb，
//! 结构化校验与 secret 掩码由上层负责。

use crate::logged::logged_query_as;
use aries_core::settings::{
    SettingError, SettingGroup, SettingGroupRecord, SettingGroupUpdate, SettingRepository,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresSettingRepository {
    pool: PgPool,
}

impl PostgresSettingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct SettingGroupRow {
    grp: String,
    payload: serde_json::Value,
    version: i32,
    updated_by: Option<i64>,
    updated_at: OffsetDateTime,
}

impl TryFrom<SettingGroupRow> for SettingGroupRecord {
    type Error = SettingError;

    fn try_from(row: SettingGroupRow) -> Result<Self, Self::Error> {
        Ok(Self {
            group: row.grp.parse().map_err(|_| SettingError::InvalidGroup)?,
            payload: row.payload,
            version: row.version,
            updated_by: row.updated_by,
            updated_at: row.updated_at,
        })
    }
}

const SETTING_COLUMNS: &str = "grp, payload, version, updated_by, updated_at";

fn map_sqlx(error: sqlx::Error) -> SettingError {
    tracing::error!(error = %error, "setting repository operation failed");
    SettingError::StoreUnavailable
}

#[async_trait]
impl SettingRepository for PostgresSettingRepository {
    async fn get_group(&self, group: SettingGroup) -> Result<SettingGroupRecord, SettingError> {
        let query = format!("SELECT {SETTING_COLUMNS} FROM setting_groups WHERE grp = $1");
        let row = logged_query_as::<SettingGroupRow>(&query)
            .bind(group.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(SettingError::NotFound)?;
        row.try_into()
    }

    async fn update_group(
        &self,
        group: SettingGroup,
        update: SettingGroupUpdate,
    ) -> Result<SettingGroupRecord, SettingError> {
        // 乐观锁：WHERE 同时匹配 grp 与期望版本，失配即视为冲突（默认行已预插，正常不会 miss）。
        let query = format!(
            "UPDATE setting_groups SET payload = $2, version = version + 1, \
             updated_by = $3, updated_at = now() \
             WHERE grp = $1 AND version = $4 \
             RETURNING {SETTING_COLUMNS}"
        );
        let row = logged_query_as::<SettingGroupRow>(&query)
            .bind(group.as_str())
            .bind(&update.payload)
            .bind(update.updated_by)
            .bind(update.expected_version)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(SettingError::Conflict)?;
        row.try_into()
    }
}
