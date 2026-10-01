//! `users` → `users`：字段直搬；bcrypt `pwd` 原样入 `password_hash`（登录链路兼容验证，
//! 首次登录成功后由 server 升级为 Argon2id）；最小 id 账号为 owner（`--owner-ids` 覆盖）。

use anyhow::Context;
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use super::{
    CommonParams, TransformOutcome, apply_outcome, archive_unmapped_field, optional_time,
    required_time,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "users";

#[derive(Debug, Clone, FromRow)]
pub struct LegacyUser {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub pwd: String,
    pub nickname: String,
    pub user_img: String,
    pub signature: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewUser {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub role: String,
    pub status: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
    pub password_updated_at: OffsetDateTime,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyUser>> {
    sqlx::query_as::<_, LegacyUser>(
        "SELECT CAST(id AS SIGNED) AS id, username, email, pwd, nickname, user_img, signature, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM users ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy users")
}

/// `owner_ids` 为空时取最小 id 账号为 owner，其余 editor。
pub fn transform(
    row: &LegacyUser,
    params: &CommonParams,
    owner_ids: &[i64],
) -> anyhow::Result<TransformOutcome<NewUser>> {
    let mut archives = Vec::new();
    archive_unmapped_field(
        &mut archives,
        row.id,
        "signature",
        &row.signature,
        "target users table has no signature column",
    );

    let display_name = if row.nickname.trim().is_empty() {
        row.username.clone()
    } else {
        row.nickname.clone()
    };
    // owner_ids 由 migrate 解析（显式 --owner-ids 或最小 id）后传入。
    let is_owner = owner_ids.contains(&row.id);

    Ok(TransformOutcome {
        row: Some(NewUser {
            id: row.id,
            username: row.username.clone(),
            email: row.email.clone(),
            // bcrypt Hash 原样搬运；PasswordHasher::verify 按前缀分发兼容验证。
            password_hash: row.pwd.clone(),
            display_name,
            avatar_url: non_empty(row.user_img.clone()),
            role: if is_owner { "owner" } else { "editor" }.to_owned(),
            status: "active".to_owned(),
            created_at: required_time(
                row.created_at.as_deref(),
                params.offset,
                TABLE,
                row.id,
                "created_at",
            )?,
            updated_at: required_time(
                row.updated_at.as_deref(),
                params.offset,
                TABLE,
                row.id,
                "updated_at",
            )?,
            deleted_at: optional_time(
                row.deleted_at.as_deref(),
                params.offset,
                TABLE,
                row.id,
                "deleted_at",
            )?,
            password_updated_at: required_time(
                row.created_at.as_deref(),
                params.offset,
                TABLE,
                row.id,
                "created_at",
            )?,
        }),
        archives,
        repaired: 0,
        notes: Vec::new(),
    })
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

pub async fn migrate(ctx: &mut MigrateCtx) -> anyhow::Result<()> {
    let rows = extract(&ctx.mysql).await?;
    let params = CommonParams {
        offset: ctx.offset,
        truncate_violations: ctx.truncate_violations,
    };
    ctx.report.table(TABLE).source_rows = rows.len() as i64;

    // owner 解析：显式 --owner-ids 优先，否则最小 id。
    let owner_ids: Vec<i64> = if ctx.owner_ids.is_empty() {
        rows.first().map(|row| vec![row.id]).unwrap_or_default()
    } else {
        ctx.owner_ids.clone()
    };
    if let Some(&owner_id) = owner_ids.first() {
        ctx.set_owner_id(owner_id);
    }

    let mut new_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let outcome = transform(row, &params, &owner_ids)?;
        if let Some(new_row) = apply_outcome(ctx, TABLE, outcome)? {
            new_rows.push(new_row);
        }
    }

    let mut migrated = 0i64;
    let mut skipped = 0i64;
    for chunk in new_rows.chunks(ctx.batch_size.max(1)) {
        let mut tx = ctx.pg.begin().await?;
        for row in chunk {
            let affected = sqlx::query(
                "INSERT INTO users (id, username, email, password_hash, display_name, avatar_url, \
                 role, status, created_at, updated_at, deleted_at, password_updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(&row.username)
            .bind(&row.email)
            .bind(&row.password_hash)
            .bind(&row.display_name)
            .bind(&row.avatar_url)
            .bind(&row.role)
            .bind(&row.status)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .bind(row.password_updated_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert user #{}", row.id))?
            .rows_affected();
            if affected == 1 {
                migrated += 1;
            } else {
                skipped += 1;
            }
        }
        tx.commit().await?;
    }

    let report = ctx.report.table(TABLE);
    report.migrated = migrated;
    report.skipped = skipped;
    if !ctx.owner_ids.is_empty() {
        report.note(format!("owner accounts from --owner-ids: {owner_ids:?}"));
    } else if let Some(&owner_id) = owner_ids.first() {
        report.note(format!(
            "smallest id account #{owner_id} assigned role owner"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::UtcOffset;

    fn params() -> CommonParams {
        CommonParams {
            offset: UtcOffset::UTC,
            truncate_violations: false,
        }
    }

    fn legacy(id: i64) -> LegacyUser {
        LegacyUser {
            id,
            username: "alice".to_owned(),
            email: "alice@example.com".to_owned(),
            pwd: "$2y$10$abcdefghijklmnopqrstuv".to_owned(),
            nickname: String::new(),
            user_img: String::new(),
            signature: "个性签名".to_owned(),
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    #[test]
    fn bcrypt_hash_copied_verbatim_and_signature_archived() {
        let outcome = transform(&legacy(1), &params(), &[1]).unwrap();
        let user = outcome.row.unwrap();
        assert_eq!(user.password_hash, "$2y$10$abcdefghijklmnopqrstuv");
        assert_eq!(user.role, "owner");
        assert_eq!(user.status, "active");
        assert_eq!(outcome.archives.len(), 1);
        assert!(outcome.archives[0].reason.contains("signature"));
    }

    #[test]
    fn empty_nickname_falls_back_to_username_and_empty_avatar_becomes_null() {
        let outcome = transform(&legacy(2), &params(), &[1]).unwrap();
        let user = outcome.row.unwrap();
        assert_eq!(user.display_name, "alice");
        assert_eq!(user.avatar_url, None);
        assert_eq!(user.role, "editor");
    }

    #[test]
    fn non_empty_nickname_and_avatar_kept() {
        let mut row = legacy(2);
        row.nickname = "爱丽丝".to_owned();
        row.user_img = "https://a.com/a.png".to_owned();
        let user = transform(&row, &params(), &[1]).unwrap().row.unwrap();
        assert_eq!(user.display_name, "爱丽丝");
        assert_eq!(user.avatar_url.as_deref(), Some("https://a.com/a.png"));
    }

    #[test]
    fn zero_date_is_hard_error() {
        let mut row = legacy(1);
        row.created_at = Some("0000-00-00 00:00:00".to_owned());
        assert!(transform(&row, &params(), &[1]).is_err());
    }
}
