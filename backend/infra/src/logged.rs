//! SQL 查询参数日志包装器。
//!
//! 包装 `sqlx::query()` / `sqlx::query_as()` / `sqlx::query_scalar()`，
//! 在 `.bind()` 时捕获参数的 Debug 表示，执行时通过 tracing Span
//! 传递给 `DbLogLayer`，最终写入 `db.parameters` 字段。

use std::fmt::Debug;

use sqlx::postgres::{PgArguments, PgQueryResult, PgRow, Postgres};
use sqlx::query::{Query, QueryAs, QueryScalar};
use sqlx::{Error, Executor};
use tracing::Instrument;

/// 参数 Span 名。server 端 `DbLogLayer` 据此识别该 Span：读取 `db.parameters`，
/// 但取 `span_name` 时跳过它（否则 SQL 日志的来源会显示成 sql_params）。
pub const SQL_PARAMS_SPAN: &str = "sql_params";

/// 敏感语句参数的统一掩码值。语句文本（含列名）命中任一关键字时，
/// 该语句的全部参数都以掩码写入日志，避免 password_hash、token 等值泄露。
pub const REDACTED: &str = "***REDACTED***";

/// 命中即视为敏感语句的 SQL 关键字（大小写不敏感的子串匹配）。
const SENSITIVE_KEYWORDS: [&str; 4] = ["password_hash", "password", "token", "secret"];

fn is_sensitive_sql(sql: &str) -> bool {
    let lower = sql.to_lowercase();
    SENSITIVE_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(keyword))
}

/// 把参数格式化为 `$1 = v1, $2 = v2` 形式；敏感语句的参数一律掩码。
fn format_params(sql: &str, params: &[String]) -> String {
    let sensitive = is_sensitive_sql(sql);
    params
        .iter()
        .enumerate()
        .map(|(i, value)| {
            if sensitive {
                format!("${} = {}", i + 1, REDACTED)
            } else {
                format!("${} = {}", i + 1, value)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 创建携带参数信息的 Span。无参数时返回 no-op Span。
/// target 固定为 `sqlx::query`：Span 是否启用跟随 SQL 日志开关（`sqlx::query=debug` 指令）。
/// 若用默认 target（本模块路径 `aries_infra::logged`），默认 info 级别下 Span 被禁用，
/// 禁用的 Span 不进作用域，`db.parameters` 永远不会被 `DbLogLayer` 看到。
fn params_span(sql: &str, params: &[String]) -> tracing::Span {
    if params.is_empty() {
        return tracing::Span::none();
    }
    let formatted = format_params(sql, params);
    tracing::debug_span!(target: "sqlx::query", SQL_PARAMS_SPAN, db.parameters = %formatted)
}

// ---------------------------------------------------------------------------
// LoggedQuery: wraps sqlx::query()
// ---------------------------------------------------------------------------

/// 包装 `sqlx::query()`，在 `.bind()` 时捕获参数的 Debug 表示。
pub fn logged_query<'q>(sql: &'q str) -> LoggedQuery<'q> {
    LoggedQuery {
        inner: sqlx::query(sql),
        sql,
        params: Vec::new(),
    }
}

pub struct LoggedQuery<'q> {
    inner: Query<'q, Postgres, PgArguments>,
    sql: &'q str,
    params: Vec<String>,
}

impl<'q> LoggedQuery<'q> {
    pub fn bind<T>(mut self, value: T) -> Self
    where
        T: 'q + Debug + sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres>,
    {
        self.params.push(format!("{:?}", value));
        self.inner = self.inner.bind(value);
        self
    }

    pub async fn execute<'e, 'c: 'e, E>(self, executor: E) -> Result<PgQueryResult, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.execute(executor).instrument(span).await
    }

    pub async fn fetch_one<'e, 'c: 'e, E>(self, executor: E) -> Result<PgRow, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_one(executor).instrument(span).await
    }

    pub async fn fetch_all<'e, 'c: 'e, E>(self, executor: E) -> Result<Vec<PgRow>, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_all(executor).instrument(span).await
    }

    pub async fn fetch_optional<'e, 'c: 'e, E>(self, executor: E) -> Result<Option<PgRow>, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_optional(executor).instrument(span).await
    }
}

// ---------------------------------------------------------------------------
// LoggedQueryAs: wraps sqlx::query_as()
// ---------------------------------------------------------------------------

/// 包装 `sqlx::query_as()`，在 `.bind()` 时捕获参数的 Debug 表示。
pub fn logged_query_as<'q, O>(sql: &'q str) -> LoggedQueryAs<'q, O>
where
    O: for<'r> sqlx::FromRow<'r, PgRow> + Send + Unpin,
{
    LoggedQueryAs {
        inner: sqlx::query_as::<_, O>(sql),
        sql,
        params: Vec::new(),
    }
}

pub struct LoggedQueryAs<'q, O> {
    inner: QueryAs<'q, Postgres, O, PgArguments>,
    sql: &'q str,
    params: Vec<String>,
}

impl<'q, O> LoggedQueryAs<'q, O>
where
    O: for<'r> sqlx::FromRow<'r, PgRow> + Send + Unpin,
{
    pub fn bind<T>(mut self, value: T) -> Self
    where
        T: 'q + Debug + sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres>,
    {
        self.params.push(format!("{:?}", value));
        self.inner = self.inner.bind(value);
        self
    }

    pub async fn fetch_one<'e, 'c: 'e, E>(self, executor: E) -> Result<O, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_one(executor).instrument(span).await
    }

    pub async fn fetch_all<'e, 'c: 'e, E>(self, executor: E) -> Result<Vec<O>, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_all(executor).instrument(span).await
    }

    pub async fn fetch_optional<'e, 'c: 'e, E>(self, executor: E) -> Result<Option<O>, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_optional(executor).instrument(span).await
    }
}

// ---------------------------------------------------------------------------
// LoggedQueryScalar: wraps sqlx::query_scalar()
// ---------------------------------------------------------------------------

/// 包装 `sqlx::query_scalar()`，在 `.bind()` 时捕获参数的 Debug 表示。
pub fn logged_query_scalar<'q, O>(sql: &'q str) -> LoggedQueryScalar<'q, O>
where
    O: for<'r> sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres> + Send + Unpin,
{
    LoggedQueryScalar {
        inner: sqlx::query_scalar::<Postgres, O>(sql),
        sql,
        params: Vec::new(),
    }
}

pub struct LoggedQueryScalar<'q, O> {
    inner: QueryScalar<'q, Postgres, O, PgArguments>,
    sql: &'q str,
    params: Vec<String>,
}

impl<'q, O> LoggedQueryScalar<'q, O>
where
    O: for<'r> sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres> + Send + Unpin,
{
    pub fn bind<T>(mut self, value: T) -> Self
    where
        T: 'q + Debug + sqlx::Encode<'q, Postgres> + sqlx::Type<Postgres>,
    {
        self.params.push(format!("{:?}", value));
        self.inner = self.inner.bind(value);
        self
    }

    pub async fn fetch_one<'e, 'c: 'e, E>(self, executor: E) -> Result<O, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_one(executor).instrument(span).await
    }

    pub async fn fetch_all<'e, 'c: 'e, E>(self, executor: E) -> Result<Vec<O>, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_all(executor).instrument(span).await
    }

    pub async fn fetch_optional<'e, 'c: 'e, E>(self, executor: E) -> Result<Option<O>, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        let span = params_span(self.sql, &self.params);
        self.inner.fetch_optional(executor).instrument(span).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：参数 Span 的启用必须跟随 SQL 日志开关（`sqlx::query=debug` 指令）。
    /// 修复前 Span 用默认 target（模块路径），默认 info 级别下被禁用，
    /// `db.parameters` 永远不会进作用域，日志里看不到参数。
    #[test]
    fn params_span_enablement_follows_sql_log_toggle() {
        use tracing_subscriber::{EnvFilter, layer::SubscriberExt};

        let build = |sql_on: bool| {
            let directive = if sql_on {
                "info,tower_http=info,sqlx::query=debug"
            } else {
                "info,tower_http=info,sqlx::query=warn"
            };
            tracing::Dispatch::new(tracing_subscriber::registry().with(EnvFilter::new(directive)))
        };
        let sql = "SELECT $1";

        tracing::dispatcher::with_default(&build(false), || {
            assert!(params_span(sql, &["42".to_owned()]).is_disabled());
        });
        tracing::dispatcher::with_default(&build(true), || {
            let span = params_span(sql, &["42".to_owned()]);
            assert!(!span.is_disabled());
            assert_eq!(
                span.metadata().map(|metadata| metadata.name()),
                Some(SQL_PARAMS_SPAN)
            );
        });

        // 无参数时恒为 no-op Span，不产生任何开销。
        assert!(params_span(sql, &[]).is_disabled());
    }

    /// 回归：含敏感列名（password_hash/password/token/secret，大小写不敏感）的语句，
    /// 其全部 bind 参数必须以掩码写入日志，不得泄露真实值。
    #[test]
    fn sensitive_statements_redact_all_params() {
        let update_password =
            "UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1";
        let formatted = format_params(update_password, &["7".to_owned(), "argon2:xyz".to_owned()]);
        assert_eq!(formatted, "$1 = ***REDACTED***, $2 = ***REDACTED***");
        assert!(!formatted.contains("argon2:xyz"));

        // 关键字大小写不敏感。
        let formatted = format_params(
            "INSERT INTO users (PASSWORD_HASH) VALUES ($1)",
            &["hash-upper".to_owned()],
        );
        assert_eq!(formatted, "$1 = ***REDACTED***");

        // 普通语句保持原样，便于调试。
        let formatted = format_params(
            "SELECT title FROM articles WHERE id = $1",
            &["7".to_owned()],
        );
        assert_eq!(formatted, "$1 = 7");
    }
}
