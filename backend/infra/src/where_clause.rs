//! 动态 WHERE 条件构建的共享 helper。
//!
//! 分页列表需要 count + data 两条查询：它们必须共享同一份 WHERE 条件与
//! bind 序列。手写两遍 `conditions.push(format!(...))` + 两遍 `if let Some ... bind`
//! 既重复又脆弱（bind 顺序必须一致，靠人眼对齐）。`WhereBuilder` 把条件片段与
//! 绑定值收集到一处，`push` 自动派生 `$n` 占位序号，`clause` 生成 WHERE 子句，
//! `bind_to` 把绑定值按 push 顺序重放到任意查询上——count 和 data 天然一致。
//!
//! 条件模板用 `{}` 作为占位符（可出现多次，如 `(a ILIKE {} OR b ILIKE {})`，
//! 同一值在 SQL 中复用占位）；keyword 条件继续搭配 `like_pattern` + `ESCAPE '\'`
//! （见 `crate::like`），绑定值为转义后的匹配模式字符串。

use crate::logged::{LoggedQuery, LoggedQueryAs, LoggedQueryScalar};

/// 动态条件绑定值：当前 list 查询只出现整数与字符串两种（keyword 为 `like_pattern` 产物）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindValue {
    I64(i64),
    Str(String),
}

impl From<i64> for BindValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<String> for BindValue {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<&str> for BindValue {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

/// 可接收条件绑定值的查询目标：`logged` 系列包装器的统一抽象，
/// 让 `WhereBuilder::bind_to` 对 count（scalar）与 data（as）查询通用。
pub trait BindConditions<'q> {
    fn bind_condition(self, value: &'q BindValue) -> Self;
}

impl<'q> BindConditions<'q> for LoggedQuery<'q> {
    fn bind_condition(self, value: &'q BindValue) -> Self {
        match value {
            BindValue::I64(value) => self.bind(*value),
            BindValue::Str(value) => self.bind(value.as_str()),
        }
    }
}

impl<'q, O> BindConditions<'q> for LoggedQueryAs<'q, O>
where
    O: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
{
    fn bind_condition(self, value: &'q BindValue) -> Self {
        match value {
            BindValue::I64(value) => self.bind(*value),
            BindValue::Str(value) => self.bind(value.as_str()),
        }
    }
}

impl<'q, O> BindConditions<'q> for LoggedQueryScalar<'q, O>
where
    O: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    fn bind_condition(self, value: &'q BindValue) -> Self {
        match value {
            BindValue::I64(value) => self.bind(*value),
            BindValue::Str(value) => self.bind(value.as_str()),
        }
    }
}

/// 动态 WHERE 条件收集器：条件片段与绑定值一一对应，`$n` 序号按 push 顺序派生。
#[derive(Debug, Default)]
pub struct WhereBuilder {
    conditions: Vec<String>,
    values: Vec<BindValue>,
}

impl WhereBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一个无条件片段（如 `deleted_at IS NULL`），不占绑定序号。
    pub fn push_static(&mut self, condition: &str) -> &mut Self {
        self.conditions.push(condition.to_owned());
        self
    }

    /// 追加一个条件并登记绑定值：模板中的每个 `{}` 都被替换为本条件的 `$n` 占位序号
    /// （同一占位可在模板中出现多次，绑定值只登记一次）。
    pub fn push(&mut self, template: &str, value: impl Into<BindValue>) -> &mut Self {
        let index = self.values.len() + 1;
        self.conditions
            .push(template.replace("{}", &format!("${index}")));
        self.values.push(value.into());
        self
    }

    /// 生成 `WHERE ...` 子句；无条件时生成恒真 `WHERE true`，SQL 始终合法
    /// （与 `FilterSql` 的起始 `true` 条件同一手法）。
    pub fn clause(&self) -> String {
        if self.conditions.is_empty() {
            return "WHERE true".to_owned();
        }
        format!("WHERE {}", self.conditions.join(" AND "))
    }

    /// 下一个可用的绑定占位序号（1 起），供 LIMIT/OFFSET 等追加占位使用。
    pub fn next_index(&self) -> u32 {
        self.values.len() as u32 + 1
    }

    /// 把收集的绑定值按 push 顺序绑定到查询上；count 与 data 查询调用同一份 builder，
    /// bind 序列从构造上保持一致。
    pub fn bind_to<'q, Q>(&'q self, query: Q) -> Q
    where
        Q: BindConditions<'q>,
    {
        let mut query = query;
        for value in &self.values {
            query = query.bind_condition(value);
        }
        query
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 记录绑定顺序的测试目标：验证 bind_to 按 push 顺序重放绑定值。
    #[derive(Default)]
    struct RecordingTarget {
        seen: Vec<BindValue>,
    }

    impl<'q> BindConditions<'q> for RecordingTarget {
        fn bind_condition(mut self, value: &'q BindValue) -> Self {
            self.seen.push(value.clone());
            self
        }
    }

    #[test]
    fn push_assigns_sequential_placeholders() {
        let mut builder = WhereBuilder::new();
        builder.push_static("deleted_at IS NULL");
        builder.push("target_type = {}", "article");
        builder.push("target_id = {}", 7_i64);

        assert_eq!(
            builder.clause(),
            "WHERE deleted_at IS NULL AND target_type = $1 AND target_id = $2"
        );
        assert_eq!(builder.next_index(), 3);
    }

    #[test]
    fn push_reuses_one_placeholder_for_repeated_slots() {
        let mut builder = WhereBuilder::new();
        builder.push_static("deleted_at IS NULL");
        builder.push(
            "(title ILIKE {} ESCAPE '\\' OR slug::text ILIKE {} ESCAPE '\\')",
            "%hello%",
        );

        assert_eq!(
            builder.clause(),
            "WHERE deleted_at IS NULL AND (title ILIKE $1 ESCAPE '\\' OR slug::text ILIKE $1 ESCAPE '\\')"
        );
        // 模板出现两个占位但只登记一个绑定值。
        assert_eq!(builder.next_index(), 2);
    }

    #[test]
    fn empty_conditions_generate_tautology_where() {
        let builder = WhereBuilder::new();
        assert_eq!(builder.clause(), "WHERE true");
        assert_eq!(builder.next_index(), 1);
    }

    #[test]
    fn bind_to_replays_values_in_push_order() {
        let mut builder = WhereBuilder::new();
        builder.push_static("deleted_at IS NULL");
        builder.push("category_id = {}", 3_i64);
        builder.push("status = {}", "published");
        builder.push("(title ILIKE {} OR url ILIKE {})", "%a%_b\\%".to_owned());

        let target = builder.bind_to(RecordingTarget::default());
        assert_eq!(
            target.seen,
            vec![
                BindValue::I64(3),
                BindValue::Str("published".to_owned()),
                BindValue::Str("%a%_b\\%".to_owned()),
            ]
        );

        // 同一份 builder 可重复重放（count + data 两遍），序列一致。
        let again = builder.bind_to(RecordingTarget::default());
        assert_eq!(again.seen, target.seen);
    }
}
