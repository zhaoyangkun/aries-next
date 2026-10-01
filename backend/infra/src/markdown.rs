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

        let rendered = markdown_to_html(source, &options);
        // Markdown 中可能包含 Raw HTML，统一使用 Allowlist 清理后才能进入公开页面。
        // 放行 class 属性：代码块的 language-* class 是前端语法高亮的唯一语言线索，
        // class 本身不具备执行能力，安全风险可忽略。
        Ok(ammonia::Builder::default()
            .add_generic_attributes(["class"])
            .clean(&rendered)
            .to_string())
    }
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
}
