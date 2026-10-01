//! `validate` 子命令：行数对比、主键 min/max、内容 Hash 抽查、关联计数、
//! Archive 对账（源行数 = migrated + skipped + archived + failed）、Sequence 核查。
//! 任何不一致非零退出并保留 Report。

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::Context;
use serde::Serialize;
use sqlx::{MySqlPool, PgPool};

use aries_core::content::MarkdownRenderer;
use aries_infra::ComrakMarkdownRenderer;

use crate::report::RunReport;

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub check: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Default, Serialize)]
pub struct ValidateReport {
    pub results: Vec<CheckResult>,
}

impl ValidateReport {
    fn record(&mut self, check: impl Into<String>, ok: bool, detail: impl Into<String>) {
        self.results.push(CheckResult {
            check: check.into(),
            ok,
            detail: detail.into(),
        });
    }

    pub fn has_failures(&self) -> bool {
        self.results.iter().any(|result| !result.ok)
    }
}

/// (源表, 目标表, 是否保留原 ID)
const COUNT_PAIRS: [(&str, &str, bool); 12] = [
    ("users", "users", true),
    ("categories", "categories", true),
    ("articles", "articles", true),
    ("tags", "tags", true),
    ("tag_article", "article_tags", false),
    ("pictures", "media_assets", true),
    ("comments", "comments", true),
    ("pages", "pages", true),
    ("journals", "journals", true),
    ("galleries", "gallery_items", true),
    ("links", "links", true),
    ("navs", "navigation_items", true),
];

pub async fn run(
    mysql: &MySqlPool,
    pg: &PgPool,
    renderer: &ComrakMarkdownRenderer,
    archive_path: Option<&Path>,
    report_path: Option<&Path>,
    sample_size: i64,
) -> anyhow::Result<ValidateReport> {
    let mut report = ValidateReport::default();

    let run_report = match report_path {
        Some(path) if path.exists() => Some(RunReport::load(path)?),
        _ => None,
    };
    let index = match archive_path {
        Some(path) if path.exists() => crate::archive::load_index(path)?,
        _ => crate::archive::ArchiveIndex {
            counts: BTreeMap::new(),
            whole_row_ids: BTreeMap::new(),
        },
    };
    let archive_counts = &index.counts;
    let archive_rows = &index.whole_row_ids;

    check_counts(mysql, pg, &mut report, archive_rows).await?;
    check_pk_ranges(mysql, pg, &mut report, archive_rows).await?;
    check_content_hashes(mysql, pg, renderer, &mut report, sample_size, archive_rows).await?;
    check_associations(mysql, pg, &mut report).await?;
    check_galleries(mysql, pg, &mut report).await?;
    check_sequences(pg, &mut report).await?;
    reconcile_report(&mut report, run_report.as_ref(), archive_counts);

    Ok(report)
}

async fn mysql_count(mysql: &MySqlPool, table: &str) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM `{table}`"))
            .fetch_one(mysql)
            .await?,
    )
}

async fn pg_count(pg: &PgPool, table: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(pg)
        .await?)
}

async fn check_counts(
    mysql: &MySqlPool,
    pg: &PgPool,
    report: &mut ValidateReport,
    archive_rows: &BTreeMap<String, HashSet<i64>>,
) -> anyhow::Result<()> {
    for (source, target, _) in COUNT_PAIRS {
        let source_count = mysql_count(mysql, source).await?;
        let target_count = pg_count(pg, target).await?;
        let archived_rows = archive_rows.get(target).map_or(0, HashSet::len) as i64;
        // 期望目标行数 = 源行数 - 整行归档数；media_assets 另含 gallery 媒体行。
        let expected = if source == "pictures" {
            source_count - archived_rows + pg_count(pg, "gallery_items").await?
        } else {
            source_count - archived_rows
        };
        // tag_article 的重复关联去重不进 Archive（由 check_associations 精确校验），
        // 这里允许 target 小于期望值。
        let ok = if source == "tag_article" {
            target_count <= expected
        } else {
            target_count == expected
        };
        report.record(
            format!("row count {source} → {target}"),
            ok,
            format!("source={source_count} target={target_count} expected={expected} archived_rows={archived_rows}"),
        );
    }
    Ok(())
}

async fn check_pk_ranges(
    mysql: &MySqlPool,
    pg: &PgPool,
    report: &mut ValidateReport,
    archive_rows: &BTreeMap<String, HashSet<i64>>,
) -> anyhow::Result<()> {
    for (source, target, preserves_id) in COUNT_PAIRS {
        if !preserves_id {
            continue;
        }
        // 源侧排除整行归档的 id 后求 min/max，与目标精确对比。
        let excluded = archive_rows.get(target);
        let source_ids: Vec<i64> = sqlx::query_scalar(&format!(
            "SELECT CAST(id AS SIGNED) FROM `{source}` ORDER BY id"
        ))
        .fetch_all(mysql)
        .await?;
        let kept: Vec<i64> = source_ids
            .iter()
            .copied()
            .filter(|id| !excluded.is_some_and(|set| set.contains(id)))
            .collect();
        let (target_min, target_max): (Option<i64>, Option<i64>) =
            sqlx::query_as(&format!("SELECT MIN(id), MAX(id) FROM {target}"))
                .fetch_one(pg)
                .await?;
        let (Some(&source_min), Some(&source_max), Some(target_min), Some(target_max)) =
            (kept.first(), kept.last(), target_min, target_max)
        else {
            report.record(
                format!("pk range {source}"),
                true,
                "empty on one or both sides; skipped".to_owned(),
            );
            continue;
        };
        // pictures 只占 media_assets 的一部分（gallery 媒体行 id 更大），min 必须一致、max 不弱。
        let (min_ok, max_ok) = if source == "pictures" {
            (source_min == target_min, source_max <= target_max)
        } else {
            (source_min == target_min, source_max == target_max)
        };
        report.record(
            format!("pk range {source} → {target}"),
            min_ok && max_ok,
            format!("source=[{source_min},{source_max}] target=[{target_min},{target_max}]"),
        );
    }
    Ok(())
}

async fn check_content_hashes(
    mysql: &MySqlPool,
    pg: &PgPool,
    renderer: &ComrakMarkdownRenderer,
    report: &mut ValidateReport,
    sample_size: i64,
    archive_rows: &BTreeMap<String, HashSet<i64>>,
) -> anyhow::Result<()> {
    // (源表, 源 Markdown 列, 目标表, 目标 Markdown 列, 目标 HTML 列)
    let checks: [(&str, &str, &str, &str, &str); 4] = [
        (
            "articles",
            "content",
            "articles",
            "markdown_source",
            "rendered_html",
        ),
        (
            "comments",
            "content",
            "comments",
            "content_markdown",
            "content_html",
        ),
        ("pages", "html", "pages", "content_markdown", "content_html"),
        (
            "journals",
            "content",
            "journals",
            "content_markdown",
            "content_html",
        ),
    ];
    for (source_table, source_column, target_table, target_md, target_html) in checks {
        let source_rows: Vec<(i64, String)> = sqlx::query_as(&format!(
            "SELECT CAST(id AS SIGNED), `{source_column}` FROM `{source_table}` \
             ORDER BY id LIMIT {sample_size}"
        ))
        .fetch_all(mysql)
        .await
        .with_context(|| format!("validate: failed to sample {source_table}"))?;

        let archived_ids = archive_rows.get(target_table);
        let mut mismatches = Vec::new();
        for (id, source_markdown) in source_rows {
            if archived_ids.is_some_and(|set| set.contains(&id)) {
                continue; // 整行归档，目标本就不应有此行
            }
            let target: Option<(String, String)> = sqlx::query_as(&format!(
                "SELECT {target_md}, {target_html} FROM {target_table} WHERE id = $1"
            ))
            .bind(id)
            .fetch_optional(pg)
            .await?;
            let Some((target_markdown, target_html)) = target else {
                mismatches.push(format!("#{id} missing in target"));
                continue;
            };
            // 截断迁移的行不参与严格比对（源与目标必然不等，Report 已记录）。
            if target_markdown != source_markdown {
                mismatches.push(format!("#{id} markdown differs (possibly truncated)"));
                continue;
            }
            let expected_html = renderer.render(&target_markdown)?;
            if expected_html != target_html {
                mismatches.push(format!("#{id} rendered html diverges from fresh render"));
            }
        }
        report.record(
            format!("content hash sample {source_table} → {target_table}"),
            mismatches.is_empty(),
            if mismatches.is_empty() {
                "all sampled rows match".to_owned()
            } else {
                mismatches.join("; ")
            },
        );
    }
    Ok(())
}

async fn check_associations(
    mysql: &MySqlPool,
    pg: &PgPool,
    report: &mut ValidateReport,
) -> anyhow::Result<()> {
    let source_pairs: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT CAST(article_id AS SIGNED), CAST(tag_id AS SIGNED) FROM tag_article",
    )
    .fetch_all(mysql)
    .await?;
    let article_ids = valid_ids(mysql, "articles").await?;
    let tag_ids = valid_ids(mysql, "tags").await?;
    let expected: HashSet<(i64, i64)> = source_pairs
        .into_iter()
        .filter(|(article_id, tag_id)| article_ids.contains(article_id) && tag_ids.contains(tag_id))
        .collect();
    let target_count = pg_count(pg, "article_tags").await?;
    report.record(
        "article_tags association count",
        target_count == expected.len() as i64,
        format!(
            "expected {} deduped valid pairs, target {target_count}",
            expected.len()
        ),
    );
    Ok(())
}

async fn valid_ids(mysql: &MySqlPool, table: &str) -> anyhow::Result<HashSet<i64>> {
    let ids: Vec<i64> = sqlx::query_scalar(&format!("SELECT CAST(id AS SIGNED) FROM `{table}`"))
        .fetch_all(mysql)
        .await?;
    Ok(ids.into_iter().collect())
}

async fn check_galleries(
    mysql: &MySqlPool,
    pg: &PgPool,
    report: &mut ValidateReport,
) -> anyhow::Result<()> {
    let gallery_categories: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM categories WHERE `type` = 2")
            .fetch_one(mysql)
            .await?;
    let target_galleries = pg_count(pg, "galleries").await?;
    report.record(
        "galleries aggregated from gallery categories",
        target_galleries == gallery_categories,
        format!(
            "source gallery categories={gallery_categories} target galleries={target_galleries}"
        ),
    );
    Ok(())
}

async fn check_sequences(pg: &PgPool, report: &mut ValidateReport) -> anyhow::Result<()> {
    for table in crate::tables::IDENTITY_TABLES {
        let sequence: Option<String> =
            sqlx::query_scalar(&format!("SELECT pg_get_serial_sequence('{table}', 'id')"))
                .fetch_one(pg)
                .await?;
        let Some(sequence) = sequence else {
            report.record(
                format!("sequence {table}"),
                false,
                "no identity sequence found".to_owned(),
            );
            continue;
        };
        // sequence 名由数据库返回，非用户输入。
        let last_value: i64 = sqlx::query_scalar(&format!("SELECT last_value FROM {sequence}"))
            .fetch_one(pg)
            .await?;
        let max_id: Option<i64> = sqlx::query_scalar(&format!("SELECT MAX(id) FROM {table}"))
            .fetch_one(pg)
            .await?;
        let ok = max_id.is_none_or(|max_id| last_value >= max_id);
        report.record(
            format!("sequence {table}"),
            ok,
            format!("last_value={last_value} max_id={max_id:?}"),
        );
    }
    Ok(())
}

/// 对账：RunReport 内部一致性（源行数 = migrated + skipped + archived + failed），
/// 以及 Report 的 archived 计数与 Archive 文件实际行数一致。
fn reconcile_report(
    report: &mut ValidateReport,
    run_report: Option<&RunReport>,
    archive_counts: &BTreeMap<String, u64>,
) {
    for (table, count) in archive_counts {
        let detail = format!("archive file has {count} lines for {table}");
        let ok = run_report.is_none_or(|run| {
            run.tables
                .get(table)
                .is_none_or(|table_report| table_report.archived == *count as i64)
        });
        report.record(format!("archive reconciliation {table}"), ok, detail);
    }
    if let Some(run) = run_report {
        for (table, table_report) in &run.tables {
            let accounted = table_report.migrated
                + table_report.skipped
                + table_report.archived_rows
                + table_report.failed;
            // media_assets 有两个来源（pictures + galleries 行），source_rows 只记 pictures，
            // 不参与恒等校验；galleries 媒体行的计数并入其中。
            if table == "media_assets" {
                continue;
            }
            report.record(
                format!("report consistency {table}"),
                accounted == table_report.source_rows,
                format!(
                    "source_rows={} accounted={accounted} \
                     (migrated={} skipped={} archived_rows={} failed={})",
                    table_report.source_rows,
                    table_report.migrated,
                    table_report.skipped,
                    table_report.archived_rows,
                    table_report.failed
                ),
            );
        }
    }
}
