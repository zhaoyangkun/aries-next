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
            .map_err(map_sqlx)?;
        let Some(row) = row else {
            // 默认行由 Migration 预插，但早期开发库 / 手工清理过的库可能缺失；
            // 自愈补默认行（payload 取列默认值），避免 GET 永久 404。
            sqlx::query(
                "INSERT INTO setting_groups (grp) VALUES ($1) ON CONFLICT (grp) DO NOTHING",
            )
            .bind(group.as_str())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
            return logged_query_as::<SettingGroupRow>(&query)
                .bind(group.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(map_sqlx)?
                .ok_or(SettingError::NotFound)?
                .try_into();
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use uuid::Uuid;

    /// 环境变量门控 + dotenvy 读取 .env + 独立 search_path 运行 Migration。
    async fn create_test_schema() -> anyhow::Result<(PgPool, PgPool, String)> {
        let _ = dotenvy::dotenv();
        let base_config = crate::PostgresConfig::from_env()?;
        let admin_pool = crate::connect_postgres(&base_config).await?;
        let test_schema = format!("aries_test_settings_{}", Uuid::now_v7().simple());

        // 随机 Schema 隔离业务数据，测试无论成功或失败都会执行清理。
        crate::logged::logged_query(&format!("CREATE SCHEMA \"{test_schema}\""))
            .execute(&admin_pool)
            .await
            .context("failed to create isolated settings test schema")?;
        let test_config =
            base_config.with_search_path(format!("{test_schema},{}", base_config.schema()));
        let test_pool = crate::connect_postgres(&test_config).await?;
        crate::run_migrations(&test_pool).await?;
        Ok((admin_pool, test_pool, test_schema))
    }

    async fn drop_test_schema(admin_pool: &PgPool, test_schema: &str) -> anyhow::Result<()> {
        crate::logged::logged_query(&format!("DROP SCHEMA \"{test_schema}\" CASCADE"))
            .execute(admin_pool)
            .await
            .context("failed to remove isolated settings test schema")?;
        Ok(())
    }

    #[tokio::test]
    async fn postgresql_setting_repository_get_group_self_heals_missing_default_row()
    -> anyhow::Result<()> {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let (admin_pool, test_pool, test_schema) = create_test_schema().await?;
        let scenario_result = async {
            let repository = PostgresSettingRepository::new(test_pool.clone());

            // 模拟早期开发库 / 被手工清理的库：预插默认行缺失。
            sqlx::query("DELETE FROM setting_groups WHERE grp = 'appearance'")
                .execute(&test_pool)
                .await
                .context("failed to remove seeded appearance row")?;

            let record = repository
                .get_group(SettingGroup::Appearance)
                .await
                .context("get_group should self-heal the missing default row")?;
            anyhow::ensure!(record.group == SettingGroup::Appearance);
            anyhow::ensure!(record.version == 1);
            anyhow::ensure!(record.payload == serde_json::json!({}));

            // 自愈写入的行落库，再次读取不再走补写路径。
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM setting_groups WHERE grp = 'appearance'")
                    .fetch_one(&test_pool)
                    .await
                    .context("failed to count appearance rows")?;
            anyhow::ensure!(count == 1);
            Ok(())
        }
        .await;

        test_pool.close().await;
        let cleanup_result = drop_test_schema(&admin_pool, &test_schema).await;
        admin_pool.close().await;
        scenario_result?;
        cleanup_result
    }
}
