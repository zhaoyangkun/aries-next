//! 每表一个模块，统一 extract → transform（纯函数，便于单测）→ load。

pub mod article_tags;
pub mod articles;
pub mod categories;
pub mod comments;
pub mod galleries;
pub mod journals;
pub mod links;
pub mod media;
pub mod navigation;
pub mod pages;
pub mod settings;
pub mod tags;
pub mod users;

use anyhow::{Context, bail};
use sqlx::PgPool;
use time::{OffsetDateTime, UtcOffset};

use crate::{context::MigrateCtx, timeconv};

/// Transform 产生的 Archive 条目（副作用在 apply 阶段统一执行）。
#[derive(Debug)]
pub struct ArchiveEntry {
    pub id: Option<i64>,
    pub fields: serde_json::Value,
    pub reason: String,
    /// true 表示整行未迁入（参与对账）；false 表示仅字段级归档（行本身已迁移）。
    pub whole_row: bool,
}

/// 纯函数 Transform 的输出：目标行 + 需要归档的字段/行 + 修复计数。
#[derive(Debug)]
pub struct TransformOutcome<T> {
    /// None 表示整行无法迁入，应整体归档（archives 中含原因）。
    pub row: Option<T>,
    pub archives: Vec<ArchiveEntry>,
    pub repaired: i64,
    pub notes: Vec<String>,
}

impl<T> TransformOutcome<T> {
    pub fn ok(row: T) -> Self {
        Self {
            row: Some(row),
            archives: Vec::new(),
            repaired: 0,
            notes: Vec::new(),
        }
    }

    pub fn archive_row(
        id: Option<i64>,
        fields: serde_json::Value,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            row: None,
            archives: vec![ArchiveEntry {
                id,
                fields,
                reason: reason.into(),
                whole_row: true,
            }],
            repaired: 0,
            notes: Vec::new(),
        }
    }
}

/// Transform 公共参数。
#[derive(Debug, Clone, Copy)]
pub struct CommonParams {
    pub offset: UtcOffset,
    /// true 时超长字段截断（原文进 Archive）、空必填字段用占位值；false 时违例硬失败。
    pub truncate_violations: bool,
}

/// 把纯函数 Transform 的结果落到 Archive 与 Report，返回目标行。
pub fn apply_outcome<T>(
    ctx: &mut MigrateCtx,
    table: &str,
    outcome: TransformOutcome<T>,
) -> anyhow::Result<Option<T>> {
    for entry in outcome.archives {
        ctx.archive_row(
            table,
            entry.id,
            entry.fields,
            &entry.reason,
            entry.whole_row,
        )?;
    }
    let report = ctx.report.table(table);
    report.repaired += outcome.repaired;
    for note in outcome.notes {
        report.note(note);
    }
    Ok(outcome.row)
}

/// tinyint(1) → bool；Preflight 已拒绝 0/1 以外的值，这里兜底硬失败。
pub fn tinyint_to_bool(value: i64, table: &str, id: i64, column: &str) -> anyhow::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => bail!("{table} #{id}: column `{column}` has non-boolean tinyint value {other}"),
    }
}

/// 必填时间列：NULL / Zero Date / 越界均为硬错误（Preflight 应先拦截）。
pub fn required_time(
    raw: Option<&str>,
    offset: UtcOffset,
    table: &str,
    id: i64,
    column: &str,
) -> anyhow::Result<OffsetDateTime> {
    let raw = raw.ok_or_else(|| anyhow::anyhow!("{table} #{id}: column `{column}` is NULL"))?;
    timeconv::legacy_datetime_to_utc(raw, offset)
        .with_context(|| format!("{table} #{id}: column `{column}`"))
}

/// 可空时间列：NULL → None；非 NULL 但非法 → 硬错误（不静默置空）。
pub fn optional_time(
    raw: Option<&str>,
    offset: UtcOffset,
    table: &str,
    id: i64,
    column: &str,
) -> anyhow::Result<Option<OffsetDateTime>> {
    match raw {
        None => Ok(None),
        Some(raw) => Ok(Some(
            timeconv::legacy_datetime_to_utc(raw, offset)
                .with_context(|| format!("{table} #{id}: column `{column}`"))?,
        )),
    }
}

/// 长度约束：超限默认硬失败；`truncate_violations` 时截断并归档原文。
pub fn enforce_length(
    params: &CommonParams,
    table: &str,
    id: i64,
    field: &str,
    value: String,
    max_chars: usize,
) -> anyhow::Result<(String, Option<ArchiveEntry>, i64)> {
    let len = value.chars().count();
    if len <= max_chars {
        return Ok((value, None, 0));
    }
    if !params.truncate_violations {
        bail!(
            "{table} #{id}: field `{field}` is {len} chars, exceeds target limit {max_chars}; \
             fix the source row or rerun with --truncate-violations"
        );
    }
    let truncated: String = value.chars().take(max_chars).collect();
    let entry = ArchiveEntry {
        id: Some(id),
        fields: serde_json::json!({ field: value }),
        reason: format!("field `{field}` exceeded {max_chars} chars and was truncated"),
        whole_row: false,
    };
    Ok((truncated, Some(entry), 1))
}

/// 非空约束：空值默认硬失败；`truncate_violations` 时用占位值并归档原（空）值。
pub fn enforce_nonempty(
    params: &CommonParams,
    table: &str,
    id: i64,
    field: &str,
    value: String,
    placeholder: impl FnOnce() -> String,
) -> anyhow::Result<(String, Option<ArchiveEntry>, i64)> {
    if !value.trim().is_empty() {
        return Ok((value, None, 0));
    }
    if !params.truncate_violations {
        bail!(
            "{table} #{id}: field `{field}` is empty but the target requires a non-empty value; \
             fix the source row or rerun with --truncate-violations"
        );
    }
    let entry = ArchiveEntry {
        id: Some(id),
        fields: serde_json::json!({ field: value }),
        reason: format!("field `{field}` was empty and was replaced with a placeholder"),
        whole_row: false,
    };
    Ok((placeholder(), Some(entry), 1))
}

/// 无目标字段的非空值归档（空串不携带信息，直接跳过）。
pub fn archive_unmapped_field(
    archives: &mut Vec<ArchiveEntry>,
    id: i64,
    field: &str,
    value: &str,
    reason: &str,
) {
    if !value.is_empty() {
        archives.push(ArchiveEntry {
            id: Some(id),
            fields: serde_json::json!({ field: value }),
            reason: reason.to_owned(),
            whole_row: false,
        });
    }
}

/// 生成 slug：trim、小写、非 `[a-z0-9]` 连续段转 `-`、去首尾 `-`；空结果回退 `prefix-{id}`。
pub fn slugify(name: &str, prefix: &str, id: i64) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_dash = false;
    for ch in name.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        format!("{prefix}-{id}")
    } else {
        slug
    }
}

/// 解决 slug 冲突（citext 大小写不敏感）：冲突时追加 `-{id}`。
pub fn dedupe_slug(
    slug: String,
    id: i64,
    used: &std::collections::HashSet<String>,
) -> (String, bool) {
    if !used.contains(&slug.to_lowercase()) {
        return (slug, false);
    }
    (format!("{slug}-{id}"), true)
}

/// 读取源库某表的全部 id（供悬挂引用检测）。表名为代码内常量。
pub async fn fetch_id_set(
    mysql: &sqlx::MySqlPool,
    table: &str,
) -> anyhow::Result<std::collections::HashSet<i64>> {
    let query = format!("SELECT CAST(id AS SIGNED) FROM `{table}`");
    let ids: Vec<i64> = sqlx::query_scalar(&query)
        .fetch_all(mysql)
        .await
        .with_context(|| format!("failed to fetch ids from {table}"))?;
    Ok(ids.into_iter().collect())
}

/// 分批事务写入，逐行 `ON CONFLICT (id) DO NOTHING` 统计 migrated / skipped（重跑=Resume）。
pub async fn reset_sequence(pg: &PgPool, table: &str) -> anyhow::Result<()> {
    // 表名为代码内常量，不存在注入面。
    let query = format!(
        "SELECT setval(pg_get_serial_sequence('{table}', 'id'), \
         COALESCE(MAX(id), 1), MAX(id) IS NOT NULL) FROM {table}"
    );
    sqlx::query(&query)
        .execute(pg)
        .await
        .with_context(|| format!("failed to reset sequence for {table}"))?;
    Ok(())
}

/// 迁移时保留原 ID 的 IDENTITY 列表（Load 结束后统一重置 Sequence）。
pub const IDENTITY_TABLES: [&str; 12] = [
    "users",
    "categories",
    "tags",
    "articles",
    "media_assets",
    "pages",
    "journals",
    "comments",
    "galleries",
    "gallery_items",
    "links",
    "navigation_items",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn params(truncate: bool) -> CommonParams {
        CommonParams {
            offset: UtcOffset::UTC,
            truncate_violations: truncate,
        }
    }

    #[test]
    fn tinyint_accepts_only_zero_and_one() {
        assert!(!tinyint_to_bool(0, "t", 1, "c").unwrap());
        assert!(tinyint_to_bool(1, "t", 1, "c").unwrap());
        assert!(tinyint_to_bool(2, "t", 1, "c").is_err());
        assert!(tinyint_to_bool(-1, "t", 1, "c").is_err());
    }

    #[test]
    fn length_enforcement_keeps_within_limit() {
        let (value, archive, repaired) =
            enforce_length(&params(false), "t", 1, "f", "abc".to_owned(), 5).unwrap();
        assert_eq!(value, "abc");
        assert!(archive.is_none());
        assert_eq!(repaired, 0);
    }

    #[test]
    fn length_enforcement_fails_without_flag_and_truncates_with_it() {
        let long = "a".repeat(10);
        assert!(enforce_length(&params(false), "t", 1, "f", long.clone(), 5).is_err());
        let (value, archive, repaired) =
            enforce_length(&params(true), "t", 1, "f", long, 5).unwrap();
        assert_eq!(value, "aaaaa");
        assert!(archive.is_some());
        assert_eq!(repaired, 1);
    }

    #[test]
    fn length_counts_chars_not_bytes() {
        // 中文按字符计数，与 PostgreSQL char_length 一致。
        let (value, _, _) =
            enforce_length(&params(true), "t", 1, "f", "你好世界啊".to_owned(), 3).unwrap();
        assert_eq!(value, "你好世");
    }

    #[test]
    fn nonempty_enforcement_fails_or_uses_placeholder() {
        assert!(
            enforce_nonempty(&params(false), "t", 1, "f", "  ".to_owned(), || "x"
                .to_owned())
            .is_err()
        );
        let (value, archive, repaired) =
            enforce_nonempty(&params(true), "t", 1, "f", String::new(), || {
                "占位".to_owned()
            })
            .unwrap();
        assert_eq!(value, "占位");
        assert!(archive.is_some());
        assert_eq!(repaired, 1);
    }

    #[test]
    fn slugify_normalizes_and_falls_back() {
        assert_eq!(slugify("Hello World", "tag", 7), "hello-world");
        assert_eq!(slugify("  Rust 语言  ", "tag", 7), "rust");
        assert_eq!(slugify("a--b", "tag", 7), "a-b");
        assert_eq!(slugify("中文标签", "tag", 7), "tag-7");
        assert_eq!(slugify("!!!", "tag", 7), "tag-7");
        assert_eq!(slugify("-x-", "tag", 7), "x");
    }

    #[test]
    fn dedupe_slug_appends_id_on_case_insensitive_conflict() {
        let mut used = std::collections::HashSet::new();
        let (slug, repaired) = dedupe_slug("rust".to_owned(), 1, &used);
        assert!(!repaired);
        used.insert(slug.to_lowercase());
        let (slug2, repaired2) = dedupe_slug("Rust".to_owned(), 2, &used);
        assert!(repaired2);
        assert_eq!(slug2, "Rust-2");
    }

    #[test]
    fn archive_unmapped_field_skips_empty_values() {
        let mut archives = Vec::new();
        archive_unmapped_field(&mut archives, 1, "signature", "", "no target column");
        assert!(archives.is_empty());
        archive_unmapped_field(&mut archives, 1, "signature", "hello", "no target column");
        assert_eq!(archives.len(), 1);
    }
}
