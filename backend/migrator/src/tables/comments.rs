//! `comments` → `comments`：多态目标映射（type 1→article、3/4→page、2→link + target_id=0）、
//! 层级重建（parent 链上溯求根，根评论 root_id 自指在 Load 后二次 UPDATE 回填）、
//! `device` SHA-256 后入 `user_agent_digest`（原始 UA 不落盘、不进 Archive）。

use std::collections::{HashMap, HashSet};

use anyhow::{Context, bail};
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use aries_core::content::MarkdownRenderer;
use aries_infra::{ComrakMarkdownRenderer, upload::sha256_hex};

use super::{
    ArchiveEntry, CommonParams, TransformOutcome, apply_outcome, enforce_length, enforce_nonempty,
    optional_time, required_time, tinyint_to_bool,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "comments";

const MAX_CONTENT_CHARS: usize = 2000;
const MAX_NAME_CHARS: usize = 60;
const MAX_EMAIL_CHARS: usize = 254;
const MAX_URL_CHARS: usize = 2048;

#[derive(Debug, Clone, FromRow)]
pub struct LegacyComment {
    pub id: i64,
    pub admin_user_id: i64,
    pub article_id: i64,
    pub page_id: i64,
    pub root_comment_id: i64,
    pub parent_comment_id: i64,
    pub r#type: i64,
    pub email: String,
    pub url: String,
    pub user_img: String,
    pub nick_name: String,
    pub content: String,
    pub md_content: String,
    pub device: String,
    pub is_recycled: i64,
    pub is_checked: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewComment {
    pub id: i64,
    pub target_type: String,
    pub target_id: i64,
    /// 顶层评论为 None，Load 后回填为自身 id。
    pub root_id: Option<i64>,
    pub parent_id: Option<i64>,
    pub author_name: String,
    pub author_email: String,
    pub author_url: Option<String>,
    pub content_markdown: String,
    pub content_html: String,
    pub status: String,
    pub moderator_id: Option<i64>,
    pub user_agent_digest: String,
    pub is_admin_reply: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyComment>> {
    sqlx::query_as::<_, LegacyComment>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(admin_user_id AS SIGNED) AS admin_user_id, \
         CAST(article_id AS SIGNED) AS article_id, CAST(page_id AS SIGNED) AS page_id, \
         CAST(root_comment_id AS SIGNED) AS root_comment_id, \
         CAST(parent_comment_id AS SIGNED) AS parent_comment_id, \
         CAST(`type` AS SIGNED) AS `type`, email, url, user_img, nick_name, content, md_content, \
         device, CAST(is_recycled AS SIGNED) AS is_recycled, CAST(is_checked AS SIGNED) AS is_checked, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM comments ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy comments")
}

/// type → (target_type, target_id)。type=2（友链页）无对应 Link ID，映射 ('link', 0)，
/// 数据完整入库；新站前台暂不展示友链页评论（见 docs/database-mapping.md）。
pub fn map_target(row: &LegacyComment) -> anyhow::Result<(&'static str, i64)> {
    match row.r#type {
        1 => Ok(("article", row.article_id)),
        2 => Ok(("link", 0)),
        3 | 4 => Ok(("page", row.page_id)),
        other => bail!(
            "comments #{}: type {other} out of range (expected 1-4)",
            row.id
        ),
    }
}

/// `is_checked` / `is_recycled` → status；recycled 优先。
pub fn map_status(is_checked: bool, is_recycled: bool) -> &'static str {
    if is_recycled {
        "recycled"
    } else if is_checked {
        "approved"
    } else {
        "pending"
    }
}

/// 沿 parent 链上溯求根。返回 (parent_id, root_id)：
/// - 顶层（parent=0 / 悬挂 / 成环）→ (None, None)，root_id 由 Load 后回填为自身 id。
/// - 正常回复 → (Some(parent), Some(root))。
pub fn resolve_hierarchy(
    row: &LegacyComment,
    parents: &HashMap<i64, i64>,
) -> (Option<i64>, Option<i64>, bool) {
    if row.parent_comment_id == 0 || row.parent_comment_id == row.id {
        return (None, None, false);
    }
    let mut current = row.parent_comment_id;
    let mut visited = HashSet::from([row.id]);
    loop {
        if !visited.insert(current) {
            // 成环：提升为顶层。
            return (None, None, true);
        }
        match parents.get(&current) {
            None => return (None, None, true), // 悬挂 parent：提升为顶层
            Some(&next) if next == 0 || next == current => {
                // current 是顶层评论。
                return (Some(row.parent_comment_id), Some(current), false);
            }
            Some(&next) => current = next,
        }
    }
}

pub struct CommentRefs<'a> {
    pub article_ids: &'a HashSet<i64>,
    pub page_ids: &'a HashSet<i64>,
    pub user_ids: &'a HashSet<i64>,
    pub renderer: &'a ComrakMarkdownRenderer,
}

pub fn transform(
    row: &LegacyComment,
    params: &CommonParams,
    parents: &HashMap<i64, i64>,
    refs: &CommentRefs,
) -> anyhow::Result<TransformOutcome<NewComment>> {
    let (target_type, target_id) = map_target(row)?;

    // 目标文章/页面悬挂：评论无处挂载，整行进 Archive（不静默丢弃）。
    let target_dangling = match target_type {
        "article" => !refs.article_ids.contains(&target_id),
        "page" => !refs.page_ids.contains(&target_id),
        _ => false,
    };
    if target_dangling {
        return Ok(TransformOutcome::archive_row(
            Some(row.id),
            serde_json::json!({
                "type": row.r#type,
                "article_id": row.article_id,
                "page_id": row.page_id,
                "nick_name": row.nick_name,
                "content": row.content,
            }),
            format!("dangling {target_type} target {target_id}"),
        ));
    }

    let mut archives = Vec::new();
    let mut repaired = 0i64;

    // 层级：沿 parent 链上溯；悬挂/成环提升为顶层。
    let (parent_id, root_id, hierarchy_repaired) = resolve_hierarchy(row, parents);
    if hierarchy_repaired {
        repaired += 1;
        archives.push(ArchiveEntry {
            id: Some(row.id),
            fields: serde_json::json!({
                "parent_comment_id": row.parent_comment_id,
                "root_comment_id": row.root_comment_id,
            }),
            reason: "dangling or cyclic parent reference; promoted to top level".to_owned(),
            whole_row: false,
        });
    }

    // 长度/非空约束。
    let (value, archive_a, fix_a) = enforce_nonempty(
        params,
        TABLE,
        row.id,
        "nick_name",
        row.nick_name.clone(),
        || "匿名".to_owned(),
    )?;
    let (author_name, archive_b, fix_b) =
        enforce_length(params, TABLE, row.id, "nick_name", value, MAX_NAME_CHARS)?;
    for entry in [archive_a, archive_b].into_iter().flatten() {
        archives.push(entry);
    }
    repaired += fix_a + fix_b;

    let (author_email, archive, fix) = enforce_length(
        params,
        TABLE,
        row.id,
        "email",
        row.email.clone(),
        MAX_EMAIL_CHARS,
    )?;
    if let Some(entry) = archive {
        archives.push(entry);
    }
    repaired += fix;

    let author_url = if row.url.trim().is_empty() {
        None
    } else {
        let (value, archive, fix) =
            enforce_length(params, TABLE, row.id, "url", row.url.clone(), MAX_URL_CHARS)?;
        if let Some(entry) = archive {
            archives.push(entry);
        }
        repaired += fix;
        Some(value)
    };

    let (value, archive_a, fix_a) = enforce_nonempty(
        params,
        TABLE,
        row.id,
        "content",
        row.content.clone(),
        || " ".to_owned(),
    )?;
    let (content, archive_b, fix_b) =
        enforce_length(params, TABLE, row.id, "content", value, MAX_CONTENT_CHARS)?;
    for entry in [archive_a, archive_b].into_iter().flatten() {
        archives.push(entry);
    }
    repaired += fix_a + fix_b;

    super::archive_unmapped_field(
        &mut archives,
        row.id,
        "md_content",
        &row.md_content,
        "legacy rendered HTML replaced by fresh comrak render",
    );
    super::archive_unmapped_field(
        &mut archives,
        row.id,
        "user_img",
        &row.user_img,
        "target comments table has no avatar column",
    );

    let is_checked = tinyint_to_bool(row.is_checked, TABLE, row.id, "is_checked")?;
    let is_recycled = tinyint_to_bool(row.is_recycled, TABLE, row.id, "is_recycled")?;
    let mut status = map_status(is_checked, is_recycled).to_owned();

    let deleted_at = optional_time(
        row.deleted_at.as_deref(),
        params.offset,
        TABLE,
        row.id,
        "deleted_at",
    )?;
    // 目标语义：软删除的评论应处于回收站状态。
    if deleted_at.is_some() && status != "recycled" {
        status = "recycled".to_owned();
        repaired += 1;
    }

    let is_admin_reply = row.admin_user_id != 0;
    let moderator_id = if is_admin_reply && refs.user_ids.contains(&row.admin_user_id) {
        Some(row.admin_user_id)
    } else {
        None
    };

    Ok(TransformOutcome {
        row: Some(NewComment {
            id: row.id,
            target_type: target_type.to_owned(),
            target_id,
            root_id,
            parent_id,
            author_name,
            author_email,
            author_url,
            content_html: refs.renderer.render(&content)?,
            content_markdown: content,
            status,
            moderator_id,
            // 原始 UA 只取 SHA-256 摘要；空 UA 存空串（目标列 NOT NULL DEFAULT ''）。
            user_agent_digest: if row.device.is_empty() {
                String::new()
            } else {
                sha256_hex(row.device.as_bytes())
            },
            is_admin_reply,
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
            deleted_at,
        }),
        archives,
        repaired,
        notes: Vec::new(),
    })
}

pub async fn migrate(ctx: &mut MigrateCtx) -> anyhow::Result<()> {
    let rows = extract(&ctx.mysql).await?;
    let params = CommonParams {
        offset: ctx.offset,
        truncate_violations: ctx.truncate_violations,
    };
    ctx.report.table(TABLE).source_rows = rows.len() as i64;

    let parents: HashMap<i64, i64> = rows
        .iter()
        .map(|row| (row.id, row.parent_comment_id))
        .collect();
    let article_ids = super::fetch_id_set(&ctx.mysql, "articles").await?;
    let page_ids = super::fetch_id_set(&ctx.mysql, "pages").await?;
    let user_ids = super::fetch_id_set(&ctx.mysql, "users").await?;
    let renderer = ctx.renderer;
    let refs = CommentRefs {
        article_ids: &article_ids,
        page_ids: &page_ids,
        user_ids: &user_ids,
        renderer: &renderer,
    };

    let mut new_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let outcome = transform(row, &params, &parents, &refs)?;
        if let Some(new_row) = apply_outcome(ctx, TABLE, outcome)? {
            new_rows.push(new_row);
        }
    }

    let mut migrated = 0i64;
    let mut skipped = 0i64;
    let mut top_level_ids = Vec::new();
    for chunk in new_rows.chunks(ctx.batch_size.max(1)) {
        let mut tx = ctx.pg.begin().await?;
        for row in chunk {
            let affected = sqlx::query(
                "INSERT INTO comments (id, target_type, target_id, root_id, parent_id, \
                 author_name, author_email, author_url, content_markdown, content_html, status, \
                 moderator_id, user_agent_digest, is_admin_reply, created_at, updated_at, deleted_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17) \
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(&row.target_type)
            .bind(row.target_id)
            .bind(row.root_id)
            .bind(row.parent_id)
            .bind(&row.author_name)
            .bind(&row.author_email)
            .bind(&row.author_url)
            .bind(&row.content_markdown)
            .bind(&row.content_html)
            .bind(&row.status)
            .bind(row.moderator_id)
            .bind(&row.user_agent_digest)
            .bind(row.is_admin_reply)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert comment #{}", row.id))?
            .rows_affected();
            if affected == 1 {
                migrated += 1;
                if row.root_id.is_none() {
                    top_level_ids.push(row.id);
                }
            } else {
                skipped += 1;
                // 已存在的行仍需保证 root_id 已回填（上次运行可能在此中断）。
                if row.root_id.is_none() {
                    top_level_ids.push(row.id);
                }
            }
        }
        tx.commit().await?;
    }

    // 根评论 root_id 自指回填（幂等）。
    if !top_level_ids.is_empty() {
        sqlx::query("UPDATE comments SET root_id = id WHERE id = ANY($1) AND root_id IS NULL")
            .bind(&top_level_ids)
            .execute(&ctx.pg)
            .await
            .context("failed to backfill top-level comment root_id")?;
    }

    let report = ctx.report.table(TABLE);
    report.migrated = migrated;
    report.skipped = skipped;
    report.note(format!(
        "{} top-level comments root_id backfilled to self",
        top_level_ids.len()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::UtcOffset;

    fn params(truncate: bool) -> CommonParams {
        CommonParams {
            offset: UtcOffset::UTC,
            truncate_violations: truncate,
        }
    }

    fn legacy(id: i64) -> LegacyComment {
        LegacyComment {
            id,
            admin_user_id: 0,
            article_id: 1,
            page_id: 0,
            root_comment_id: 0,
            parent_comment_id: 0,
            r#type: 1,
            email: "guest@example.com".to_owned(),
            url: String::new(),
            user_img: "https://a.com/avatar.png".to_owned(),
            nick_name: "访客".to_owned(),
            content: "你好".to_owned(),
            md_content: "<p>你好</p>".to_owned(),
            device: "Mozilla/5.0".to_owned(),
            is_recycled: 0,
            is_checked: 1,
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    fn refs<'a>(
        articles: &'a HashSet<i64>,
        pages: &'a HashSet<i64>,
        users: &'a HashSet<i64>,
        renderer: &'a ComrakMarkdownRenderer,
    ) -> CommentRefs<'a> {
        CommentRefs {
            article_ids: articles,
            page_ids: pages,
            user_ids: users,
            renderer,
        }
    }

    #[test]
    fn target_mapping_covers_all_types() {
        let mut row = legacy(1);
        assert_eq!(map_target(&row).unwrap(), ("article", 1));
        row.r#type = 2;
        assert_eq!(map_target(&row).unwrap(), ("link", 0));
        row.r#type = 3;
        row.page_id = 7;
        assert_eq!(map_target(&row).unwrap(), ("page", 7));
        row.r#type = 4;
        assert_eq!(map_target(&row).unwrap(), ("page", 7));
        row.r#type = 9;
        assert!(map_target(&row).is_err());
    }

    #[test]
    fn status_mapping_and_recycled_precedence() {
        assert_eq!(map_status(true, false), "approved");
        assert_eq!(map_status(false, false), "pending");
        assert_eq!(map_status(true, true), "recycled");
        assert_eq!(map_status(false, true), "recycled");
    }

    #[test]
    fn hierarchy_resolution_top_reply_dangling_and_cycle() {
        // 顶层
        let parents: HashMap<i64, i64> = [(1, 0), (2, 1), (3, 2), (4, 99), (5, 6), (6, 5)]
            .into_iter()
            .collect();
        let top = legacy(1);
        assert_eq!(resolve_hierarchy(&top, &parents), (None, None, false));

        // 正常回复：3 的 parent 是 2，根是 1
        let reply = legacy(3);
        let mut reply = reply;
        reply.parent_comment_id = 2;
        assert_eq!(
            resolve_hierarchy(&reply, &parents),
            (Some(2), Some(1), false)
        );

        // 悬挂 parent 提升为顶层
        let mut orphan = legacy(4);
        orphan.parent_comment_id = 99;
        assert_eq!(resolve_hierarchy(&orphan, &parents), (None, None, true));

        // 成环提升为顶层
        let mut cyclic = legacy(5);
        cyclic.parent_comment_id = 6;
        assert_eq!(resolve_hierarchy(&cyclic, &parents), (None, None, true));
    }

    #[test]
    fn transform_guest_comment_hashes_ua_and_archives_legacy_fields() {
        let renderer = ComrakMarkdownRenderer;
        let articles: HashSet<i64> = [1].into_iter().collect();
        let pages: HashSet<i64> = HashSet::new();
        let users: HashSet<i64> = [1].into_iter().collect();
        let parents: HashMap<i64, i64> = [(1, 0)].into_iter().collect();

        let outcome = transform(
            &legacy(1),
            &params(false),
            &parents,
            &refs(&articles, &pages, &users, &renderer),
        )
        .unwrap();
        let row = outcome.row.unwrap();
        assert_eq!(row.target_type, "article");
        assert_eq!(row.status, "approved");
        assert!(!row.is_admin_reply);
        assert_eq!(row.user_agent_digest, sha256_hex(b"Mozilla/5.0"));
        assert!(row.content_html.contains("<p>你好</p>"));
        // md_content 与 user_img 进 Archive。
        assert!(
            outcome
                .archives
                .iter()
                .any(|a| a.fields.get("md_content").is_some())
        );
        assert!(
            outcome
                .archives
                .iter()
                .any(|a| a.fields.get("user_img").is_some())
        );
    }

    #[test]
    fn dangling_article_target_archives_whole_comment() {
        let renderer = ComrakMarkdownRenderer;
        let articles: HashSet<i64> = HashSet::new();
        let pages: HashSet<i64> = HashSet::new();
        let users: HashSet<i64> = HashSet::new();
        let parents: HashMap<i64, i64> = [(1, 0)].into_iter().collect();
        let outcome = transform(
            &legacy(1),
            &params(false),
            &parents,
            &refs(&articles, &pages, &users, &renderer),
        )
        .unwrap();
        assert!(outcome.row.is_none());
        assert_eq!(outcome.archives.len(), 1);
    }

    #[test]
    fn overlong_content_fails_by_default_and_truncates_with_flag() {
        let renderer = ComrakMarkdownRenderer;
        let articles: HashSet<i64> = [1].into_iter().collect();
        let pages: HashSet<i64> = HashSet::new();
        let users: HashSet<i64> = HashSet::new();
        let parents: HashMap<i64, i64> = [(1, 0)].into_iter().collect();

        let mut row = legacy(1);
        row.content = "长".repeat(2001);
        assert!(
            transform(
                &row,
                &params(false),
                &parents,
                &refs(&articles, &pages, &users, &renderer)
            )
            .is_err()
        );

        let outcome = transform(
            &row,
            &params(true),
            &parents,
            &refs(&articles, &pages, &users, &renderer),
        )
        .unwrap();
        let migrated = outcome.row.unwrap();
        assert_eq!(migrated.content_markdown.chars().count(), 2000);
        assert_eq!(outcome.repaired, 1);
        assert!(
            outcome
                .archives
                .iter()
                .any(|a| a.fields.get("content").is_some())
        );
    }

    #[test]
    fn admin_reply_sets_flag_and_moderator() {
        let renderer = ComrakMarkdownRenderer;
        let articles: HashSet<i64> = [1].into_iter().collect();
        let pages: HashSet<i64> = HashSet::new();
        let users: HashSet<i64> = [7].into_iter().collect();
        let parents: HashMap<i64, i64> = [(1, 0)].into_iter().collect();
        let mut row = legacy(1);
        row.admin_user_id = 7;
        row.email = String::new();
        let outcome = transform(
            &row,
            &params(false),
            &parents,
            &refs(&articles, &pages, &users, &renderer),
        )
        .unwrap();
        let migrated = outcome.row.unwrap();
        assert!(migrated.is_admin_reply);
        assert_eq!(migrated.moderator_id, Some(7));
        assert_eq!(migrated.author_email, "");
    }

    #[test]
    fn soft_deleted_comment_forced_to_recycled() {
        let renderer = ComrakMarkdownRenderer;
        let articles: HashSet<i64> = [1].into_iter().collect();
        let pages: HashSet<i64> = HashSet::new();
        let users: HashSet<i64> = HashSet::new();
        let parents: HashMap<i64, i64> = [(1, 0)].into_iter().collect();
        let mut row = legacy(1);
        row.deleted_at = Some("2021-06-01 00:00:00".to_owned());
        let outcome = transform(
            &row,
            &params(false),
            &parents,
            &refs(&articles, &pages, &users, &renderer),
        )
        .unwrap();
        assert_eq!(outcome.row.unwrap().status, "recycled");
        assert_eq!(outcome.repaired, 1);
    }
}
