//! 只读 Preflight：实现 docs/database-mapping.md「必须执行的 Preflight」全项。
//! 输出三级结果：hard_errors（阻止 migrate）、violations（长度/空值违例，
//! `--truncate-violations` 可覆盖）、warnings（自动修复或归档的事项）。

use std::collections::BTreeMap;

use anyhow::Context;
use serde::Serialize;
use sqlx::MySqlPool;

use crate::tables::settings::is_secret_key;

const LISTING_CAP: usize = 50;

#[derive(Debug, Default, Serialize)]
pub struct PreflightReport {
    pub hard_errors: Vec<String>,
    pub violations: Vec<String>,
    pub warnings: Vec<String>,
    pub stats: BTreeMap<String, i64>,
}

impl PreflightReport {
    pub fn is_clean(&self) -> bool {
        self.hard_errors.is_empty() && self.violations.is_empty()
    }
}

pub struct PreflightOptions<'a> {
    /// 媒体 URL HEAD 抽查条数（0 = 关闭）。
    pub media_precheck: usize,
    pub source_base_url: Option<&'a str>,
    pub http_client: Option<&'a reqwest::Client>,
}

pub async fn run(
    mysql: &MySqlPool,
    options: &PreflightOptions<'_>,
) -> anyhow::Result<PreflightReport> {
    let mut report = PreflightReport::default();

    check_citext_conflicts(mysql, &mut report).await?;
    check_orphans(mysql, &mut report).await?;
    check_zero_dates(mysql, &mut report).await?;
    check_boolean_columns(mysql, &mut report).await?;
    check_article_states(mysql, &mut report).await?;
    check_tag_article_duplicates(mysql, &mut report).await?;
    check_comment_types(mysql, &mut report).await?;
    check_length_violations(mysql, &mut report).await?;
    check_setting_keys(mysql, &mut report).await?;
    if options.media_precheck > 0 {
        check_media_urls(mysql, &mut report, options).await?;
    }

    for (table, target) in [
        ("users", "users"),
        ("categories", "categories"),
        ("articles", "articles"),
        ("tags", "tags"),
        ("tag_article", "article_tags"),
        ("pictures", "media_assets"),
        ("comments", "comments"),
        ("pages", "pages"),
        ("journals", "journals"),
        ("galleries", "gallery_items"),
        ("links", "links"),
        ("navs", "navigation_items"),
        ("sys_setting_items", "settings"),
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM `{table}`"))
            .fetch_one(mysql)
            .await
            .with_context(|| format!("failed to count {table}"))?;
        report.stats.insert(format!("{table} → {target}"), count);
    }

    Ok(report)
}

fn format_listing(prefix: &str, ids: &[i64], total: i64) -> String {
    let shown: Vec<String> = ids
        .iter()
        .take(LISTING_CAP)
        .map(|id| id.to_string())
        .collect();
    let suffix = if total as usize > LISTING_CAP {
        format!(" ... ({total} total)")
    } else {
        String::new()
    };
    format!("{prefix}: {}{suffix}", shown.join(", "))
}

async fn check_citext_conflicts(
    mysql: &MySqlPool,
    report: &mut PreflightReport,
) -> anyhow::Result<()> {
    // (表, 冲突键表达式, 附加 WHERE)。slug 类列空串会在 Transform 时生成占位值，
    // 不构成冲突，故排除空串行。
    let checks: [(&str, &str, &str); 6] = [
        ("users", "LOWER(username)", ""),
        ("users", "LOWER(email)", ""),
        (
            "categories",
            "CONCAT(`type`, ':', LOWER(url))",
            "WHERE url <> ''",
        ),
        ("articles", "LOWER(url)", "WHERE url <> ''"),
        (
            "pages",
            "LOWER(url)",
            "WHERE deleted_at IS NULL AND url <> ''",
        ),
        ("tags", "LOWER(name)", ""),
    ];
    for (table, key_expr, where_clause) in checks {
        let query = format!(
            "SELECT {key_expr} AS dup_key, COUNT(*) AS c FROM `{table}` {where_clause} \
             GROUP BY dup_key HAVING c > 1 ORDER BY c DESC LIMIT {LISTING_CAP}"
        );
        let rows: Vec<(String, i64)> = sqlx::query_as(&query)
            .fetch_all(mysql)
            .await
            .with_context(|| format!("preflight: citext conflict check failed for {table}"))?;
        for (key, count) in rows {
            report.hard_errors.push(format!(
                "{table}: case-insensitive duplicate `{key}` x{count} (target column is citext UNIQUE)"
            ));
        }
    }
    Ok(())
}

async fn check_orphans(mysql: &MySqlPool, report: &mut PreflightReport) -> anyhow::Result<()> {
    // (label, sql, hard?)
    let checks: [(&str, &str, bool); 10] = [
        (
            "articles.user_id not in users",
            "SELECT CAST(a.id AS SIGNED) FROM articles a LEFT JOIN users u ON a.user_id = u.id \
             WHERE u.id IS NULL ORDER BY a.id",
            true,
        ),
        (
            "articles.category_id dangling (auto NULL repair)",
            "SELECT CAST(a.id AS SIGNED) FROM articles a LEFT JOIN categories c ON a.category_id = c.id \
             WHERE a.category_id <> 0 AND c.id IS NULL ORDER BY a.id",
            false,
        ),
        (
            "categories.parent_id dangling (auto NULL repair)",
            "SELECT CAST(c.id AS SIGNED) FROM categories c LEFT JOIN categories p ON c.parent_id = p.id \
             WHERE c.parent_id <> 0 AND p.id IS NULL ORDER BY c.id",
            false,
        ),
        (
            "tag_article dangling pairs (will be archived)",
            "SELECT CAST(ta.article_id AS SIGNED) FROM tag_article ta \
             LEFT JOIN articles a ON ta.article_id = a.id LEFT JOIN tags t ON ta.tag_id = t.id \
             WHERE a.id IS NULL OR t.id IS NULL ORDER BY ta.article_id",
            false,
        ),
        (
            "comments target dangling (will be archived)",
            "SELECT CAST(c.id AS SIGNED) FROM comments c \
             LEFT JOIN articles a ON c.`type` = 1 AND c.article_id = a.id \
             LEFT JOIN pages p ON c.`type` IN (3, 4) AND c.page_id = p.id \
             WHERE (c.`type` = 1 AND a.id IS NULL) OR (c.`type` IN (3, 4) AND p.id IS NULL) \
             ORDER BY c.id",
            false,
        ),
        (
            "comments parent dangling (promoted to top level)",
            "SELECT CAST(c.id AS SIGNED) FROM comments c LEFT JOIN comments p ON c.parent_comment_id = p.id \
             WHERE c.parent_comment_id <> 0 AND p.id IS NULL ORDER BY c.id",
            false,
        ),
        (
            "galleries.category_id not a gallery category (will be archived)",
            "SELECT CAST(g.id AS SIGNED) FROM galleries g LEFT JOIN categories c \
             ON g.category_id = c.id AND c.`type` = 2 WHERE c.id IS NULL ORDER BY g.id",
            false,
        ),
        (
            "links.category_id dangling (auto NULL repair)",
            "SELECT CAST(l.id AS SIGNED) FROM links l LEFT JOIN categories c ON l.category_id = c.id \
             WHERE l.category_id <> 0 AND c.id IS NULL ORDER BY l.id",
            false,
        ),
        (
            "navs.parent_nav_id dangling (auto NULL repair)",
            "SELECT CAST(n.id AS SIGNED) FROM navs n LEFT JOIN navs p ON n.parent_nav_id = p.id \
             WHERE n.parent_nav_id <> 0 AND p.id IS NULL ORDER BY n.id",
            false,
        ),
        (
            "pages soft-deleted slugs conflicting with live rows (partial unique index)",
            "SELECT CAST(p.id AS SIGNED) FROM pages p JOIN pages live ON LOWER(p.url) = LOWER(live.url) \
             AND live.deleted_at IS NULL WHERE p.deleted_at IS NOT NULL ORDER BY p.id",
            false,
        ),
    ];
    for (label, query, hard) in checks {
        let ids: Vec<i64> = sqlx::query_scalar(query)
            .fetch_all(mysql)
            .await
            .with_context(|| format!("preflight: orphan check failed: {label}"))?;
        if ids.is_empty() {
            continue;
        }
        let message = format_listing(label, &ids, ids.len() as i64);
        if hard {
            report.hard_errors.push(message);
        } else {
            report.warnings.push(message);
        }
    }
    Ok(())
}

async fn check_zero_dates(mysql: &MySqlPool, report: &mut PreflightReport) -> anyhow::Result<()> {
    let tables = [
        "users",
        "categories",
        "articles",
        "tags",
        "comments",
        "pages",
        "journals",
        "galleries",
        "links",
        "navs",
        "pictures",
    ];
    for table in tables {
        let query = format!(
            "SELECT CAST(id AS SIGNED) FROM `{table}` WHERE \
             CAST(created_at AS CHAR) LIKE '0000%' OR CAST(updated_at AS CHAR) LIKE '0000%' OR \
             CAST(deleted_at AS CHAR) LIKE '0000%' ORDER BY id LIMIT {LISTING_CAP}"
        );
        let ids: Vec<i64> = sqlx::query_scalar(&query)
            .fetch_all(mysql)
            .await
            .with_context(|| format!("preflight: zero date check failed for {table}"))?;
        if !ids.is_empty() {
            report.hard_errors.push(format_listing(
                &format!("{table}: zero date (0000-00-00) rows must be fixed manually"),
                &ids,
                ids.len() as i64,
            ));
        }
    }
    Ok(())
}

async fn check_boolean_columns(
    mysql: &MySqlPool,
    report: &mut PreflightReport,
) -> anyhow::Result<()> {
    let checks: [(&str, &str); 9] = [
        ("articles", "is_top"),
        ("articles", "is_recycled"),
        ("articles", "is_published"),
        ("articles", "is_allow_commented"),
        ("comments", "is_recycled"),
        ("comments", "is_checked"),
        ("journals", "is_secret"),
        ("navs", "open_type"),
        ("categories", "`type`"),
    ];
    for (table, column) in checks {
        // categories.type 合法值为 0/1/2，其余列合法值为 0/1。
        let allowed = if column == "`type`" {
            "0, 1, 2"
        } else {
            "0, 1"
        };
        let query = format!(
            "SELECT CAST(id AS SIGNED) FROM `{table}` WHERE {column} NOT IN ({allowed}) \
             ORDER BY id LIMIT {LISTING_CAP}"
        );
        let ids: Vec<i64> = sqlx::query_scalar(&query)
            .fetch_all(mysql)
            .await
            .with_context(|| format!("preflight: boolean check failed for {table}.{column}"))?;
        if !ids.is_empty() {
            report.hard_errors.push(format_listing(
                &format!("{table}.{column}: values outside {allowed}"),
                &ids,
                ids.len() as i64,
            ));
        }
    }
    Ok(())
}

async fn check_article_states(
    mysql: &MySqlPool,
    report: &mut PreflightReport,
) -> anyhow::Result<()> {
    let query = format!(
        "SELECT CAST(id AS SIGNED) FROM articles WHERE is_published = 1 AND is_recycled = 0 AND \
         (TRIM(url) = '' OR TRIM(title) = '' OR TRIM(content) = '') ORDER BY id LIMIT {LISTING_CAP}"
    );
    let ids: Vec<i64> = sqlx::query_scalar(&query)
        .fetch_all(mysql)
        .await
        .context("preflight: article state check failed")?;
    if !ids.is_empty() {
        report.hard_errors.push(format_listing(
            "articles: published articles with empty url/title/content (target CHECK rejects)",
            &ids,
            ids.len() as i64,
        ));
    }

    let negative: Vec<i64> = sqlx::query_scalar(&format!(
        "SELECT CAST(id AS SIGNED) FROM articles WHERE comment_count < 0 OR visit_count < 0 \
         ORDER BY id LIMIT {LISTING_CAP}"
    ))
    .fetch_all(mysql)
    .await
    .context("preflight: article counter check failed")?;
    if !negative.is_empty() {
        report.hard_errors.push(format_listing(
            "articles: negative comment_count/visit_count (target CHECK rejects)",
            &negative,
            negative.len() as i64,
        ));
    }

    let published: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM articles WHERE is_published = 1 AND is_recycled = 0",
    )
    .fetch_one(mysql)
    .await?;
    report
        .warnings
        .push(format!("articles: {published} published rows will use created_at as published_at (legacy schema has no publish time)"));
    Ok(())
}

async fn check_tag_article_duplicates(
    mysql: &MySqlPool,
    report: &mut PreflightReport,
) -> anyhow::Result<()> {
    let count: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(c - 1), 0) AS SIGNED) FROM (SELECT COUNT(*) AS c FROM tag_article \
         GROUP BY article_id, tag_id HAVING c > 1) dups",
    )
    .fetch_one(mysql)
    .await
    .context("preflight: tag_article duplicate check failed")?;
    if count > 0 {
        report.warnings.push(format!(
            "tag_article: {count} duplicate associations will be dropped"
        ));
    }
    Ok(())
}

async fn check_comment_types(
    mysql: &MySqlPool,
    report: &mut PreflightReport,
) -> anyhow::Result<()> {
    let invalid: Vec<i64> = sqlx::query_scalar(&format!(
        "SELECT CAST(id AS SIGNED) FROM comments WHERE `type` NOT IN (1, 2, 3, 4) \
         ORDER BY id LIMIT {LISTING_CAP}"
    ))
    .fetch_all(mysql)
    .await
    .context("preflight: comment type check failed")?;
    if !invalid.is_empty() {
        report.hard_errors.push(format_listing(
            "comments.type: values outside 1-4",
            &invalid,
            invalid.len() as i64,
        ));
    }
    let link_comments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE `type` = 2")
        .fetch_one(mysql)
        .await?;
    if link_comments > 0 {
        report.warnings.push(format!(
            "comments: {link_comments} link-page comments (type=2) will be stored as \
             target_type='link', target_id=0; the new frontend does not render them yet"
        ));
    }
    Ok(())
}

async fn check_length_violations(
    mysql: &MySqlPool,
    report: &mut PreflightReport,
) -> anyhow::Result<()> {
    // (label, sql)
    let checks: [(&str, &str); 9] = [
        (
            "comments.content empty or > 2000 chars",
            "SELECT CAST(id AS SIGNED) FROM comments WHERE CHAR_LENGTH(content) > 2000 \
             OR CHAR_LENGTH(TRIM(content)) = 0 ORDER BY id",
        ),
        (
            "comments.nick_name empty or > 60 chars",
            "SELECT CAST(id AS SIGNED) FROM comments WHERE CHAR_LENGTH(nick_name) > 60 \
             OR CHAR_LENGTH(TRIM(nick_name)) = 0 ORDER BY id",
        ),
        (
            "comments.email > 254 chars",
            "SELECT CAST(id AS SIGNED) FROM comments WHERE CHAR_LENGTH(email) > 254 ORDER BY id",
        ),
        (
            "comments.url > 2048 chars",
            "SELECT CAST(id AS SIGNED) FROM comments WHERE CHAR_LENGTH(url) > 2048 ORDER BY id",
        ),
        (
            "journals.content empty or > 2000 chars",
            "SELECT CAST(id AS SIGNED) FROM journals WHERE CHAR_LENGTH(content) > 2000 \
             OR CHAR_LENGTH(TRIM(content)) = 0 ORDER BY id",
        ),
        (
            "links.name empty or > 100 chars",
            "SELECT CAST(id AS SIGNED) FROM links WHERE CHAR_LENGTH(name) > 100 \
             OR CHAR_LENGTH(TRIM(name)) = 0 ORDER BY id",
        ),
        (
            "links.url empty or > 2048 chars",
            "SELECT CAST(id AS SIGNED) FROM links WHERE CHAR_LENGTH(url) > 2048 \
             OR CHAR_LENGTH(TRIM(url)) = 0 ORDER BY id",
        ),
        (
            "navs.name empty or > 60 chars",
            "SELECT CAST(id AS SIGNED) FROM navs WHERE CHAR_LENGTH(name) > 60 \
             OR CHAR_LENGTH(TRIM(name)) = 0 ORDER BY id",
        ),
        (
            "pages.title empty or > 200 chars",
            "SELECT CAST(id AS SIGNED) FROM pages WHERE CHAR_LENGTH(title) > 200 \
             OR CHAR_LENGTH(TRIM(title)) = 0 ORDER BY id",
        ),
    ];
    for (label, query) in checks {
        let ids: Vec<i64> = sqlx::query_scalar(query)
            .fetch_all(mysql)
            .await
            .with_context(|| format!("preflight: length check failed: {label}"))?;
        if !ids.is_empty() {
            report.violations.push(format_listing(
                &format!("{label} (hard fail unless --truncate-violations)"),
                &ids,
                ids.len() as i64,
            ));
        }
    }
    Ok(())
}

async fn check_setting_keys(mysql: &MySqlPool, report: &mut PreflightReport) -> anyhow::Result<()> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT s.name, i.`key` FROM sys_setting_items i \
         LEFT JOIN sys_settings s ON i.sys_id = s.id ORDER BY i.id",
    )
    .fetch_all(mysql)
    .await
    .context("preflight: setting key check failed")?;
    let mut unrecognized = Vec::new();
    for (group, key) in rows {
        let mut probe = crate::tables::settings::SettingsPlan::default();
        if crate::tables::settings::map_setting_item(0, &group, &key, "", &mut probe)
            != crate::tables::settings::MappingOutcome::Unrecognized
        {
            continue;
        }
        if is_secret_key(&key) {
            unrecognized.push(format!(
                "{group}.{key} (secret, value will not be recorded)"
            ));
        } else {
            unrecognized.push(format!("{group}.{key} (will be archived)"));
        }
    }
    if !unrecognized.is_empty() {
        report.warnings.push(format!(
            "unrecognized setting keys: {}",
            unrecognized.join(", ")
        ));
    }
    Ok(())
}

async fn check_media_urls(
    mysql: &MySqlPool,
    report: &mut PreflightReport,
    options: &PreflightOptions<'_>,
) -> anyhow::Result<()> {
    let Some(client) = options.http_client else {
        return Ok(());
    };
    let urls: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT url FROM pictures ORDER BY id LIMIT {}",
        options.media_precheck
    ))
    .fetch_all(mysql)
    .await
    .context("preflight: media url sampling failed")?;
    let mut failures = Vec::new();
    for url in urls {
        let Some(absolute) = crate::context::resolve_source_url(&url, options.source_base_url)
        else {
            failures.push(format!("{url} (relative url without --media-base-url)"));
            continue;
        };
        match client.head(&absolute).send().await {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => failures.push(format!("{absolute} (HTTP {})", response.status())),
            Err(error) => failures.push(format!("{absolute} ({error})")),
        }
    }
    for failure in failures {
        report
            .warnings
            .push(format!("media precheck unreachable: {failure}"));
    }
    Ok(())
}
