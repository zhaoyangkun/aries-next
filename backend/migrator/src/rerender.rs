//! re-render 子命令：渲染器（ComrakMarkdownRenderer）升级后，存量行的派生 HTML 列
//! 仍是旧产物。本命令直连 PostgreSQL，把各表的 Markdown 源列用当前渲染器重算并回写。
//! 幂等、只写派生列、跳过软删除行；渲染结果与存量一致时不产生 UPDATE。

use anyhow::{Context, bail};
use sqlx::PgPool;

use aries_core::content::MarkdownRenderer;
use aries_infra::ComrakMarkdownRenderer;

/// 一张表的 (Markdown 源列 → 渲染 HTML 列) 映射。
/// 表名与列名均为编译期常量，不参与任何外部输入拼接。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderTarget {
    name: &'static str,
    table: &'static str,
    markdown_column: &'static str,
    html_column: &'static str,
}

/// 全部含派生 HTML 列的表（与 migrations/ 下的 Schema 一一对应）。
const TARGETS: [RenderTarget; 4] = [
    RenderTarget {
        name: "articles",
        table: "articles",
        markdown_column: "markdown_source",
        html_column: "rendered_html",
    },
    RenderTarget {
        name: "pages",
        table: "pages",
        markdown_column: "content_markdown",
        html_column: "content_html",
    },
    RenderTarget {
        name: "journals",
        table: "journals",
        markdown_column: "content_markdown",
        html_column: "content_html",
    },
    RenderTarget {
        name: "comments",
        table: "comments",
        markdown_column: "content_markdown",
        html_column: "content_html",
    },
];

fn select_targets(only: &[String]) -> anyhow::Result<Vec<RenderTarget>> {
    for name in only {
        if !TARGETS.iter().any(|target| target.name == name) {
            let valid: Vec<&str> = TARGETS.iter().map(|target| target.name).collect();
            bail!("unknown --only table `{name}`; valid tables: {valid:?}");
        }
    }
    Ok(TARGETS
        .into_iter()
        .filter(|target| only.is_empty() || only.iter().any(|name| name == target.name))
        .collect())
}

pub async fn run(pg: &PgPool, only: &[String], dry_run: bool) -> anyhow::Result<()> {
    let targets = select_targets(only)?;
    let renderer = ComrakMarkdownRenderer;
    for target in targets {
        re_render_table(pg, &renderer, target, dry_run).await?;
    }
    Ok(())
}

async fn re_render_table(
    pg: &PgPool,
    renderer: &ComrakMarkdownRenderer,
    target: RenderTarget,
    dry_run: bool,
) -> anyhow::Result<()> {
    let select = format!(
        "SELECT id, {}, {} FROM {} WHERE deleted_at IS NULL ORDER BY id",
        target.markdown_column, target.html_column, target.table
    );
    let rows: Vec<(i64, String, String)> = sqlx::query_as(&select)
        .fetch_all(pg)
        .await
        .with_context(|| format!("failed to read {}", target.table))?;

    let update = format!(
        "UPDATE {} SET {} = $1 WHERE id = $2",
        target.table, target.html_column
    );
    let mut updated = 0usize;
    for (id, markdown, existing_html) in &rows {
        let rendered = renderer
            .render(markdown)
            .with_context(|| format!("failed to render {} id={id}", target.table))?;
        if rendered != *existing_html {
            updated += 1;
            if !dry_run {
                sqlx::query(&update)
                    .bind(&rendered)
                    .bind(id)
                    .execute(pg)
                    .await
                    .with_context(|| format!("failed to update {} id={id}", target.table))?;
            }
        }
    }

    let mode = if dry_run { "would update" } else { "updated" };
    println!(
        "{}: {} {} of {} rows",
        target.table,
        mode,
        updated,
        rows.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_only_selects_all_targets() {
        let selected = select_targets(&[]).expect("empty --only is valid");
        assert_eq!(selected.len(), TARGETS.len());
    }

    #[test]
    fn only_filters_targets() {
        let selected = select_targets(&["articles".to_owned()]).expect("known table");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "articles");
    }

    #[test]
    fn unknown_only_name_is_rejected() {
        let err = select_targets(&["settings".to_owned()]).expect_err("unknown table");
        assert!(err.to_string().contains("settings"));
    }

    #[test]
    fn targets_match_schema_columns() {
        // 派生 HTML 列清单与 migrations/ 下定义保持一致（articles 独有 rendered_html 命名）
        let names: Vec<&str> = TARGETS.iter().map(|target| target.name).collect();
        assert_eq!(names, ["articles", "pages", "journals", "comments"]);
    }
}
