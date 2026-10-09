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
        // 数学公式：$...$ / $$...$$ 与 ```math 代码块；产物保留 $ 定界符原文，
        // 由前端 KaTeX 按 data-math-style / language-math 线索做最终渲染。
        options.extension.math_dollars = true;
        options.extension.math_code = true;

        let normalized = normalize_legacy_display_math(source);
        let rendered = markdown_to_html(normalized.as_ref(), &options);
        // Markdown 中可能包含 Raw HTML，统一使用 Allowlist 清理后才能进入公开页面。
        // 放行 class 属性：代码块的 language-* class 是前端语法高亮的唯一语言线索，
        // class 本身不具备执行能力，安全风险可忽略。
        // 放行 span 的 data-math-style 属性：comrak 数学扩展的渲染产物，
        // 前端 KaTeX 依此识别行内/展示公式。
        Ok(ammonia::Builder::default()
            .add_generic_attributes(["class"])
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
