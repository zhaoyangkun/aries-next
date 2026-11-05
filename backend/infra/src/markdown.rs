use std::borrow::Cow;

use aries_core::content::{ContentError, MarkdownRenderer};
use comrak::{Options, markdown_to_html};

#[derive(Debug, Default, Clone, Copy)]
pub struct ComrakMarkdownRenderer;

impl MarkdownRenderer for ComrakMarkdownRenderer {
    fn render(&self, source: &str) -> Result<String, ContentError> {
        let mut options = Options::default();
        options.extension.strikethrough = true;
        options.extension.table = true;
        options.extension.tasklist = true;
        options.extension.autolink = true;
        // 脚注：旧站文章大量使用 [^n] 引用 + 文末定义语法，开启后渲染为
        // 上标引用 + 文末脚注列表，而非字面文本
        options.extension.footnotes = true;
        // 数学公式：$...$ / $$...$$ 与 ```math 代码块；产物保留 $ 定界符原文，
        // 由前端 KaTeX 按 data-math-style / language-math 线索做最终渲染。
        options.extension.math_dollars = true;
        options.extension.math_code = true;

        let display_normalized = normalize_legacy_display_math(source);
        let normalized = normalize_legacy_inline_math(display_normalized.as_ref());
        let rendered = markdown_to_html(normalized.as_ref(), &options);
        // Markdown 中可能包含 Raw HTML，统一使用 Allowlist 清理后才能进入公开页面。
        // 放行 class 属性：代码块的 language-* class 是前端语法高亮的唯一语言线索，
        // class 本身不具备执行能力，安全风险可忽略。
        // 放行 id 属性：脚注的跳转锚点（fn / fnref 互指）依赖 id，id 本身不具备执行能力。
        // 放行相对/片段 URL：脚注回链（#fnref-n）与站内相对链接需要 href 片段，
        // 协议相对外链（//host）虽可通过，但与已放行的绝对 http(s) 外链能力等价，无新增风险面。
        // 放行 span 的 data-math-style 属性：comrak 数学扩展的渲染产物，
        // 前端 KaTeX 依此识别行内/展示公式。
        Ok(ammonia::Builder::default()
            .add_generic_attributes(["class", "id"])
            .url_relative(ammonia::UrlRelative::PassThrough)
            .add_tag_attributes("span", ["data-math-style"])
            .clean(&rendered)
            .to_string())
    }
}

/// 识别 fenced code 起始/结束标记（``` 或 ~~~，可带至多 3 空格缩进）。
/// 返回 (fence 字符, 围栏长度)。
fn fence_marker(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|c| *c == marker).count();
    (count >= 3).then_some((marker, count))
}

/// 兼容遗留的不规范展示公式写法：源文 `$$\n公式\n\n$$`（闭合 `$$` 前有空行）
/// 会被拆成两个段落，comrak 的 math_dollars 只在段落内做 inline 解析，
/// 定界符跨段导致无法识别。这里把配对的 `$$` 块内空行折叠掉（公式对空白不敏感）。
/// 保守策略：只处理独占一行的 `$$` 配对块，fenced code 内的内容一律不动。
fn normalize_legacy_display_math(source: &str) -> Cow<'_, str> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut in_fence: Option<(char, usize)> = None;
    let mut changed = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some((marker, len)) = fence_marker(line) {
            match in_fence {
                // 结束围栏要求同类字符且长度不小于起始围栏
                Some((open, open_len)) if open == marker && len >= open_len => in_fence = None,
                None => in_fence = Some((marker, len)),
                _ => {}
            }
            out.push(line);
            i += 1;
            continue;
        }
        if in_fence.is_none() && line.trim() == "$$" {
            // 向后找下一个独占一行的 $$ 作为闭合定界符
            if let Some(offset) = lines[i + 1..].iter().position(|l| l.trim() == "$$") {
                let close = i + 1 + offset;
                let inner = &lines[i + 1..close];
                // 块内出现 fence 起始标记则放弃配对（$$ 可能只是普通文本）
                let has_fence = inner.iter().any(|l| fence_marker(l).is_some());
                let has_blank = inner.iter().any(|l| l.trim().is_empty());
                if !has_fence && has_blank {
                    out.push("$$");
                    out.extend(inner.iter().filter(|l| !l.trim().is_empty()).copied());
                    out.push("$$");
                    changed = true;
                    i = close + 1;
                    continue;
                }
            }
        }
        out.push(line);
        i += 1;
    }

    if !changed {
        return Cow::Borrowed(source);
    }
    let mut result = out.join("\n");
    if source.ends_with('\n') {
        result.push('\n');
    }
    Cow::Owned(result)
}

/// 兼容旧内容中 `$ x_i $`、`$ \phi(x)$` 这类定界符内侧带空白的行内公式。
/// comrak 会把前一处公式的闭合符与后一处公式的起始符错配，因此需在 Markdown 解析前
/// 去掉定界符内侧空白。只处理具有明显 TeX 特征的内容，避免把金额等普通文本当成公式。
fn normalize_legacy_inline_math(source: &str) -> Cow<'_, str> {
    let mut result = String::with_capacity(source.len());
    let mut in_fence: Option<(char, usize)> = None;
    let mut changed = false;

    for chunk in source.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        if let Some((marker, len)) = fence_marker(line) {
            match in_fence {
                Some((open, open_len)) if open == marker && len >= open_len => in_fence = None,
                None => in_fence = Some((marker, len)),
                _ => {}
            }
            result.push_str(chunk);
            continue;
        }
        if in_fence.is_some() {
            result.push_str(chunk);
            continue;
        }

        match normalize_inline_math_line(line) {
            Cow::Borrowed(_) => result.push_str(chunk),
            Cow::Owned(normalized) => {
                result.push_str(&normalized);
                if chunk.ends_with('\n') {
                    result.push('\n');
                }
                changed = true;
            }
        }
    }

    if changed {
        Cow::Owned(result)
    } else {
        Cow::Borrowed(source)
    }
}

fn normalize_inline_math_line(line: &str) -> Cow<'_, str> {
    let bytes = line.as_bytes();
    let mut result = String::with_capacity(line.len());
    let mut copied_until = 0;
    let mut index = 0;
    let mut code_delimiter: Option<usize> = None;
    let mut math_open: Option<usize> = None;
    let mut changed = false;

    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index = escaped_char_end(line, index);
                continue;
            }
            b'`' => {
                let run = backtick_run(bytes, index);
                match code_delimiter {
                    Some(open) if open == run => code_delimiter = None,
                    None => code_delimiter = Some(run),
                    _ => {}
                }
                math_open = None;
                index += run;
                continue;
            }
            b'$' if code_delimiter.is_none() => {
                if !is_single_dollar(bytes, index) {
                    math_open = None;
                    index += 1;
                    continue;
                }
                if let Some(open) = math_open {
                    let inner = &line[open + 1..index];
                    let formula = inner.trim();
                    if inner.len() == formula.len() {
                        math_open = None;
                        index += 1;
                        continue;
                    }
                    if looks_like_math(formula) {
                        result.push_str(&line[copied_until..open]);
                        result.push('$');
                        result.push_str(formula);
                        result.push('$');
                        copied_until = index + 1;
                        changed = true;
                        math_open = None;
                        index += 1;
                        continue;
                    }
                    // 当前闭合符也可能是后一段公式的起始符。
                    math_open = Some(index);
                } else {
                    math_open = Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }

    if !changed {
        return Cow::Borrowed(line);
    }
    result.push_str(&line[copied_until..]);
    Cow::Owned(result)
}

fn escaped_char_end(line: &str, slash: usize) -> usize {
    match line[slash + 1..].chars().next() {
        Some(character) => slash + 1 + character.len_utf8(),
        None => slash + 1,
    }
}

fn backtick_run(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .take_while(|byte| **byte == b'`')
        .count()
}

fn is_single_dollar(bytes: &[u8], index: usize) -> bool {
    let previous_is_dollar = index > 0 && bytes[index - 1] == b'$';
    let next_is_dollar = index + 1 < bytes.len() && bytes[index + 1] == b'$';
    !previous_is_dollar && !next_is_dollar
}

fn looks_like_math(value: &str) -> bool {
    if value.is_empty() || value.contains('$') || value.contains('`') {
        return false;
    }
    let mut characters = value.chars();
    if matches!(characters.next(), Some(character) if character.is_alphabetic())
        && characters.next().is_none()
    {
        return true;
    }
    value.chars().any(|character| {
        matches!(
            character,
            '\\' | '_'
                | '^'
                | '{'
                | '}'
                | '='
                | '+'
                | '-'
                | '*'
                | '/'
                | '|'
                | '<'
                | '>'
                | '!'
                | '&'
                | '('
                | ')'
                | '['
                | ']'
                | ','
                | ';'
                | ':'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_keeps_markdown_but_removes_executable_html() {
        let html = ComrakMarkdownRenderer
            .render("# Title\n\n<script>alert(1)</script>\n\n[bad](javascript:alert(1))")
            .expect("markdown should render");

        assert!(html.contains("<h1>Title</h1>"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("javascript:"));
    }

    #[test]
    fn renderer_keeps_code_language_class_for_highlighting() {
        let html = ComrakMarkdownRenderer
            .render("```rust\nfn main() {}\n```")
            .expect("markdown should render");
        // 前端 highlight.js 依赖 language-* class 识别语言。
        assert!(html.contains("language-rust"));
    }

    #[test]
    fn footnotes_render_as_references_and_backlinks_survive_sanitize() {
        let html = ComrakMarkdownRenderer
            .render("遵循命名规范[^1]。\n\n[^1]: [Rust 规范](https://doc.rust-lang.org/style/)。")
            .expect("markdown should render");

        // 引用渲染为上标（comrak 0.35 标记 class="footnote-ref"），不再是字面 [^1]
        assert!(html.contains("footnote-ref"), "unexpected html: {html}");
        assert!(!html.contains("[^1]"), "unexpected html: {html}");
        // 文末定义列表 + 回链：id 锚点与 # 片段 href 必须穿过 Sanitization
        assert!(html.contains("id=\"fn-1\""), "unexpected html: {html}");
        assert!(html.contains("href=\"#fn"), "unexpected html: {html}");
    }

    #[test]
    fn inline_math_renders_span_with_data_math_style() {
        let html = ComrakMarkdownRenderer
            .render("假设模型输出为 $\\hat{y}$，损失为 $F$。")
            .expect("markdown should render");

        // comrak 0.35 的产物：span 内容是剥离定界符后的原始 TeX
        assert!(
            html.contains("<span data-math-style=\"inline\">\\hat{y}</span>"),
            "unexpected html: {html}"
        );
        assert!(html.contains("<span data-math-style=\"inline\">F</span>"));
    }

    #[test]
    fn display_math_renders_span_with_display_style() {
        let html = ComrakMarkdownRenderer
            .render("展示公式：\n\n$$\nJ = \\sum_{m=0}^Nf(y_i, \\hat{y_i})\n$$\n\n结束。")
            .expect("markdown should render");

        assert!(
            html.contains("<span data-math-style=\"display\">"),
            "unexpected html: {html}"
        );
        assert!(html.contains("\\sum_{m=0}^Nf(y_i, \\hat{y_i})"));
    }

    #[test]
    fn legacy_display_math_with_blank_line_is_folded() {
        // 遗留写法：闭合 $$ 前有空行，会被拆段导致 comrak 无法识别
        let source = "交叉熵损失：\n\n$$\nJ = \\sum_{m=0}^Nf(y_i, \\hat{y_i})\n\n$$\n\n\
                      平均绝对误差：\n\n$$\nJ_{MAE}=\\frac{1}{N}\\sum_{i=1}^{N}|y_i - \\hat{y_i}|\n\n$$\n";
        let html = ComrakMarkdownRenderer
            .render(source)
            .expect("markdown should render");

        let display_count = html.matches("data-math-style=\"display\"").count();
        assert_eq!(display_count, 2, "unexpected html: {html}");
        assert!(html.contains("\\frac{1}{N}"));
    }

    #[test]
    fn dollar_signs_inside_code_fence_are_untouched() {
        let html = ComrakMarkdownRenderer
            .render("```\n$$\nnot math\n\n$$\n```")
            .expect("markdown should render");

        assert!(
            !html.contains("data-math-style"),
            "code fence content must not be treated as math: {html}"
        );
        // 代码块原文（含空行）保持不变
        assert!(html.contains("not math"));
    }

    #[test]
    fn unmatched_dollars_are_left_as_text() {
        let html = ComrakMarkdownRenderer
            .render("价格是 $$ 或者单独的 $ 符号。")
            .expect("markdown should render");
        assert!(!html.contains("data-math-style"), "unexpected html: {html}");

        // 归一化对未配对的 $$ 不做任何修改
        let source = "单独一段：\n\n$$\n只有起始没有闭合\n\n后续段落\n";
        assert!(matches!(
            normalize_legacy_display_math(source),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn legacy_spaced_inline_math_is_rendered() {
        let html = ComrakMarkdownRenderer
            .render("插值公式：$ x_0 $、$ \\phi_1(x)$、$ x_j$、$ h $、$ a$。")
            .expect("markdown should render");

        assert_eq!(
            html.matches("data-math-style=\"inline\"").count(),
            5,
            "unexpected html: {html}"
        );
        for formula in ["x_0", "\\phi_1(x)", "x_j", "h", "a"] {
            assert!(
                html.contains(&format!(
                    "<span data-math-style=\"inline\">{formula}</span>"
                )),
                "missing formula {formula}: {html}"
            );
        }
    }

    #[test]
    fn spaced_dollar_normalization_preserves_code_and_non_math_text() {
        let source = "行内代码 `$ x_0 $` 与转义符号 \\$ x_0 $ 不应改动。\n\n\
                      ```text\n$ x_0 $\n```\n\n\
                      价格从 $ 100 涨到 $ 200，单独的 $ 符号保持原样。";
        let html = ComrakMarkdownRenderer
            .render(source)
            .expect("markdown should render");

        assert!(
            !html.contains("data-math-style"),
            "non-math dollars must not be normalized: {html}"
        );
        assert!(html.contains("$ x_0 $"), "unexpected html: {html}");
    }

    #[test]
    fn spaced_dollar_normalization_does_not_merge_adjacent_math() {
        let html = ComrakMarkdownRenderer
            .render("$x$ + $ y $")
            .expect("markdown should render");

        assert_eq!(
            html.matches("data-math-style=\"inline\"").count(),
            2,
            "unexpected html: {html}"
        );
        assert!(html.contains("<span data-math-style=\"inline\">x</span>"));
        assert!(html.contains("<span data-math-style=\"inline\">y</span>"));
    }

    #[test]
    fn math_code_block_renders_with_language_class() {
        let html = ComrakMarkdownRenderer
            .render("```math\nx^2 + y^2 = z^2\n```")
            .expect("markdown should render");

        // ```math 产物保留 language-math class 供前端 KaTeX 识别
        assert!(html.contains("language-math"), "unexpected html: {html}");
    }

    #[test]
    fn math_content_cannot_inject_html() {
        let html = ComrakMarkdownRenderer
            .render("$<script>alert(1)</script>$")
            .expect("markdown should render");
        assert!(!html.contains("<script"), "unexpected html: {html}");

        let html = ComrakMarkdownRenderer
            .render("$$\n<script>alert(1)</script>\n\n$$")
            .expect("markdown should render");
        assert!(!html.contains("<script"), "unexpected html: {html}");
    }

    #[test]
    fn normalize_folds_blank_lines_inside_paired_dollars() {
        let source = "$$\nJ = 1\n\n$$\n";
        let normalized = normalize_legacy_display_math(source);
        assert_eq!(normalized.as_ref(), "$$\nJ = 1\n$$\n");
    }

    #[test]
    fn normalize_ignores_dollars_inside_fences() {
        let source = "```text\n$$\nx\n\n$$\n```\n";
        assert!(matches!(
            normalize_legacy_display_math(source),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn normalize_ignores_already_well_formed_blocks() {
        let source = "$$\nJ = 1\n$$\n";
        assert!(matches!(
            normalize_legacy_display_math(source),
            Cow::Borrowed(_)
        ));
    }
}
