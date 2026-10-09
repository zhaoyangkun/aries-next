//! 运行日志 Repository 的 PostgreSQL 实现。
//! SQL 与查询参数原样来自原 HTTP 层实现：WHERE 条件按固定顺序生成占位序号，
//! bind 顺序与之一一对应；tail 查询保持裸 sqlx（不包参数日志），由调用方消音。

use aries_core::logs::{
    LogEntry, LogError, LogFilter, LogLevelCounts, LogList, LogRepository, LogStatsBucket,
};
use async_trait::async_trait;
use sqlx::PgPool;
use time::OffsetDateTime;

#[derive(Clone)]
pub struct PostgresLogRepository {
    pool: PgPool,
}

impl PostgresLogRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn map_sqlx(error: sqlx::Error) -> LogError {
    tracing::error!(error = %error, "log repository operation failed");
    LogError::StoreUnavailable
}

const LOG_ENTRY_COLUMNS: &str = "id, ts, level, target, message, span_name, request_id, fields";

type LogRow = (
    i64,
    OffsetDateTime,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    serde_json::Value,
);

fn row_to_entry(
    (id, ts, level, target, message, span_name, request_id, fields): LogRow,
) -> LogEntry {
    LogEntry {
        id,
        ts,
        level,
        target,
        message,
        span_name,
        request_id,
        fields,
    }
}

/// 过滤条件：按固定顺序生成 WHERE 条件与 bind，占位序号连续编号。
struct FilterSql<'a> {
    filter: &'a LogFilter,
}

impl<'a> FilterSql<'a> {
    /// 返回 (where_clause, 下一个可用 bind 序号)。
    fn where_clause(&self) -> (String, u32) {
        let mut conditions = vec!["true".to_string()];
        let mut bind_index = 1u32;
        if self.filter.levels.is_some() {
            conditions.push(format!("level = ANY(${bind_index})"));
            bind_index += 1;
        }
        if self.filter.target.is_some() {
            conditions.push(format!("target = ${bind_index}"));
            bind_index += 1;
        }
        if self.filter.exclude_target.is_some() {
            conditions.push(format!("target <> ${bind_index}"));
            bind_index += 1;
        }
        if self.filter.keyword.is_some() {
            // 同时匹配 fields 的 JSON 文本：SQL 语句、HTTP 路径等都在 fields 里。
            conditions.push(format!(
                "(message ILIKE ${bind_index} ESCAPE '\\' OR fields::text ILIKE ${bind_index} ESCAPE '\\')"
            ));
            bind_index += 1;
        }
        if self.filter.request_id.is_some() {
            conditions.push(format!("request_id = ${bind_index}"));
            bind_index += 1;
        }
        if self.filter.start.is_some() {
            conditions.push(format!("ts >= ${bind_index}"));
            bind_index += 1;
        }
        if self.filter.end.is_some() {
            conditions.push(format!("ts < ${bind_index}"));
            bind_index += 1;
        }
        (conditions.join(" AND "), bind_index)
    }

    // bind 顺序必须与 where_clause 的条件顺序严格一致。
    fn bind_scalar<'q>(
        &'q self,
        mut query: sqlx::query::QueryScalar<'q, sqlx::Postgres, i64, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::QueryScalar<'q, sqlx::Postgres, i64, sqlx::postgres::PgArguments> {
        if let Some(ref set) = self.filter.levels {
            query = query.bind(set);
        }
        if let Some(ref value) = self.filter.target {
            query = query.bind(value);
        }
        if let Some(ref value) = self.filter.exclude_target {
            query = query.bind(value);
        }
        if let Some(ref value) = self.filter.keyword {
            query = query.bind(value);
        }
        if let Some(ref value) = self.filter.request_id {
            query = query.bind(value);
        }
        if let Some(value) = self.filter.start {
            query = query.bind(value);
        }
        if let Some(value) = self.filter.end {
            query = query.bind(value);
        }
        query
    }

    fn bind_rows<'q>(
        &'q self,
        mut query: sqlx::query::QueryAs<'q, sqlx::Postgres, LogRow, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::QueryAs<'q, sqlx::Postgres, LogRow, sqlx::postgres::PgArguments> {
        if let Some(ref set) = self.filter.levels {
            query = query.bind(set);
        }
        if let Some(ref value) = self.filter.target {
            query = query.bind(value);
        }
        if let Some(ref value) = self.filter.exclude_target {
            query = query.bind(value);
        }
        if let Some(ref value) = self.filter.keyword {
            query = query.bind(value);
        }
        if let Some(ref value) = self.filter.request_id {
            query = query.bind(value);
        }
        if let Some(value) = self.filter.start {
            query = query.bind(value);
        }
        if let Some(value) = self.filter.end {
            query = query.bind(value);
        }
        query
    }
}

#[async_trait]
impl LogRepository for PostgresLogRepository {
    async fn list(
        &self,
        filter: &LogFilter,
        page: u32,
        page_size: u32,
        order_asc: bool,
        cursor: Option<(OffsetDateTime, i64)>,
    ) -> Result<LogList, LogError> {
        let filters = FilterSql { filter };
        let (where_clause, bind_index) = filters.where_clause();
        let order_dir = if order_asc { "ASC" } else { "DESC" };

        let count_sql = format!("SELECT COUNT(*) FROM server_logs WHERE {where_clause}");
        let total = filters
            .bind_scalar(sqlx::query_scalar::<sqlx::Postgres, i64>(&count_sql))
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;

        // keyset 模式：游标条件只作用于数据查询（total 仍是筛选全集的计数），
        // 元组比较 (ts, id) < (...) 走 server_logs_ts_id_idx 复合索引，忽略 page。
        let (data_where, limit_idx) = if cursor.is_some() {
            (
                format!(
                    "{where_clause} AND (ts, id) < (${bind_index}, ${})",
                    bind_index + 1
                ),
                bind_index + 2,
            )
        } else {
            (where_clause, bind_index)
        };
        let data_sql = format!(
            "SELECT {LOG_ENTRY_COLUMNS} FROM server_logs WHERE {data_where} \
             ORDER BY ts {order_dir}, id {order_dir} LIMIT ${limit_idx} OFFSET ${}",
            limit_idx + 1
        );
        let offset = if cursor.is_some() {
            0
        } else {
            i64::from(page.saturating_sub(1) * page_size)
        };
        let query = filters.bind_rows(sqlx::query_as::<sqlx::Postgres, LogRow>(&data_sql));
        let query = if let Some((cursor_ts, cursor_id)) = cursor {
            query.bind(cursor_ts).bind(cursor_id)
        } else {
            query
        };
        let rows = query
            .bind(i64::from(page_size))
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;

        Ok(LogList {
            items: rows.into_iter().map(row_to_entry).collect(),
            total,
            page,
            page_size,
        })
    }

    async fn list_around(
        &self,
        filter: &LogFilter,
        around_id: i64,
        context: u32,
    ) -> Result<LogList, LogError> {
        let filters = FilterSql { filter };

        // 锚点行必须存在。
        let anchor = sqlx::query_as::<_, LogRow>(&format!(
            "SELECT {LOG_ENTRY_COLUMNS} FROM server_logs WHERE id = $1"
        ))
        .bind(around_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?
        .ok_or(LogError::NotFound)?;
        let anchor_ts = anchor.1;

        let (where_clause, bind_index) = filters.where_clause();
        let anchor_ts_idx = bind_index;
        let anchor_id_idx = bind_index + 1;
        let limit_idx = bind_index + 2;

        // 前半段：早于锚点，倒序取后反转。
        let before_sql = format!(
            "SELECT {LOG_ENTRY_COLUMNS} FROM server_logs \
             WHERE {where_clause} AND (ts < ${anchor_ts_idx} OR (ts = ${anchor_ts_idx} AND id < ${anchor_id_idx})) \
             ORDER BY ts DESC, id DESC LIMIT ${limit_idx}"
        );
        let before_rows = filters
            .bind_rows(sqlx::query_as::<sqlx::Postgres, LogRow>(&before_sql))
            .bind(anchor_ts)
            .bind(around_id)
            .bind(i64::from(context))
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;

        // 后半段：晚于锚点。
        let after_sql = format!(
            "SELECT {LOG_ENTRY_COLUMNS} FROM server_logs \
             WHERE {where_clause} AND (ts > ${anchor_ts_idx} OR (ts = ${anchor_ts_idx} AND id > ${anchor_id_idx})) \
             ORDER BY ts ASC, id ASC LIMIT ${limit_idx}"
        );
        let after_rows = filters
            .bind_rows(sqlx::query_as::<sqlx::Postgres, LogRow>(&after_sql))
            .bind(anchor_ts)
            .bind(around_id)
            .bind(i64::from(context))
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;

        let mut items: Vec<LogEntry> = before_rows.into_iter().rev().map(row_to_entry).collect();
        items.push(row_to_entry(anchor));
        items.extend(after_rows.into_iter().map(row_to_entry));

        Ok(LogList {
            total: items.len() as i64,
            page: 1,
            page_size: context,
            items,
        })
    }

    async fn level_counts_24h(&self) -> Result<LogLevelCounts, LogError> {
        let count_rows = sqlx::query_as::<_, (String, i64)>(
            "SELECT level, COUNT(*) FROM server_logs \
             WHERE ts > now() - interval '24 hours' GROUP BY level",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let mut counts = LogLevelCounts::default();
        for (level, count) in count_rows {
            counts.accumulate(&level, count);
        }
        Ok(counts)
    }

    async fn stats(&self, hours: u32) -> Result<Vec<LogStatsBucket>, LogError> {
        // 分桶粒度白名单（非用户输入拼接）：≤48h 按小时，>48h 按天。
        let bucket_unit = if hours <= 48 { "hour" } else { "day" };
        let rows = sqlx::query_as::<_, (OffsetDateTime, String, i64)>(
            "SELECT date_trunc($1, ts) AS bucket, level, COUNT(*) \
             FROM server_logs \
             WHERE ts > now() - make_interval(hours => $2) \
             GROUP BY 1, 2 ORDER BY 1",
        )
        .bind(bucket_unit)
        .bind(i32::try_from(hours).unwrap_or(24))
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut buckets: Vec<LogStatsBucket> = Vec::new();
        for (bucket_ts, level, count) in rows {
            let counts = match buckets.last_mut() {
                Some(last) if last.bucket == bucket_ts => &mut last.counts,
                _ => {
                    buckets.push(LogStatsBucket {
                        bucket: bucket_ts,
                        counts: LogLevelCounts::default(),
                    });
                    &mut buckets.last_mut().unwrap().counts
                }
            };
            counts.accumulate(&level, count);
        }
        Ok(buckets)
    }

    async fn list_targets(&self) -> Result<Vec<String>, LogError> {
        sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT target FROM server_logs ORDER BY 1 LIMIT 200",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    async fn tail_anchor(&self) -> Result<i64, LogError> {
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM server_logs")
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)
    }

    async fn tail_poll(
        &self,
        filter: &LogFilter,
        after_id: i64,
        limit: i64,
    ) -> Result<Vec<LogEntry>, LogError> {
        let filters = FilterSql { filter };
        let (where_clause, bind_index) = filters.where_clause();
        let sql = format!(
            "SELECT {LOG_ENTRY_COLUMNS} FROM server_logs \
             WHERE {where_clause} AND id > ${bind_index} ORDER BY id ASC LIMIT ${}",
            bind_index + 1
        );
        let rows = filters
            .bind_rows(sqlx::query_as::<sqlx::Postgres, LogRow>(&sql))
            .bind(after_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(rows.into_iter().map(row_to_entry).collect())
    }

    async fn count_errors_since(&self, since: OffsetDateTime) -> Result<i64, LogError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM server_logs WHERE level = 'ERROR' AND ts > $1",
        )
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)
    }
}
