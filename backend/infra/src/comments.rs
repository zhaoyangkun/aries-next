//! 评论 Repository 的 PostgreSQL 实现。
//! 层级查询使用 CTE 重组 Thread，Dashboard 聚合使用单次多表统计。

use crate::like::like_pattern;
use crate::logged::{logged_query, logged_query_as, logged_query_scalar};
use crate::where_clause::WhereBuilder;
use aries_core::comments::{
    AdminReply, AiAssessment, Comment, CommentError, CommentListQuery, CommentPage,
    CommentRepository, CommentStatusChange, CommentTargetType, DashboardStats, NewComment,
};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresCommentRepository {
    pool: PgPool,
}

impl PostgresCommentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 数据库行映射，字段名与 Migration DDL 一一对应。
#[derive(Debug, FromRow)]
struct CommentRow {
    id: i64,
    target_type: String,
    target_id: i64,
    root_id: Option<i64>,
    parent_id: Option<i64>,
    author_name: String,
    author_email: String,
    author_url: Option<String>,
    content_markdown: String,
    content_html: String,
    status: String,
    moderator_id: Option<i64>,
    moderated_at: Option<OffsetDateTime>,
    moderation_reason: Option<String>,
    ip_hash: String,
    user_agent_digest: String,
    is_admin_reply: bool,
    ai_risk: Option<String>,
    ai_reason: Option<String>,
    ai_confidence: Option<f32>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl TryFrom<CommentRow> for Comment {
    type Error = CommentError;

    fn try_from(row: CommentRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            target_type: row
                .target_type
                .parse()
                .map_err(|_| CommentError::InvalidTargetType)?,
            target_id: row.target_id,
            root_id: row.root_id,
            parent_id: row.parent_id,
            author_name: row.author_name,
            author_email: row.author_email,
            author_url: row.author_url,
            content_markdown: row.content_markdown,
            content_html: row.content_html,
            status: row
                .status
                .parse()
                .map_err(|_| CommentError::InvalidStatus)?,
            moderator_id: row.moderator_id,
            moderated_at: row.moderated_at,
            moderation_reason: row.moderation_reason,
            ip_hash: row.ip_hash,
            user_agent_digest: row.user_agent_digest,
            is_admin_reply: row.is_admin_reply,
            ai_risk: row
                .ai_risk
                .map(|risk| risk.parse())
                .transpose()
                .map_err(|_| CommentError::Validation)?,
            ai_reason: row.ai_reason,
            ai_confidence: row.ai_confidence,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

const COMMENT_COLUMNS: &str = "id, target_type, target_id, root_id, parent_id, \
    author_name, author_email, author_url, content_markdown, content_html, status, \
    moderator_id, moderated_at, moderation_reason, ip_hash, user_agent_digest, \
    is_admin_reply, ai_risk, ai_reason, ai_confidence, created_at, updated_at";

fn map_sqlx(error: sqlx::Error) -> CommentError {
    tracing::error!(error = %error, "comment repository operation failed");
    CommentError::StoreUnavailable
}

/// 在同一事务内把 `articles.comment_count` 重建为该文章的真实 approved 评论数。
///
/// 采用重建（recompute）而非增减计数：审核/回复操作低频，重建只需一次索引计数，
/// 且能自愈任何历史漂移，无需在每个调用点追踪旧状态。非 article 目标
/// （page/link）没有 comment_count 列，直接跳过。
async fn sync_article_comment_count(
    tx: &mut Transaction<'_, Postgres>,
    target_type: &str,
    target_id: i64,
) -> Result<(), CommentError> {
    if target_type != CommentTargetType::Article.as_str() {
        return Ok(());
    }
    logged_query(
        "UPDATE articles SET comment_count = ( \
            SELECT COUNT(*) FROM comments \
            WHERE target_type = 'article' AND target_id = $1 \
              AND status = 'approved' AND deleted_at IS NULL) \
         WHERE id = $1",
    )
    .bind(target_id)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

#[async_trait]
impl CommentRepository for PostgresCommentRepository {
    async fn create(&self, new_comment: NewComment) -> Result<Comment, CommentError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        // 重复提交防护：同 Email + Target + 相同内容在 5 分钟窗口内视为重复。
        // 防 TOCTOU：先取事务级咨询锁（锁键由 目标+作者+内容 派生），
        // 并发相同提交在锁上串行——后进入的事务等前者提交后再查重，
        // 因此能看到已提交的重复行，不会双双通过 EXISTS 检查。
        let lock_key = format!(
            "{}|{}|{}|{}",
            new_comment.target_type.as_str(),
            new_comment.target_id,
            new_comment.author_email,
            new_comment.content_markdown,
        );
        logged_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(&lock_key)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        let duplicate = logged_query_scalar::<bool>(
            "SELECT EXISTS(SELECT 1 FROM comments \
             WHERE target_type = $1 AND target_id = $2 AND author_email = $3 \
               AND content_markdown = $4 AND created_at > now() - interval '5 minutes' \
               AND deleted_at IS NULL)",
        )
        .bind(new_comment.target_type.as_str())
        .bind(new_comment.target_id)
        .bind(&new_comment.author_email)
        .bind(&new_comment.content_markdown)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if duplicate {
            return Err(CommentError::Duplicate);
        }

        // 插入根评论行；root_id 暂时为 NULL，插入后回填。
        // content_html 由 HTTP 层渲染传入；status 由站点评论策略决定。
        let insert_sql = format!(
            "INSERT INTO comments \
             (target_type, target_id, root_id, parent_id, author_name, author_email, \
              author_url, content_markdown, content_html, status, ip_hash, user_agent_digest) \
             VALUES ($1, $2, NULL, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             RETURNING {COMMENT_COLUMNS}"
        );

        let row = logged_query_as::<CommentRow>(&insert_sql)
            .bind(new_comment.target_type.as_str())
            .bind(new_comment.target_id)
            .bind(new_comment.parent_id)
            .bind(&new_comment.author_name)
            .bind(&new_comment.author_email)
            .bind(&new_comment.author_url)
            .bind(&new_comment.content_markdown)
            .bind(&new_comment.content_html)
            .bind(new_comment.initial_status.as_str())
            .bind(&new_comment.ip_hash)
            .bind(&new_comment.user_agent_digest)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        // 回填 root_id：如果有 parent_id 则继承其 root_id，否则 root_id = 自身 id。
        let root_id = if let Some(parent_id) = new_comment.parent_id {
            let parent_root: (i64,) =
                sqlx::query_as("SELECT COALESCE(root_id, id) FROM comments WHERE id = $1")
                    .bind(parent_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(map_sqlx)?;
            parent_root.0
        } else {
            row.id
        };

        // 回填 root_id 和渲染后的 HTML。
        logged_query("UPDATE comments SET root_id = $1 WHERE id = $2")
            .bind(root_id)
            .bind(row.id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        // 维护 articles.comment_count（pending 评论计入结果为 0，重建无副作用）。
        sync_article_comment_count(
            &mut tx,
            new_comment.target_type.as_str(),
            new_comment.target_id,
        )
        .await?;

        tx.commit().await.map_err(map_sqlx)?;

        let mut comment: Comment = row.try_into()?;
        comment.root_id = Some(root_id);
        Ok(comment)
    }

    async fn create_admin_reply(&self, reply: AdminReply) -> Result<Comment, CommentError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        // 管理员回复没有访客 Email（列允许空）；署名取操作管理员的展示名。
        let insert_sql = format!(
            "INSERT INTO comments \
             (target_type, target_id, root_id, parent_id, author_name, author_email, \
              content_markdown, content_html, status, is_admin_reply, \
              moderator_id, moderated_at) \
             VALUES ($1, $2, NULL, $3, $4, '', $5, $6, 'approved', true, $7, now()) \
             RETURNING {COMMENT_COLUMNS}"
        );

        let row = logged_query_as::<CommentRow>(&insert_sql)
            .bind(reply.target_type.as_str())
            .bind(reply.target_id)
            .bind(reply.parent_id)
            .bind(&reply.author_name)
            .bind(&reply.content_markdown)
            .bind(&reply.content_html)
            .bind(reply.moderator_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        let root_id = if let Some(parent_id) = reply.parent_id {
            let parent_root: (i64,) =
                sqlx::query_as("SELECT COALESCE(root_id, id) FROM comments WHERE id = $1")
                    .bind(parent_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(map_sqlx)?;
            parent_root.0
        } else {
            row.id
        };

        logged_query("UPDATE comments SET root_id = $1 WHERE id = $2")
            .bind(root_id)
            .bind(row.id)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        // 管理员回复恒为 approved，直接反映到 articles.comment_count。
        sync_article_comment_count(&mut tx, reply.target_type.as_str(), reply.target_id).await?;

        tx.commit().await.map_err(map_sqlx)?;

        let mut comment: Comment = row.try_into()?;
        comment.root_id = Some(root_id);
        Ok(comment)
    }

    async fn find(&self, comment_id: i64) -> Result<Option<Comment>, CommentError> {
        let query =
            format!("SELECT {COMMENT_COLUMNS} FROM comments WHERE id = $1 AND deleted_at IS NULL");
        let row = logged_query_as::<CommentRow>(&query)
            .bind(comment_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            Some(r) => Ok(Some(r.try_into()?)),
            None => Ok(None),
        }
    }

    async fn list(&self, query: CommentListQuery) -> Result<CommentPage, CommentError> {
        let offset = (query.page.saturating_sub(1)) * query.page_size;
        let limit = i64::from(query.page_size);

        // count 与 data 共享同一份条件构建，bind 序列从构造上保持一致。
        let mut where_ = WhereBuilder::new();
        where_.push_static("deleted_at IS NULL");
        if let Some(ref tt) = query.target_type {
            where_.push("target_type = {}", tt.as_str());
        }
        if let Some(tid) = query.target_id {
            where_.push("target_id = {}", tid);
        }
        if let Some(ref st) = query.status {
            where_.push("status = {}", st.as_str());
        }
        if let Some(ref kw) = query.keyword {
            where_.push(
                "(content_markdown ILIKE {} ESCAPE '\\' \
                 OR author_name ILIKE {} ESCAPE '\\')",
                like_pattern(kw),
            );
        }
        let clause = where_.clause();

        // 计数查询。
        let count_sql = format!("SELECT COUNT(*) FROM comments {clause}");
        let total = where_
            .bind_to(logged_query_scalar::<i64>(&count_sql))
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;

        // 列表查询。
        let next_index = where_.next_index();
        let data_sql = format!(
            "SELECT {COMMENT_COLUMNS} FROM comments {clause} \
             ORDER BY created_at DESC LIMIT ${next_index} OFFSET ${}",
            next_index + 1
        );
        let rows = where_
            .bind_to(logged_query_as::<CommentRow>(&data_sql))
            .bind(limit)
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let items: Vec<Comment> = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(CommentPage {
            items,
            total,
            page: query.page,
            page_size: query.page_size,
        })
    }

    async fn list_approved_by_target(
        &self,
        target_type: CommentTargetType,
        target_id: i64,
    ) -> Result<Vec<Comment>, CommentError> {
        // 拉取该 Target 下所有已批准的根评论及其直接回复（最多 2 层）。
        // UNION 结果不允许按表达式排序（PostgreSQL 限制），故包一层子查询。
        let query = format!(
            "SELECT * FROM ( \
             WITH roots AS ( \
                SELECT {COMMENT_COLUMNS}, 0 AS depth \
                FROM comments \
                WHERE target_type = $1 AND target_id = $2 AND status = 'approved' \
                  AND root_id = id AND deleted_at IS NULL \
             ), \
             replies AS ( \
                SELECT {COMMENT_COLUMNS}, 1 AS depth \
                FROM comments \
                WHERE target_type = $1 AND target_id = $2 AND status = 'approved' \
                  AND root_id != id AND deleted_at IS NULL \
             ) \
             SELECT * FROM roots \
             UNION ALL \
             SELECT * FROM replies \
             ) combined \
             ORDER BY \
               CASE WHEN depth = 0 THEN id ELSE root_id END, \
               depth, created_at"
        );

        let rows = logged_query_as::<CommentRow>(&query)
            .bind(target_type.as_str())
            .bind(target_id)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;

        rows.into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()
    }

    async fn change_status(&self, change: CommentStatusChange) -> Result<Comment, CommentError> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;

        let query = format!(
            "UPDATE comments SET status = $1, moderator_id = $2, moderated_at = now(), \
             moderation_reason = $3, updated_at = now() \
             WHERE id = $4 AND deleted_at IS NULL \
             RETURNING {COMMENT_COLUMNS}"
        );
        let row = logged_query_as::<CommentRow>(&query)
            .bind(change.new_status.as_str())
            .bind(change.moderator_id)
            .bind(&change.reason)
            .bind(change.comment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(CommentError::NotFound)?;

        // 状态变更可能影响 approved 计数（审核/回收/恢复），同事务重建。
        sync_article_comment_count(&mut tx, &row.target_type, row.target_id).await?;

        tx.commit().await.map_err(map_sqlx)?;

        row.try_into()
    }

    async fn delete(&self, comment_id: i64) -> Result<(), CommentError> {
        // 仅允许物理删除 recycled 评论；recycled 不计入 approved 计数，
        // 因此本操作不会改变 articles.comment_count，无需同步。
        let result = logged_query(
            "UPDATE comments SET deleted_at = now(), updated_at = now() \
             WHERE id = $1 AND status = 'recycled' AND deleted_at IS NULL",
        )
        .bind(comment_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if result.rows_affected() == 0 {
            return Err(CommentError::NotFound);
        }
        Ok(())
    }

    async fn apply_ai_assessment(&self, assessment: AiAssessment) -> Result<Comment, CommentError> {
        // spam 结论同时把状态置为 spam；其余结论只记录风险字段，不改变原状态。
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let query = format!(
            "UPDATE comments SET ai_risk = $2, ai_reason = $3, ai_confidence = $4, \
             status = CASE WHEN $2 = 'spam' THEN 'spam' ELSE status END, \
             updated_at = now() \
             WHERE id = $1 AND deleted_at IS NULL \
             RETURNING {COMMENT_COLUMNS}"
        );
        let row = logged_query_as::<CommentRow>(&query)
            .bind(assessment.comment_id)
            .bind(assessment.risk.as_str())
            .bind(&assessment.reason)
            .bind(assessment.confidence)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .ok_or(CommentError::NotFound)?;

        // spam 标记可能把 approved 评论移出计数，同事务重建。
        sync_article_comment_count(&mut tx, &row.target_type, row.target_id).await?;

        tx.commit().await.map_err(map_sqlx)?;
        row.try_into()
    }

    async fn count_approved(
        &self,
        target_type: CommentTargetType,
        target_id: i64,
    ) -> Result<i64, CommentError> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM comments \
             WHERE target_type = $1 AND target_id = $2 AND status = 'approved' AND deleted_at IS NULL",
        )
        .bind(target_type.as_str())
        .bind(target_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(count.0)
    }

    async fn dashboard_stats(&self) -> Result<DashboardStats, CommentError> {
        // 文章按状态分组统计。
        let article_stats = logged_query_as::<(String, i64)>(
            "SELECT status, COUNT(*) FROM articles GROUP BY status",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut article_total = 0i64;
        let mut article_draft = 0i64;
        let mut article_published = 0i64;
        let mut article_recycled = 0i64;
        for (status, count) in article_stats {
            article_total += count;
            match status.as_str() {
                "draft" => article_draft = count,
                "published" => article_published = count,
                "recycled" => article_recycled = count,
                _ => {}
            }
        }

        // 评论统计。
        let comment_total: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM comments WHERE deleted_at IS NULL")
                .fetch_one(&self.pool)
                .await
                .map_err(map_sqlx)?;

        let comment_pending: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM comments WHERE status = 'pending' AND deleted_at IS NULL",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let comment_today: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM comments WHERE created_at >= CURRENT_DATE AND deleted_at IS NULL",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        // 最近 5 条待审核评论。
        let recent_pending_sql = format!(
            "SELECT {COMMENT_COLUMNS} FROM comments \
             WHERE status = 'pending' AND deleted_at IS NULL \
             ORDER BY created_at DESC LIMIT 5"
        );
        let recent_pending_rows = logged_query_as::<CommentRow>(&recent_pending_sql)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let recent_pending_comments: Vec<Comment> = recent_pending_rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        // 最近 5 条失败的后台任务。
        let recent_failed = logged_query_as::<(i64, String, serde_json::Value, String, i32, i32, OffsetDateTime, Option<String>, OffsetDateTime, OffsetDateTime)>(
            "SELECT id, kind, payload, status, attempts, max_attempts, run_at, last_error, created_at, updated_at \
             FROM background_jobs WHERE status = 'failed' ORDER BY updated_at DESC LIMIT 5"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let recent_failed_jobs = recent_failed
            .into_iter()
            .map(|row| aries_core::jobs::BackgroundJob {
                id: row.0,
                kind: row
                    .1
                    .parse()
                    .unwrap_or(aries_core::jobs::JobKind::MediaCleanup),
                payload: row.2,
                status: aries_core::jobs::JobStatus::Failed,
                attempts: row.4,
                max_attempts: row.5,
                run_at: row.6,
                last_error: row.7,
                created_at: row.8,
                updated_at: row.9,
            })
            .collect();

        Ok(DashboardStats {
            article_total,
            article_draft,
            article_published,
            article_recycled,
            comment_total: comment_total.0,
            comment_pending: comment_pending.0,
            comment_today: comment_today.0,
            recent_pending_comments,
            recent_failed_jobs,
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, ensure};
    use uuid::Uuid;

    use super::*;
    use aries_core::comments::{
        AdminReply, AiAssessment, AiRisk, CommentStatus, CommentStatusChange, NewComment,
    };

    /// 集成：articles.comment_count 必须随评论审核/删除/恢复等状态变更保持一致。
    /// 覆盖路径：create（pending/approved）、create_admin_reply、change_status
    /// （审核/回收/恢复/标 spam）、apply_ai_assessment（spam）、delete（recycled 物理删除）。
    #[tokio::test]
    async fn postgresql_repository_maintains_article_comment_count() -> anyhow::Result<()> {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let _ = dotenvy::dotenv();
        let base_config = crate::PostgresConfig::from_env()?;
        let admin_pool = crate::connect_postgres(&base_config).await?;
        let test_schema = format!("aries_test_{}", Uuid::now_v7().simple());

        logged_query(&format!("CREATE SCHEMA \"{test_schema}\""))
            .execute(&admin_pool)
            .await
            .context("failed to create isolated comment test schema")?;

        let test_config = crate::PostgresConfig {
            options: base_config.options.clone(),
            schema: format!("{test_schema},{}", base_config.schema()),
            slow_query_ms: 0,
        };
        let test_pool = crate::connect_postgres(&test_config).await?;
        let scenario_result = async {
            crate::run_migrations(&test_pool).await?;
            run_comment_count_scenario(&test_pool).await
        }
        .await;

        test_pool.close().await;
        let cleanup_result = logged_query(&format!("DROP SCHEMA \"{test_schema}\" CASCADE"))
            .execute(&admin_pool)
            .await
            .context("failed to remove isolated comment test schema");
        admin_pool.close().await;

        scenario_result?;
        cleanup_result?;
        Ok(())
    }

    async fn article_comment_count(pool: &PgPool, article_id: i64) -> anyhow::Result<i64> {
        let count: i64 = sqlx::query_scalar("SELECT comment_count FROM articles WHERE id = $1")
            .bind(article_id)
            .fetch_one(pool)
            .await?;
        Ok(count)
    }

    fn new_comment(article_id: i64, name: &str, status: CommentStatus) -> NewComment {
        NewComment {
            target_type: CommentTargetType::Article,
            target_id: article_id,
            parent_id: None,
            author_name: name.to_owned(),
            author_email: format!("{name}@example.com"),
            author_url: None,
            content_markdown: format!("content by {name}"),
            content_html: format!("<p>content by {name}</p>"),
            initial_status: status,
            ip_hash: "ip-hash".to_owned(),
            user_agent_digest: "ua-digest".to_owned(),
        }
    }

    async fn run_comment_count_scenario(pool: &PgPool) -> anyhow::Result<()> {
        let repository = PostgresCommentRepository::new(pool.clone());

        // 种子数据：一个已发布文章（author_id 有外键约束，先建用户）。
        let author_id: i64 = sqlx::query_scalar(
            "INSERT INTO users (username, email, password_hash, display_name) \
             VALUES ('author', 'author@example.com', 'hash', 'Author') RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        let article_id: i64 = sqlx::query_scalar(
            "INSERT INTO articles (author_id, status, slug, title, markdown_source, rendered_html, published_at) \
             VALUES ($1, 'published', 'hello', 'Hello', 'md', 'html', now()) RETURNING id",
        )
        .bind(author_id)
        .fetch_one(pool)
        .await?;

        // 待审核评论不计数。
        let pending = repository
            .create(new_comment(article_id, "alice", CommentStatus::Pending))
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == 0);

        // 审核通过 → +1。
        repository
            .change_status(CommentStatusChange {
                comment_id: pending.id,
                new_status: CommentStatus::Approved,
                moderator_id: author_id,
                reason: None,
            })
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == 1);

        // 策略自动批准的评论 → +1。
        let auto = repository
            .create(new_comment(article_id, "bob", CommentStatus::Approved))
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == 2);

        // 管理员回复恒为 approved → +1。
        repository
            .create_admin_reply(AdminReply {
                target_type: CommentTargetType::Article,
                target_id: article_id,
                parent_id: Some(pending.id),
                author_name: "Author".to_owned(),
                content_markdown: "reply".to_owned(),
                content_html: "<p>reply</p>".to_owned(),
                moderator_id: author_id,
            })
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == 3);

        // 回收 → -1；恢复 → +1。
        repository
            .change_status(CommentStatusChange {
                comment_id: auto.id,
                new_status: CommentStatus::Recycled,
                moderator_id: author_id,
                reason: None,
            })
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == 2);
        repository
            .change_status(CommentStatusChange {
                comment_id: auto.id,
                new_status: CommentStatus::Approved,
                moderator_id: author_id,
                reason: Some("restore".to_owned()),
            })
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == 3);

        // 标记 spam → -1。
        repository
            .change_status(CommentStatusChange {
                comment_id: auto.id,
                new_status: CommentStatus::Spam,
                moderator_id: author_id,
                reason: None,
            })
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == 2);

        // AI 审核把 approved 评论判为 spam → -1。
        repository
            .apply_ai_assessment(AiAssessment {
                comment_id: pending.id,
                risk: AiRisk::Spam,
                reason: Some("ai flagged".to_owned()),
                confidence: Some(0.99),
            })
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == 1);

        // recycled 物理删除不影响 approved 计数。
        repository
            .change_status(CommentStatusChange {
                comment_id: auto.id,
                new_status: CommentStatus::Recycled,
                moderator_id: author_id,
                reason: None,
            })
            .await?;
        repository.delete(auto.id).await?;
        ensure!(article_comment_count(pool, article_id).await? == 1);

        // 计数必须与 count_approved 一致。
        ensure!(
            repository
                .count_approved(CommentTargetType::Article, article_id)
                .await?
                == 1
        );

        // 非 article 目标不触碰 articles.comment_count。
        let before = article_comment_count(pool, article_id).await?;
        repository
            .create(NewComment {
                target_type: CommentTargetType::Page,
                target_id: 1,
                ..new_comment(article_id, "carol", CommentStatus::Approved)
            })
            .await?;
        ensure!(article_comment_count(pool, article_id).await? == before);

        Ok(())
    }

    /// 集成：重复提交检查在事务内并受事务级咨询锁保护。
    /// 并发相同提交被锁串行化，恰一个成功、另一个返回 Duplicate；
    /// 5 分钟窗口内的串行重复同样被拒，且只落库一条评论。
    #[tokio::test]
    async fn postgresql_repository_rejects_concurrent_duplicate_comments() -> anyhow::Result<()> {
        if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
            return Ok(());
        }
        let _ = dotenvy::dotenv();
        let base_config = crate::PostgresConfig::from_env()?;
        let admin_pool = crate::connect_postgres(&base_config).await?;
        let test_schema = format!("aries_test_{}", Uuid::now_v7().simple());

        logged_query(&format!("CREATE SCHEMA \"{test_schema}\""))
            .execute(&admin_pool)
            .await
            .context("failed to create isolated comment test schema")?;

        let test_config = crate::PostgresConfig {
            options: base_config.options.clone(),
            schema: format!("{test_schema},{}", base_config.schema()),
            slow_query_ms: 0,
        };
        let test_pool = crate::connect_postgres(&test_config).await?;
        let scenario_result = async {
            crate::run_migrations(&test_pool).await?;
            run_concurrent_duplicate_scenario(&test_pool).await
        }
        .await;

        test_pool.close().await;
        let cleanup_result = logged_query(&format!("DROP SCHEMA \"{test_schema}\" CASCADE"))
            .execute(&admin_pool)
            .await
            .context("failed to remove isolated comment test schema");
        admin_pool.close().await;

        scenario_result?;
        cleanup_result?;
        Ok(())
    }

    async fn run_concurrent_duplicate_scenario(pool: &PgPool) -> anyhow::Result<()> {
        let repository = PostgresCommentRepository::new(pool.clone());

        // 种子数据：一个已发布文章（author_id 有外键约束，先建用户）。
        let author_id: i64 = sqlx::query_scalar(
            "INSERT INTO users (username, email, password_hash, display_name) \
             VALUES ('author', 'author@example.com', 'hash', 'Author') RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        let article_id: i64 = sqlx::query_scalar(
            "INSERT INTO articles (author_id, status, slug, title, markdown_source, rendered_html, published_at) \
             VALUES ($1, 'published', 'dup', 'Dup', 'md', 'html', now()) RETURNING id",
        )
        .bind(author_id)
        .fetch_one(pool)
        .await?;

        // 完全相同的 NewComment 并发创建两次：咨询锁把两个事务串行化，
        // 后到的事务能看到先提交的行，因此恰一个成功、一个 Duplicate。
        let first = repository.create(new_comment(article_id, "dave", CommentStatus::Approved));
        let second = repository.create(new_comment(article_id, "dave", CommentStatus::Approved));
        let outcomes = tokio::join!(first, second);
        let outcomes = [outcomes.0, outcomes.1];
        let succeeded = outcomes.iter().filter(|result| result.is_ok()).count();
        let duplicated = outcomes
            .iter()
            .filter(|result| matches!(result, Err(CommentError::Duplicate)))
            .count();
        ensure!(
            succeeded == 1 && duplicated == 1,
            "expected exactly one success and one duplicate, got {succeeded} success / {duplicated} duplicate"
        );

        // 窗口内串行重复同样被拒，且只落库一条。
        let again = repository
            .create(new_comment(article_id, "dave", CommentStatus::Approved))
            .await;
        ensure!(
            matches!(again, Err(CommentError::Duplicate)),
            "sequential duplicate within the window must be rejected"
        );
        let stored: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM comments WHERE target_type = 'article' AND target_id = $1",
        )
        .bind(article_id)
        .fetch_one(pool)
        .await?;
        ensure!(
            stored == 1,
            "expected exactly one stored comment, got {stored}"
        );

        // 不同内容（不同作者）不受去重影响。
        repository
            .create(new_comment(article_id, "erin", CommentStatus::Approved))
            .await?;
        Ok(())
    }
}
