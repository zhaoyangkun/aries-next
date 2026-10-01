//! ILIKE 关键字搜索的共享 helper。
//!
//! 用户输入的 keyword 中的 `%`/`_`/`\` 必须按字面量匹配，否则会被当作
//! LIKE 通配符。统一使用本模块的转义，SQL 侧配套 `ESCAPE '\'`
//!（见各 Repository 的 keyword 条件）。

/// ILIKE 通配符转义：keyword 中的 `%`/`_`/`\` 前面加 `\`，使其按字面量匹配。
pub fn escape_like(keyword: &str) -> String {
    keyword
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// 生成 `%{keyword}%` 匹配模式（keyword 已按 [`escape_like`] 转义），
/// 直接绑定到 `ILIKE $n ESCAPE '\'` 条件。
pub fn like_pattern(keyword: &str) -> String {
    format!("%{}%", escape_like(keyword))
}

/// 生成 `{keyword}%` 前缀匹配模式（keyword 已按 [`escape_like`] 转义），
/// 供搜索建议区分「标题前缀命中」与「包含命中」的排序档。
pub fn prefix_pattern(keyword: &str) -> String {
    format!("{}%", escape_like(keyword))
}

#[cfg(test)]
mod tests {
    use super::{escape_like, like_pattern};

    #[test]
    fn escape_like_escapes_wildcards_and_backslash() {
        assert_eq!(escape_like("plain"), "plain");
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        // 反斜杠先转义，避免二次转义破坏后续 %/_ 的转义序列。
        assert_eq!(escape_like("a\\%b"), "a\\\\\\%b");
        assert_eq!(like_pattern("100%"), "%100\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
    }
}
