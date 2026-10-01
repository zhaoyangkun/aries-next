use std::str::FromStr;

use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use aries_core::auth::{
    AuditEvent, AuditListQuery, AuditLogEntry, AuditPage, AuthError, AuthRepository,
    AuthenticatedSession, CredentialUser, NewOwner, NewPasswordReset, NewSession, ProfileUpdate,
    Role, User, UserStatus,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub struct PostgresAuthRepository {
    pool: PgPool,
}

impl PostgresAuthRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: i64,
    username: String,
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    role: String,
    status: String,
}

impl TryFrom<UserRow> for User {
    type Error = AuthError;

    fn try_from(row: UserRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            username: row.username,
            email: row.email,
            display_name: row.display_name,
            avatar_url: row.avatar_url,
            role: Role::from_str(&row.role)?,
            status: UserStatus::from_str(&row.status)?,
        })
    }
}

#[derive(Debug, FromRow)]
struct CredentialRow {
    id: i64,
    username: String,
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    role: String,
    status: String,
    password_hash: String,
}

impl TryFrom<CredentialRow> for CredentialUser {
    type Error = AuthError;

    fn try_from(row: CredentialRow) -> Result<Self, Self::Error> {
        Ok(Self {
            user: User {
                id: row.id,
                username: row.username,
                email: row.email,
                display_name: row.display_name,
                avatar_url: row.avatar_url,
                role: Role::from_str(&row.role)?,
                status: UserStatus::from_str(&row.status)?,
            },
            password_hash: row.password_hash,
        })
    }
}

#[derive(Debug, FromRow)]
struct SessionRow {
    session_id: Uuid,
    expires_at: OffsetDateTime,
    id: i64,
    username: String,
    email: String,
    display_name: String,
    avatar_url: Option<String>,
    role: String,
    status: String,
}

impl TryFrom<SessionRow> for AuthenticatedSession {
    type Error = AuthError;

    fn try_from(row: SessionRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.session_id,
            expires_at: row.expires_at,
            user: User {
                id: row.id,
                username: row.username,
                email: row.email,
                display_name: row.display_name,
                avatar_url: row.avatar_url,
                role: Role::from_str(&row.role)?,
                status: UserStatus::from_str(&row.status)?,
            },
        })
    }
}

const USER_COLUMNS: &str =
    "id, username::text AS username, email::text AS email, display_name, avatar_url, role, status";

#[async_trait]
impl AuthRepository for PostgresAuthRepository {
    async fn is_bootstrapped(&self) -> Result<bool, AuthError> {
        logged_query_scalar::<bool>("SELECT EXISTS(SELECT 1 FROM users)")
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)
    }

    async fn create_owner(&self, owner: NewOwner) -> Result<User, AuthError> {
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;

        // Advisory Lock 保证两个并发 Bootstrap 请求中最多只有一个可以创建 Owner。
        logged_query("SELECT pg_advisory_xact_lock($1)")
            .bind(0x4152_4945_5342_4f4f_i64)
            .execute(&mut *transaction)
            .await
            .map_err(map_sqlx)?;

        let has_user = logged_query_scalar::<bool>("SELECT EXISTS(SELECT 1 FROM users)")
            .fetch_one(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        if has_user {
            return Err(AuthError::Conflict);
        }

        let query = format!(
            "INSERT INTO users (username, email, password_hash, display_name, role, status) \
             VALUES ($1, $2, $3, $4, 'owner', 'active') RETURNING {USER_COLUMNS}"
        );
        let row = logged_query_as::<UserRow>(&query)
            .bind(owner.username)
            .bind(owner.email)
            .bind(owner.password_hash)
            .bind(owner.display_name)
            .fetch_one(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        transaction.commit().await.map_err(map_sqlx)?;
        row.try_into()
    }

    async fn find_credentials(&self, login: &str) -> Result<Option<CredentialUser>, AuthError> {
        let row = logged_query_as::<CredentialRow>(
            "SELECT id, username::text AS username, email::text AS email, display_name, avatar_url, \
             role, status, password_hash FROM users \
             WHERE deleted_at IS NULL AND (username = $1 OR email = $1)",
        )
        .bind(login)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, AuthError> {
        let query =
            format!("SELECT {USER_COLUMNS} FROM users WHERE deleted_at IS NULL AND email = $1");
        let row = logged_query_as::<UserRow>(&query)
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn create_session(&self, session: NewSession) -> Result<(), AuthError> {
        logged_query(
            "INSERT INTO admin_sessions (id, token_hash, user_id, expires_at, user_agent) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(session.id)
        .bind(session.token_hash)
        .bind(session.user_id)
        .bind(session.expires_at)
        .bind(session.user_agent)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn find_session(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<AuthenticatedSession>, AuthError> {
        let row = logged_query_as::<SessionRow>(
            "SELECT s.id AS session_id, s.expires_at, u.id, u.username::text AS username, \
             u.email::text AS email, u.display_name, u.avatar_url, u.role, u.status \
             FROM admin_sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now() \
             AND u.deleted_at IS NULL AND u.status = 'active'",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn touch_session(&self, session_id: Uuid) -> Result<(), AuthError> {
        logged_query(
            "UPDATE admin_sessions SET last_seen_at = now() \
             WHERE id = $1 AND last_seen_at < now() - interval '5 minutes'",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn revoke_session(&self, session_id: Uuid) -> Result<(), AuthError> {
        logged_query(
            "UPDATE admin_sessions SET revoked_at = COALESCE(revoked_at, now()) WHERE id = $1",
        )
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn revoke_user_sessions(&self, user_id: i64) -> Result<(), AuthError> {
        logged_query(
            "UPDATE admin_sessions SET revoked_at = COALESCE(revoked_at, now()) \
             WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn delete_expired_sessions(&self) -> Result<u64, AuthError> {
        // 过期与已撤销的 Session 不再参与任何查询（find_session 均排除），可安全物理删除；
        // 撤销动作本身由 audit log 留痕，不依赖本表。
        let result = logged_query(
            "DELETE FROM admin_sessions WHERE expires_at <= now() OR revoked_at IS NOT NULL",
        )
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(result.rows_affected())
    }

    async fn update_last_login(&self, user_id: i64) -> Result<(), AuthError> {
        logged_query("UPDATE users SET last_login_at = now(), updated_at = now() WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn update_profile(
        &self,
        user_id: i64,
        profile: ProfileUpdate,
    ) -> Result<User, AuthError> {
        let query = format!(
            "UPDATE users SET email = $2, display_name = $3, avatar_url = $4, updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL RETURNING {USER_COLUMNS}"
        );
        let row = logged_query_as::<UserRow>(&query)
            .bind(user_id)
            .bind(profile.email)
            .bind(profile.display_name)
            .bind(profile.avatar_url)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?
            .ok_or(AuthError::NotFound)?;
        row.try_into()
    }

    async fn update_password(&self, user_id: i64, password_hash: &str) -> Result<(), AuthError> {
        let result = logged_query(
            "UPDATE users SET password_hash = $2, password_updated_at = now(), updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(password_hash)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(AuthError::NotFound);
        }
        Ok(())
    }

    async fn create_password_reset(&self, reset: NewPasswordReset) -> Result<(), AuthError> {
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        logged_query(
            "UPDATE password_reset_tokens SET consumed_at = now() \
             WHERE user_id = $1 AND consumed_at IS NULL",
        )
        .bind(reset.user_id)
        .execute(&mut *transaction)
        .await
        .map_err(map_sqlx)?;
        logged_query(
            "INSERT INTO password_reset_tokens (id, user_id, token_hash, expires_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(reset.id)
        .bind(reset.user_id)
        .bind(reset.token_hash)
        .bind(reset.expires_at)
        .execute(&mut *transaction)
        .await
        .map_err(map_sqlx)?;
        transaction.commit().await.map_err(map_sqlx)?;
        Ok(())
    }

    async fn consume_password_reset(
        &self,
        token_hash: &[u8],
        password_hash: &str,
    ) -> Result<Option<i64>, AuthError> {
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        let user_id = logged_query_scalar::<i64>(
            "UPDATE password_reset_tokens SET consumed_at = now() \
             WHERE token_hash = $1 AND consumed_at IS NULL AND expires_at > now() \
             RETURNING user_id",
        )
        .bind(token_hash)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(map_sqlx)?;

        if let Some(user_id) = user_id {
            update_password_and_revoke(&mut transaction, user_id, password_hash).await?;
            transaction.commit().await.map_err(map_sqlx)?;
            Ok(Some(user_id))
        } else {
            transaction.rollback().await.map_err(map_sqlx)?;
            Ok(None)
        }
    }

    async fn write_audit(&self, event: AuditEvent) -> Result<(), AuthError> {
        logged_query(
            "INSERT INTO audit_logs (actor_user_id, action, target_type, target_id, metadata) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(event.actor_user_id)
        .bind(event.action)
        .bind(event.target_type)
        .bind(event.target_id)
        .bind(event.metadata)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn list_audit(&self, query: AuditListQuery) -> Result<AuditPage, AuthError> {
        let offset =
            i64::from(query.page.saturating_sub(1)).saturating_mul(i64::from(query.page_size));
        let limit = i64::from(query.page_size);

        // 动态 WHERE：条件存在才追加绑定，与 comments 列表保持同一模式。
        let mut conditions: Vec<String> = Vec::new();
        let mut bind_index = 1u32;
        if query.actor_user_id.is_some() {
            conditions.push(format!("a.actor_user_id = ${bind_index}"));
            bind_index += 1;
        }
        if query.action.is_some() {
            conditions.push(format!("a.action = ${bind_index}"));
            bind_index += 1;
        }
        if query.target_type.is_some() {
            conditions.push(format!("a.target_type = ${bind_index}"));
            bind_index += 1;
        }
        if query.target_id.is_some() {
            conditions.push(format!("a.target_id = ${bind_index}"));
            bind_index += 1;
        }
        if query.start.is_some() {
            conditions.push(format!("a.created_at >= ${bind_index}"));
            bind_index += 1;
        }
        if query.end.is_some() {
            conditions.push(format!("a.created_at < ${bind_index}"));
            bind_index += 1;
        }
        let where_clause = if conditions.is_empty() {
            "TRUE".to_string()
        } else {
            conditions.join(" AND ")
        };

        let count_sql = format!("SELECT COUNT(*) FROM audit_logs a WHERE {where_clause}");
        let mut count_query = logged_query_scalar::<i64>(&count_sql);
        if let Some(value) = query.actor_user_id {
            count_query = count_query.bind(value);
        }
        if let Some(ref value) = query.action {
            count_query = count_query.bind(value);
        }
        if let Some(ref value) = query.target_type {
            count_query = count_query.bind(value);
        }
        if let Some(ref value) = query.target_id {
            count_query = count_query.bind(value);
        }
        if let Some(value) = query.start {
            count_query = count_query.bind(value);
        }
        if let Some(value) = query.end {
            count_query = count_query.bind(value);
        }
        let total = count_query.fetch_one(&self.pool).await.map_err(map_sqlx)?;

        let data_sql = format!(
            "SELECT a.id, a.actor_user_id, u.username AS actor_username, a.action, \
             a.target_type, a.target_id, a.metadata, a.created_at \
             FROM audit_logs a LEFT JOIN users u ON u.id = a.actor_user_id \
             WHERE {where_clause} ORDER BY a.created_at DESC, a.id DESC \
             LIMIT ${bind_index} OFFSET ${}",
            bind_index + 1
        );
        let mut data_query = logged_query_as::<AuditLogRow>(&data_sql);
        if let Some(value) = query.actor_user_id {
            data_query = data_query.bind(value);
        }
        if let Some(ref value) = query.action {
            data_query = data_query.bind(value);
        }
        if let Some(ref value) = query.target_type {
            data_query = data_query.bind(value);
        }
        if let Some(ref value) = query.target_id {
            data_query = data_query.bind(value);
        }
        if let Some(value) = query.start {
            data_query = data_query.bind(value);
        }
        if let Some(value) = query.end {
            data_query = data_query.bind(value);
        }
        data_query = data_query.bind(limit).bind(offset);

        let rows = data_query.fetch_all(&self.pool).await.map_err(map_sqlx)?;
        let items = rows.into_iter().map(Into::into).collect();
        Ok(AuditPage {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }
}

/// Audit Log 查询行：`actor_username` 通过 LEFT JOIN 取，用户被删除时为 NULL。
#[derive(Debug, FromRow)]
struct AuditLogRow {
    id: i64,
    actor_user_id: Option<i64>,
    actor_username: Option<String>,
    action: String,
    target_type: String,
    target_id: Option<String>,
    metadata: serde_json::Value,
    created_at: OffsetDateTime,
}

impl From<AuditLogRow> for AuditLogEntry {
    fn from(row: AuditLogRow) -> Self {
        Self {
            id: row.id,
            actor_user_id: row.actor_user_id,
            actor_username: row.actor_username,
            action: row.action,
            target_type: row.target_type,
            target_id: row.target_id,
            metadata: row.metadata,
            created_at: row.created_at,
        }
    }
}

async fn update_password_and_revoke(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i64,
    password_hash: &str,
) -> Result<(), AuthError> {
    logged_query(
        "UPDATE users SET password_hash = $2, password_updated_at = now(), updated_at = now() \
         WHERE id = $1",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(&mut **transaction)
    .await
    .map_err(map_sqlx)?;
    logged_query(
        "UPDATE admin_sessions SET revoked_at = COALESCE(revoked_at, now()) \
         WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut **transaction)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

fn map_sqlx(error: sqlx::Error) -> AuthError {
    if error
        .as_database_error()
        .is_some_and(|database_error| database_error.is_unique_violation())
    {
        AuthError::Conflict
    } else {
        tracing::error!(error = %error, "authentication repository operation failed");
        AuthError::StoreUnavailable
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, ensure};
    use time::Duration;

    use super::*;

    #[tokio::test]
    async fn postgresql_repository_covers_session_and_password_reset_lifecycle()
    -> anyhow::Result<()> {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let _ = dotenvy::dotenv();
        let base_config = crate::PostgresConfig::from_env()?;
        let admin_pool = crate::connect_postgres(&base_config).await?;
        let test_schema = format!("aries_test_{}", Uuid::now_v7().simple());

        // Schema 名称仅由 UUID 生成，显式引用可保证测试不会操作业务 Schema。
        logged_query(&format!("CREATE SCHEMA \"{test_schema}\""))
            .execute(&admin_pool)
            .await
            .context("failed to create isolated authentication test schema")?;

        let test_config = crate::PostgresConfig {
            options: base_config.options.clone(),
            // Citext Extension 可能已安装在业务 Schema，因此将它作为只读类型解析回退。
            schema: format!("{test_schema},{}", base_config.schema()),
            slow_query_ms: 0,
        };
        let test_pool = crate::connect_postgres(&test_config).await?;
        let scenario_result = async {
            crate::run_migrations(&test_pool).await?;
            run_repository_scenario(&test_pool).await
        }
        .await;

        test_pool.close().await;
        let cleanup_result = logged_query(&format!("DROP SCHEMA \"{test_schema}\" CASCADE"))
            .execute(&admin_pool)
            .await
            .context("failed to remove isolated authentication test schema");
        admin_pool.close().await;

        scenario_result?;
        cleanup_result?;
        Ok(())
    }

    async fn run_repository_scenario(pool: &PgPool) -> anyhow::Result<()> {
        let repository = PostgresAuthRepository::new(pool.clone());
        let owner = repository
            .create_owner(NewOwner {
                username: "owner".to_owned(),
                email: "owner@example.com".to_owned(),
                display_name: "Aries Owner".to_owned(),
                password_hash: "hash:initial".to_owned(),
            })
            .await?;
        ensure!(owner.role == Role::Owner);
        ensure!(matches!(
            repository
                .create_owner(NewOwner {
                    username: "second".to_owned(),
                    email: "second@example.com".to_owned(),
                    display_name: "Second Owner".to_owned(),
                    password_hash: "hash:second".to_owned(),
                })
                .await,
            Err(AuthError::Conflict)
        ));

        let first_session = Uuid::now_v7();
        let first_token = vec![1_u8; 32];
        repository
            .create_session(NewSession {
                id: first_session,
                token_hash: first_token.clone(),
                user_id: owner.id,
                expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
                user_agent: Some("integration-test".to_owned()),
            })
            .await?;
        ensure!(repository.find_session(&first_token).await?.is_some());
        repository.revoke_session(first_session).await?;
        let expired_session = Uuid::now_v7();
        repository
            .create_session(NewSession {
                id: expired_session,
                token_hash: vec![9_u8; 32],
                user_id: owner.id,
                expires_at: OffsetDateTime::now_utc() - Duration::minutes(1),
                user_agent: None,
            })
            .await?;
        // 撤销的 Session 与已过期的 Session 都被物理清理，且重复执行是幂等的。
        ensure!(repository.delete_expired_sessions().await? >= 2);
        ensure!(repository.delete_expired_sessions().await? == 0);
        ensure!(repository.find_session(&first_token).await?.is_none());

        let rotated_token = vec![2_u8; 32];
        repository
            .create_session(NewSession {
                id: Uuid::now_v7(),
                token_hash: rotated_token.clone(),
                user_id: owner.id,
                expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
                user_agent: None,
            })
            .await?;
        let reset_token = vec![3_u8; 32];
        repository
            .create_password_reset(NewPasswordReset {
                id: Uuid::now_v7(),
                user_id: owner.id,
                token_hash: reset_token.clone(),
                expires_at: OffsetDateTime::now_utc() + Duration::minutes(30),
            })
            .await?;
        ensure!(
            repository
                .consume_password_reset(&reset_token, "hash:updated")
                .await?
                == Some(owner.id)
        );
        ensure!(
            repository
                .consume_password_reset(&reset_token, "hash:reused")
                .await?
                .is_none()
        );
        ensure!(repository.find_session(&rotated_token).await?.is_none());
        ensure!(
            repository
                .find_credentials("owner")
                .await?
                .is_some_and(|credentials| credentials.password_hash == "hash:updated")
        );

        Ok(())
    }
}
