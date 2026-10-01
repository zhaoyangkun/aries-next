//! 公开搜索的纯函数工具：从正文 Markdown 中提取关键词命中片段。
//!
//! 片段是**纯文本**（不含 HTML），前端负责关键词高亮与转义，
//! 避免后端输出预渲染 HTML 带来的注入面。

/// 片段命中点前的最大前导字符数（按字符计）。
const CONTEXT_BEFORE: usize = 40;
/// 片段命中点后的最大尾随字符数（按字符计）。
const CONTEXT_AFTER: usize = 60;

/// 生成搜索命中片段：
/// - 摘要非空且已包含任一关键词时返回 `None`（前端直接高亮摘要即可）；
/// - 否则在正文纯文本中找第一个关键词命中点，截取前后文窗口并折叠空白；
/// - 窗口被截断的一端补 `…`；正文无任何命中返回 `None`。
pub fn build_snippet(keyword: &str, summary: &str, markdown_source: &str) -> Option<String> {
    let keywords: Vec<String> = keyword
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .map(|part| part.to_lowercase())
        .collect();
    if keywords.is_empty() {
        return None;
    }

    let summary_lower = lower_chars(summary);
    if !summary.is_empty()
        && keywords
            .iter()
            .any(|kw| contains_sub(&summary_lower, &lower_chars(kw)))
    {
        return None;
    }

    let plain = strip_markdown(markdown_source);
    let plain_lower = lower_chars(&plain);
    let plain_chars: Vec<char> = plain.chars().collect();

    let mut first: Option<(usize, usize)> = None;
    for kw in &keywords {
        let kw_chars = lower_chars(kw);
        if let Some(pos) = find_sub(&plain_lower, &kw_chars) {
            let end = pos + kw_chars.len();
            if first.is_none_or(|(p, _)| pos < p) {
                first = Some((pos, end));
            }
        }
    }
    let (hit_start, hit_end) = first?;

    let mut start = hit_start.saturating_sub(CONTEXT_BEFORE);
    // 起点落在拉丁单词/数字串中间时推进到词尾，避免片段从单词中间截断（CJK 无空格，直接按字符窗口截取）
    if start > 0
        && plain_chars[start].is_ascii_alphanumeric()
        && plain_chars[start - 1].is_ascii_alphanumeric()
    {
        while start < plain_chars.len() && plain_chars[start].is_ascii_alphanumeric() {
            start += 1;
        }
        while start < plain_chars.len() && plain_chars[start].is_whitespace() {
            start += 1;
        }
    }
    let end = (hit_end + CONTEXT_AFTER).min(plain_chars.len());

    let mut snippet: String = plain_chars[start..end].iter().collect();
    snippet = collapse_whitespace(&snippet);
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < plain_chars.len() {
        snippet.push('…');
    }
    Some(snippet)
}

fn lower_chars(s: &str) -> Vec<char> {
    s.chars().flat_map(char::to_lowercase).collect()
}

fn contains_sub(haystack: &[char], needle: &[char]) -> bool {
    find_sub(haystack, needle).is_some()
}

fn find_sub(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 去掉 Markdown 语法得到纯文本：链接保留锚文本、图片保留 Alt，
/// 移除强调/行内代码/标题/引用记号，最后折叠空白。
fn strip_markdown(md: &str) -> String {
    let chars: Vec<char> = md.chars().collect();
    let mut out = String::with_capacity(md.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            // 图片 ![alt](src) 与链接 [text](url)：保留方括号内文本，跳过圆括号部分
            '!' if chars.get(i + 1) == Some(&'[') => {
                i = emit_bracket_text(&chars, i + 1, &mut out);
            }
            '[' => {
                i = emit_bracket_text(&chars, i, &mut out);
            }
            // 移除常见 Markdown 记号字符
            '*' | '_' | '~' | '`' | '#' | '>' => {}
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// 从 `[` 位置出发：输出方括号内文本；若紧随其后是 `(…)` 则一并跳过。返回新索引。
fn emit_bracket_text(chars: &[char], open: usize, out: &mut String) -> usize {
    let mut i = open + 1;
    while i < chars.len() && chars[i] != ']' {
        out.push(chars[i]);
        i += 1;
    }
    // i 指向 ']' 或末尾；跳过后续 (…)
    if i < chars.len() && chars.get(i + 1) == Some(&'(') {
        let mut depth = 0;
        i += 1;
        while i < chars.len() {
            if chars[i] == '(' {
                depth += 1;
            } else if chars[i] == ')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            i += 1;
        }
    }
    i
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_is_none_when_summary_already_matches() {
        let snippet = build_snippet("rust", "一篇关于 Rust 的文章", "正文里没有任何东西");
        assert_eq!(snippet, None);
    }

    #[test]
    fn snippet_extracts_context_around_body_hit() {
        let body =
            "前言部分介绍背景知识。可变性的概念在 Rust 中通过 mut 关键字表达。后续章节继续展开。";
        let snippet = build_snippet("mut", "", body).expect("snippet");
        assert!(snippet.contains("mut"));
        assert!(snippet.contains("可变性"));
        assert!(snippet.starts_with('…') || snippet.contains("前言"));
    }

    #[test]
    fn snippet_is_case_insensitive() {
        let snippet = build_snippet("RUST", "", "学习 rust 从所有权开始").expect("snippet");
        assert!(snippet.to_lowercase().contains("rust"));
    }

    #[test]
    fn snippet_prefers_earliest_keyword_hit() {
        let snippet =
            build_snippet("世界  Rust", "", "你好世界，欢迎。这是 Rust 语言。").expect("snippet");
        assert!(snippet.contains("世界"));
        // 取最早命中点（世界）所在的片段
        assert!(snippet.find("世界").unwrap() < snippet.find("Rust").unwrap());
    }

    #[test]
    fn snippet_strips_markdown_link_but_keeps_anchor_text() {
        let body = "参考 [Rust 官方文档](https://www.rust-lang.org) 了解更多。";
        let snippet = build_snippet("官方文档", "", body).expect("snippet");
        assert_eq!(snippet, "参考 Rust 官方文档 了解更多。");
    }

    #[test]
    fn snippet_strips_emphasis_and_heading_marks() {
        let body = "## *重要* 的 `code` 与 __加粗__";
        let snippet = build_snippet("重要", "", body).expect("snippet");
        assert_eq!(snippet, "重要 的 code 与 加粗");
    }

    #[test]
    fn snippet_is_none_without_any_hit() {
        assert_eq!(build_snippet("不存在", "", "完全无关的正文内容"), None);
    }

    #[test]
    fn snippet_is_none_for_blank_keyword() {
        assert_eq!(build_snippet("   ", "", "任何正文"), None);
    }
}
