//! aries-migrator：MySQL → PostgreSQL 全量 ETL。
//! 子命令：preflight（只读检查）/ migrate（迁移）/ validate（校验）。
//! 数据完整性三去向：迁入目标表 / Archive JSONL / Report 计数；禁止静默丢弃。

mod archive;
mod context;
mod preflight;
mod report;
mod tables;
mod timeconv;
mod validate;

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, bail};
use clap::{Args, Parser, Subcommand};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{MySqlPool, PgPool};

use aries_infra::ComrakMarkdownRenderer;
use aries_infra::storage::{DEFAULT_LOCAL_DIR, DEFAULT_PUBLIC_BASE_URL};
use context::{DOWNLOAD_TIMEOUT_SECS, MediaOptions, MigrateCtx};
use report::RunReport;

#[derive(Debug, Parser)]
#[command(
    name = "aries-migrator",
    about = "Migrate legacy Aries (MySQL) data into Aries Next (PostgreSQL)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 只读检查：citext 冲突、孤儿引用、Zero Date、布尔越界、非法状态、长度违例等。
    Preflight(PreflightArgs),
    /// 全量迁移：Preflight 有硬错误时拒绝运行。
    Migrate(MigrateArgs),
    /// 校验：行数、主键范围、内容 Hash 抽查、关联计数、Archive 对账、Sequence。
    Validate(ValidateArgs),
}

#[derive(Debug, Args)]
struct CommonArgs {
    #[arg(long, env = "MIGRATION_MYSQL_URL")]
    mysql_url: String,
    #[arg(long, env = "MIGRATION_POSTGRES_URL")]
    postgres_url: String,
    /// 目标 Schema（与 server 的 DATABASE_SCHEMA 一致），用于设置 search_path。
    #[arg(long, env = "DATABASE_SCHEMA", default_value = "public")]
    pg_schema: String,
    /// 旧站 datetime 列的时区偏移（旧站配置时区）。
    #[arg(long, default_value = "+08:00")]
    source_offset: String,
    /// Report JSON 落盘路径；缺省只打印到 stdout。
    #[arg(long)]
    report: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct PreflightArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// 媒体 URL HEAD 抽查条数（0 = 关闭，需配合 --media-base-url 解析相对 URL）。
    #[arg(long, default_value_t = 0)]
    media_precheck: usize,
    /// 旧站地址，用于把相对媒体 URL 拼接为绝对 URL。
    #[arg(long)]
    media_base_url: Option<String>,
}

#[derive(Debug, Args)]
struct MigrateArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// 单事务批量插入大小。
    #[arg(long, default_value_t = 500)]
    batch_size: usize,
    /// 只迁移指定表（逗号分隔）：users,categories,tags,articles,article_tags,media,
    /// pages,journals,comments,galleries,links,navigation,settings
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,
    /// 长度/空值违例改为截断或占位（原文进 Archive）；缺省遇违例即失败退出。
    #[arg(long)]
    truncate_violations: bool,
    /// 跳过媒体下载，全部记录为 legacy_url 外链。
    #[arg(long)]
    skip_media_download: bool,
    /// 下载成功后的写盘目录（对应 server 的 MEDIA_LOCAL_DIR）。
    #[arg(long, env = "MEDIA_LOCAL_DIR", default_value = DEFAULT_LOCAL_DIR)]
    media_dir: PathBuf,
    /// 旧站地址，用于把相对媒体 URL 拼接为绝对 URL 后下载。
    #[arg(long)]
    media_base_url: Option<String>,
    /// 目标站媒体公开前缀（对应 server 的 MEDIA_PUBLIC_BASE_URL）。
    #[arg(long, env = "MEDIA_PUBLIC_BASE_URL", default_value = DEFAULT_PUBLIC_BASE_URL)]
    media_public_base_url: String,
    /// Archive JSONL 路径，默认 migration-archive-<run_id>.jsonl。
    #[arg(long)]
    archive: Option<PathBuf>,
    /// 显式指定 owner 账号 id（逗号分隔）；缺省取最小 id 账号，其余为 editor。
    #[arg(long, value_delimiter = ',')]
    owner_ids: Vec<i64>,
}

#[derive(Debug, Args)]
struct ValidateArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// migrate 产生的 Archive JSONL，用于对账。
    #[arg(long)]
    archive: Option<PathBuf>,
    /// migrate 产生的 Report JSON，用于严格对账（缺省则只做库级校验）。
    #[arg(long)]
    migrate_report: Option<PathBuf>,
    /// 内容 Hash 抽查样本量（每表）。
    #[arg(long, default_value_t = 100)]
    sample_size: i64,
}

/// 按 FK 依赖序排列的全部迁移步骤。
const ALL_STEPS: [&str; 13] = [
    "users",
    "categories",
    "tags",
    "articles",
    "article_tags",
    "media",
    "pages",
    "journals",
    "comments",
    "galleries",
    "links",
    "navigation",
    "settings",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let exit_code = match cli.command {
        Command::Preflight(args) => run_preflight(args).await?,
        Command::Migrate(args) => run_migrate(args).await?,
        Command::Validate(args) => run_validate(args).await?,
    };
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

async fn connect_mysql(url: &str) -> anyhow::Result<MySqlPool> {
    MySqlPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .context("failed to connect to source MySQL")
}

async fn connect_postgres(url: &str, schema: &str) -> anyhow::Result<PgPool> {
    validate_schema(schema)?;
    let options = PgConnectOptions::from_str(url)
        .context("MIGRATION_POSTGRES_URL is not a valid PostgreSQL URL")?
        // 与 server 一致设置 search_path（见 backend/infra/src/lib.rs）。
        .options([("search_path", schema)]);
    PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .context("failed to connect to target PostgreSQL")
}

fn validate_schema(schema: &str) -> anyhow::Result<()> {
    let mut chars = schema.chars();
    let valid_start = chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic());
    let valid_rest = chars.all(|c| c == '_' || c.is_ascii_alphanumeric());
    if !valid_start || !valid_rest {
        bail!("--pg-schema must be a simple PostgreSQL identifier");
    }
    Ok(())
}

fn parse_offset(raw: &str) -> anyhow::Result<time::UtcOffset> {
    timeconv::parse_source_offset(raw).map_err(anyhow::Error::from)
}

fn print_preflight(report: &preflight::PreflightReport) {
    println!(
        "{}",
        serde_json::to_string_pretty(report).expect("preflight report serializes")
    );
}

async fn run_preflight(args: PreflightArgs) -> anyhow::Result<i32> {
    let (mut report, started) = RunReport::start("preflight");
    let mysql = connect_mysql(&args.common.mysql_url).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build()?;
    let preflight = preflight::run(
        &mysql,
        &preflight::PreflightOptions {
            media_precheck: args.media_precheck,
            source_base_url: args.media_base_url.as_deref(),
            http_client: Some(&client),
        },
    )
    .await?;
    print_preflight(&preflight);
    report.note(format!(
        "preflight: {} hard errors, {} violations, {} warnings",
        preflight.hard_errors.len(),
        preflight.violations.len(),
        preflight.warnings.len()
    ));
    report.tables.insert(
        "preflight".to_owned(),
        report::TableReport {
            notes: preflight.warnings.clone(),
            ..Default::default()
        },
    );
    report.finish(started);
    save_and_print(&report, args.common.report.as_deref())?;
    Ok(if preflight.is_clean() { 0 } else { 1 })
}

async fn run_migrate(args: MigrateArgs) -> anyhow::Result<i32> {
    let offset = parse_offset(&args.common.source_offset)?;
    let mysql = connect_mysql(&args.common.mysql_url).await?;
    let pg = connect_postgres(&args.common.postgres_url, &args.common.pg_schema).await?;

    // 未知 --only 名称直接报错。
    for step in &args.only {
        if !ALL_STEPS.contains(&step.as_str()) {
            bail!("unknown --only step `{step}`; valid steps: {ALL_STEPS:?}");
        }
    }
    let should_run = |step: &str| args.only.is_empty() || args.only.iter().any(|s| s == step);

    // Preflight 门禁：硬错误永远阻止迁移；长度违例需 --truncate-violations 确认。
    let preflight = preflight::run(
        &mysql,
        &preflight::PreflightOptions {
            media_precheck: 0,
            source_base_url: args.media_base_url.as_deref(),
            http_client: None,
        },
    )
    .await?;
    if !preflight.hard_errors.is_empty() {
        print_preflight(&preflight);
        bail!(
            "preflight found {} hard error(s); fix the source data before migrating",
            preflight.hard_errors.len()
        );
    }
    if !preflight.violations.is_empty() && !args.truncate_violations {
        print_preflight(&preflight);
        bail!(
            "preflight found {} length/emptiness violation(s); fix the source data or rerun with --truncate-violations",
            preflight.violations.len()
        );
    }

    let (mut run_report, started) = RunReport::start("migrate");
    run_report.notes = preflight
        .warnings
        .iter()
        .map(|warning| format!("preflight warning: {warning}"))
        .collect();

    let archive_path = args
        .archive
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("migration-archive-{}.jsonl", run_report.run_id)));
    let archive = archive::ArchiveWriter::create(&archive_path)
        .with_context(|| format!("failed to create archive file {}", archive_path.display()))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build()?;

    let mut ctx = MigrateCtx {
        mysql,
        pg,
        renderer: ComrakMarkdownRenderer,
        offset,
        truncate_violations: args.truncate_violations,
        batch_size: args.batch_size,
        archive,
        report: run_report,
        owner_ids: args.owner_ids.clone(),
        owner_id: None,
        media: MediaOptions {
            skip_download: args.skip_media_download,
            media_dir: args.media_dir.clone(),
            source_base_url: args.media_base_url.clone(),
            public_base_url: args.media_public_base_url.clone(),
            client,
        },
    };

    // 按 FK 依赖序执行。
    if should_run("users") {
        tables::users::migrate(&mut ctx).await?;
    }
    if should_run("categories") {
        tables::categories::migrate(&mut ctx).await?;
    }
    if should_run("tags") {
        tables::tags::migrate(&mut ctx).await?;
    }
    if should_run("articles") {
        tables::articles::migrate(&mut ctx).await?;
    }
    if should_run("article_tags") {
        tables::article_tags::migrate(&mut ctx).await?;
    }
    if should_run("media") {
        tables::media::migrate(&mut ctx).await?;
    }
    if should_run("pages") {
        tables::pages::migrate(&mut ctx).await?;
    }
    if should_run("journals") {
        tables::journals::migrate(&mut ctx).await?;
    }
    if should_run("comments") {
        tables::comments::migrate(&mut ctx).await?;
    }
    if should_run("galleries") {
        tables::galleries::migrate(&mut ctx).await?;
    }
    if should_run("links") {
        tables::links::migrate(&mut ctx).await?;
    }
    if should_run("navigation") {
        tables::navigation::migrate(&mut ctx).await?;
    }
    if should_run("settings") {
        tables::settings::migrate(&mut ctx).await?;
    }

    // 重置保留原 ID 的 IDENTITY 列 Sequence（只处理本次执行过的步骤）。
    let mut reset_tables: Vec<&'static str> = Vec::new();
    if should_run("users") {
        reset_tables.push("users");
    }
    if should_run("categories") {
        reset_tables.push("categories");
    }
    if should_run("tags") {
        reset_tables.push("tags");
    }
    if should_run("articles") {
        reset_tables.push("articles");
    }
    if should_run("media") || should_run("galleries") {
        reset_tables.push("media_assets");
    }
    if should_run("pages") {
        reset_tables.push("pages");
    }
    if should_run("journals") {
        reset_tables.push("journals");
    }
    if should_run("comments") {
        reset_tables.push("comments");
    }
    if should_run("galleries") {
        reset_tables.extend(["galleries", "gallery_items"]);
    }
    if should_run("links") {
        reset_tables.push("links");
    }
    if should_run("navigation") {
        reset_tables.push("navigation_items");
    }
    for table in &reset_tables {
        tables::reset_sequence(&ctx.pg, table).await?;
    }
    if !reset_tables.is_empty() {
        ctx.report
            .notes
            .push(format!("sequences reset: {}", reset_tables.join(", ")));
    }

    let mut ctx = ctx;
    ctx.archive.finish()?;
    let mut run_report = std::mem::replace(&mut ctx.report, RunReport::start("migrate").0);
    run_report.finish(started);
    run_report.note(format!("archive file: {}", archive_path.display()));
    save_and_print(&run_report, args.common.report.as_deref())?;
    Ok(0)
}

async fn run_validate(args: ValidateArgs) -> anyhow::Result<i32> {
    let mysql = connect_mysql(&args.common.mysql_url).await?;
    let pg = connect_postgres(&args.common.postgres_url, &args.common.pg_schema).await?;
    let (mut run_report, started) = RunReport::start("validate");

    let report = validate::run(
        &mysql,
        &pg,
        &ComrakMarkdownRenderer,
        args.archive.as_deref(),
        args.migrate_report.as_deref(),
        args.sample_size,
    )
    .await?;

    let failures = report.results.iter().filter(|result| !result.ok).count();
    let has_failures = report.has_failures();
    println!("{}", serde_json::to_string_pretty(&report)?);
    run_report.note(format!(
        "validate: {} checks, {} failed",
        report.results.len(),
        failures
    ));
    run_report.finish(started);
    save_and_print(&run_report, args.common.report.as_deref())?;
    Ok(if has_failures { 1 } else { 0 })
}

fn save_and_print(report: &RunReport, path: Option<&std::path::Path>) -> anyhow::Result<()> {
    if let Some(path) = path {
        report.save(path)?;
        eprintln!("report saved to {}", path.display());
    }
    println!("{}", serde_json::to_string_pretty(report)?);
    Ok(())
}
