//! `articles` → `articles`：状态机映射、`keywords` 拆 text[]、rendered_html 用
//! `ComrakMarkdownRenderer` 重渲染（旧 `md_content` 进 Archive）、访问密码 bcrypt 原样搬运。

use std::collections::HashSet;

use anyhow::{Context, bail};
use sqlx::{FromRow, MySqlPool};
use time::OffsetDateTime;

use aries_core::content::MarkdownRenderer;
use aries_infra::ComrakMarkdownRenderer;

use super::{
    ArchiveEntry, CommonParams, TransformOutcome, apply_outcome, optional_time, required_time,
    tinyint_to_bool,
};
use crate::context::MigrateCtx;

pub const TABLE: &str = "articles";

#[derive(Debug, Clone, FromRow)]
pub struct LegacyArticle {
    pub id: i64,
    pub user_id: i64,
    pub category_id: i64,
    pub order_id: i64,
    pub is_top: i64,
    pub is_recycled: i64,
    pub is_published: i64,
    pub is_allow_commented: i64,
    pub pwd: String,
    pub url: String,
    pub title: String,
    pub summary: String,
    pub img: String,
    pub content: String,
    pub md_content: String,
    pub keywords: String,
    pub comment_count: i64,
    pub visit_count: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewArticle {
    pub id: i64,
    pub author_id: i64,
    pub category_id: Option<i64>,
    pub status: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    pub cover_url: Option<String>,
    pub markdown_source: String,
    pub rendered_html: String,
    pub seo_keywords: Vec<String>,
    pub access_password_hash: Option<String>,
    pub allow_comments: bool,
    pub is_pinned: bool,
    pub sort_order: i32,
    pub comment_count: i64,
    pub visit_count: i64,
    pub published_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub deleted_at: Option<OffsetDateTime>,
}

pub async fn extract(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacyArticle>> {
    sqlx::query_as::<_, LegacyArticle>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(user_id AS SIGNED) AS user_id, \
         CAST(category_id AS SIGNED) AS category_id, CAST(order_id AS SIGNED) AS order_id, \
         CAST(is_top AS SIGNED) AS is_top, CAST(is_recycled AS SIGNED) AS is_recycled, \
         CAST(is_published AS SIGNED) AS is_published, \
         CAST(is_allow_commented AS SIGNED) AS is_allow_commented, \
         pwd, url, title, summary, img, content, md_content, keywords, \
         CAST(comment_count AS SIGNED) AS comment_count, CAST(visit_count AS SIGNED) AS visit_count, \
         CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at, \
         CAST(deleted_at AS CHAR) AS deleted_at \
         FROM articles ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy articles")
}

/// 状态机：recycled 优先于 published。
pub fn map_status(is_published: bool, is_recycled: bool) -> &'static str {
    if is_recycled {
        "recycled"
    } else if is_published {
        "published"
    } else {
        "draft"
    }
}

/// `keywords` 逗号分隔 → text[]；trim 并去空。
pub fn split_keywords(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

pub struct ArticleRefs<'a> {
    pub user_ids: &'a HashSet<i64>,
    pub category_ids: &'a HashSet<i64>,
    pub renderer: &'a ComrakMarkdownRenderer,
}

pub fn transform(
    row: &LegacyArticle,
    params: &CommonParams,
    refs: &ArticleRefs,
) -> anyhow::Result<TransformOutcome<NewArticle>> {
    let is_published = tinyint_to_bool(row.is_published, TABLE, row.id, "is_published")?;
    let is_recycled = tinyint_to_bool(row.is_recycled, TABLE, row.id, "is_recycled")?;
    let status = map_status(is_published, is_recycled);

    let created_at = required_time(
        row.created_at.as_deref(),
        params.offset,
        TABLE,
        row.id,
        "created_at",
    )?;

    // published 必须有 published_at；旧库无发布时间列，统一用 created_at 兜底，
    // 由 migrate 聚合计数后记录一条 note。
    let published_at = if status == "published" {
        Some(created_at)
    } else {
        None
    };

    // published 的内容 CHECK：title / slug / markdown_source / rendered_html 非空白。
    if status == "published" {
        for (field, value) in [
            ("title", row.title.trim()),
            ("url", row.url.trim()),
            ("content", row.content.trim()),
        ] {
            if value.is_empty() {
                bail!(
                    "articles #{}: published article has empty `{field}`; \
                     fix the source row before migrating",
                    row.id
                );
            }
        }
    }

    if row.comment_count < 0 || row.visit_count < 0 {
        bail!(
            "articles #{}: negative comment_count/visit_count violates target CHECK",
            row.id
        );
    }

    let mut archives = Vec::new();
    let mut repaired = 0;
    let mut notes = Vec::new();

    super::archive_unmapped_field(
        &mut archives,
        row.id,
        "md_content",
        &row.md_content,
        "legacy rendered HTML replaced by fresh comrak render",
    );

    // slug：published 已在上方保证非空；草稿空 url 生成占位 slug（citext UNIQUE 不允许重复空串以外的冲突）。
    let slug = if row.url.trim().is_empty() {
        repaired += 1;
        notes.push(format!(
            "#{}: empty slug replaced with generated value",
            row.id
        ));
        format!("article-{}", row.id)
    } else {
        row.url.clone()
    };

    let category_id = match row.category_id {
        0 => None,
        id if refs.category_ids.contains(&id) => Some(id),
        dangling => {
            repaired += 1;
            archives.push(ArchiveEntry {
                id: Some(row.id),
                fields: serde_json::json!({ "category_id": dangling }),
                reason: "dangling category reference; reset to NULL".to_owned(),
                whole_row: false,
            });
            None
        }
    };

    // author_id 为 ON DELETE RESTRICT 必填；悬挂属硬错误（Preflight 应先拦截）。
    if !refs.user_ids.contains(&row.user_id) {
        bail!(
            "articles #{}: dangling author user_id {}; migrate users first or fix the source row",
            row.id,
            row.user_id
        );
    }

    Ok(TransformOutcome {
        row: Some(NewArticle {
            id: row.id,
            author_id: row.user_id,
            category_id,
            status: status.to_owned(),
            slug,
            title: row.title.clone(),
            summary: row.summary.clone(),
            cover_url: if row.img.trim().is_empty() {
                None
            } else {
                Some(row.img.clone())
            },
            markdown_source: row.content.clone(),
            rendered_html: refs.renderer.render(&row.content)?,
            seo_keywords: split_keywords(&row.keywords),
            // 访问密码 bcrypt Hash 原样搬运，验证链路兼容。
            access_password_hash: if row.pwd.is_empty() {
                None
            } else {
                Some(row.pwd.clone())
            },
            allow_comments: tinyint_to_bool(
                row.is_allow_commented,
                TABLE,
                row.id,
                "is_allow_commented",
            )?,
            is_pinned: tinyint_to_bool(row.is_top, TABLE, row.id, "is_top")?,
            sort_order: i32::try_from(row.order_id)
                .context("articles: order_id out of integer range")?,
            comment_count: row.comment_count,
            visit_count: row.visit_count,
            published_at,
            created_at,
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
        }),
        archives,
        repaired,
        notes,
    })
}

pub async fn migrate(ctx: &mut MigrateCtx) -> anyhow::Result<()> {
    let rows = extract(&ctx.mysql).await?;
    let params = CommonParams {
        offset: ctx.offset,
        truncate_violations: ctx.truncate_violations,
    };
    ctx.report.table(TABLE).source_rows = rows.len() as i64;

    let user_ids = super::fetch_id_set(&ctx.mysql, "users").await?;
    let category_ids = super::fetch_id_set(&ctx.mysql, "categories").await?;
    // Renderer 为 Copy 值类型，拷出以避免与 apply_outcome 的可变借用冲突。
    let renderer = ctx.renderer;
    let refs = ArticleRefs {
        user_ids: &user_ids,
        category_ids: &category_ids,
        renderer: &renderer,
    };

    let mut new_rows = Vec::with_capacity(rows.len());
    for row in &rows {
        let outcome = transform(row, &params, &refs)?;
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
                "INSERT INTO articles (id, author_id, category_id, status, slug, title, summary, \
                 cover_url, markdown_source, rendered_html, seo_keywords, access_password_hash, \
                 allow_comments, is_pinned, sort_order, comment_count, visit_count, published_at, \
                 created_at, updated_at, deleted_at, version) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
                 $17, $18, $19, $20, $21, 1) ON CONFLICT (id) DO NOTHING",
            )
            .bind(row.id)
            .bind(row.author_id)
            .bind(row.category_id)
            .bind(&row.status)
            .bind(&row.slug)
            .bind(&row.title)
            .bind(&row.summary)
            .bind(&row.cover_url)
            .bind(&row.markdown_source)
            .bind(&row.rendered_html)
            .bind(&row.seo_keywords)
            .bind(&row.access_password_hash)
            .bind(row.allow_comments)
            .bind(row.is_pinned)
            .bind(row.sort_order)
            .bind(row.comment_count)
            .bind(row.visit_count)
            .bind(row.published_at)
            .bind(row.created_at)
            .bind(row.updated_at)
            .bind(row.deleted_at)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("failed to insert article #{}", row.id))?
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
    let published_count = new_rows
        .iter()
        .filter(|row| row.status == "published")
        .count();
    if published_count > 0 {
        report.note(format!(
            "{published_count} published articles use created_at as published_at (legacy schema has no publish time)"
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

    fn refs<'a>(
        users: &'a HashSet<i64>,
        categories: &'a HashSet<i64>,
        renderer: &'a ComrakMarkdownRenderer,
    ) -> ArticleRefs<'a> {
        ArticleRefs {
            user_ids: users,
            category_ids: categories,
            renderer,
        }
    }

    fn legacy(id: i64) -> LegacyArticle {
        LegacyArticle {
            id,
            user_id: 1,
            category_id: 1,
            order_id: 0,
            is_top: 0,
            is_recycled: 0,
            is_published: 1,
            is_allow_commented: 1,
            pwd: String::new(),
            url: "hello-world".to_owned(),
            title: "Hello".to_owned(),
            summary: String::new(),
            img: String::new(),
            content: "# Hello".to_owned(),
            md_content: "<h1>Hello</h1>".to_owned(),
            keywords: "rust, 博客 , ,".to_owned(),
            comment_count: 2,
            visit_count: 10,
            created_at: Some("2020-01-02 03:04:05".to_owned()),
            updated_at: Some("2020-01-02 03:04:05".to_owned()),
            deleted_at: None,
        }
    }

    #[test]
    fn status_machine_covers_all_branches() {
        assert_eq!(map_status(false, false), "draft");
        assert_eq!(map_status(true, false), "published");
        assert_eq!(map_status(true, true), "recycled");
        assert_eq!(map_status(false, true), "recycled");
    }

    #[test]
    fn keywords_split_trims_and_drops_empty() {
        assert_eq!(split_keywords("rust, 博客 , ,"), vec!["rust", "博客"]);
        assert!(split_keywords("").is_empty());
        assert!(split_keywords(" , ,").is_empty());
    }

    #[test]
    fn published_article_backfills_published_at_and_rerenders_html() {
        let users: HashSet<i64> = [1].into_iter().collect();
        let categories: HashSet<i64> = [1].into_iter().collect();
        let renderer = ComrakMarkdownRenderer;
        let outcome =
            transform(&legacy(1), &params(), &refs(&users, &categories, &renderer)).unwrap();
        let row = outcome.row.unwrap();
        assert_eq!(row.status, "published");
        assert_eq!(row.published_at, Some(row.created_at));
        assert!(row.rendered_html.contains("<h1>Hello</h1>"));
        assert_eq!(row.seo_keywords, vec!["rust", "博客"]);
        // 旧渲染 HTML 进 Archive。
        assert!(
            outcome
                .archives
                .iter()
                .any(|a| a.fields.get("md_content").is_some())
        );
    }

    #[test]
    fn dangling_category_repaired_dangling_author_hard_error() {
        let users: HashSet<i64> = [1].into_iter().collect();
        let categories: HashSet<i64> = [1].into_iter().collect();
        let renderer = ComrakMarkdownRenderer;

        let mut row = legacy(1);
        row.category_id = 99;
        let outcome = transform(&row, &params(), &refs(&users, &categories, &renderer)).unwrap();
        assert_eq!(outcome.row.unwrap().category_id, None);
        assert_eq!(outcome.repaired, 1);

        let mut row = legacy(1);
        row.user_id = 99;
        assert!(transform(&row, &params(), &refs(&users, &categories, &renderer)).is_err());
    }

    #[test]
    fn published_with_empty_content_is_hard_error() {
        let users: HashSet<i64> = [1].into_iter().collect();
        let categories: HashSet<i64> = [1].into_iter().collect();
        let renderer = ComrakMarkdownRenderer;
        let mut row = legacy(1);
        row.title = String::new();
        assert!(transform(&row, &params(), &refs(&users, &categories, &renderer)).is_err());
    }

    #[test]
    fn access_password_copied_verbatim() {
        let users: HashSet<i64> = [1].into_iter().collect();
        let categories: HashSet<i64> = [1].into_iter().collect();
        let renderer = ComrakMarkdownRenderer;
        let mut row = legacy(1);
        row.pwd = "$2y$10$accesspasswordhash".to_owned();
        let outcome = transform(&row, &params(), &refs(&users, &categories, &renderer)).unwrap();
        assert_eq!(
            outcome.row.unwrap().access_password_hash.as_deref(),
            Some("$2y$10$accesspasswordhash")
        );
    }

    #[test]
    fn recycled_wins_over_published() {
        let users: HashSet<i64> = [1].into_iter().collect();
        let categories: HashSet<i64> = [1].into_iter().collect();
        let renderer = ComrakMarkdownRenderer;
        let mut row = legacy(1);
        row.is_recycled = 1;
        let outcome = transform(&row, &params(), &refs(&users, &categories, &renderer)).unwrap();
        let row = outcome.row.unwrap();
        assert_eq!(row.status, "recycled");
        assert_eq!(row.published_at, None);
    }
}
