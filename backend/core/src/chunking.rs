//! 文章分块：为 Embedding / RAG 把 Markdown 正文切成带标题上下文的文本块。
//! 纯函数、无 I/O，切分规则与上限集中在此，保证 worker 与测试行为一致。

/// 单个 chunk 的正文上限（按字符计，中文按 1 字 1 符）。
/// 取 ~800 字以兼容常见 embedding 模型的输入上限（如 bge-m3 8192 token）。
pub const MAX_CHUNK_CHARS: usize = 800;

/// 一个待嵌入的内容块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentChunk {
    /// 所属小节标题（H1-H6 文本，无前导 #），无标题小节为空串。
    pub heading: String,
    /// 块正文：已去除 Markdown 控制符的纯文本。
    pub content: String,
}

impl ContentChunk {
    /// 提交给 embedding 的完整文本：标题作为语义前缀注入。
    pub fn to_embed_text(&self) -> String {
        if self.heading.is_empty() {
            self.content.clone()
        } else {
            format!("{}\n{}", self.heading, self.content)
        }
    }
}

/// 按标题层级切分 Markdown：每个标题开启新块，标题文本记入 `heading`；
/// 块超长时按段落（空行分隔）继续细分，仍超长的单段硬切到上限。
pub fn split_markdown(markdown: &str) -> Vec<ContentChunk> {
    let mut chunks = Vec::new();
    let mut heading = String::new();
    let mut body = String::new();

    let flush = |heading: &str, body: &str, chunks: &mut Vec<ContentChunk>| {
        for paragraph in body.split("\n\n") {
            let paragraph = paragraph.trim();
            if paragraph.is_empty() {
                continue;
            }
            for piece in hard_split(paragraph, MAX_CHUNK_CHARS) {
                chunks.push(ContentChunk {
                    heading: heading.to_owned(),
                    content: piece,
                });
            }
        }
    };

    for line in markdown.lines() {
        if let Some(title) = parse_heading(line) {
            flush(&heading, &body, &mut chunks);
            heading = title;
            body.clear();
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(&heading, &body, &mut chunks);
    chunks
}

/// 提取 ATX 标题文本（`#` 1-6 级，要求 `#` 后有空格），非标题返回 None。
fn parse_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let level = trimmed.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.starts_with(' ') {
        return None;
    }
    let title = strip_md_inline(rest.trim());
    if title.is_empty() { None } else { Some(title) }
}

/// 去除行内 Markdown 控制符（粗斜体、行内代码、链接），保留可读文本。
fn strip_md_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' | '`' => {}
            '[' => {
                // 链接 [text](url)：保留 text，跳过 (url)。
                let mut depth = 1;
                for c in chars.by_ref() {
                    if c == '[' {
                        depth += 1;
                    } else if c == ']' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        out.push(c);
                    }
                }
                if chars.peek() == Some(&'(') {
                    let mut depth = 1;
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out.trim().to_owned()
}

/// 超长文本按上限硬切（按字符边界），每片尽量在句读处断开。
fn hard_split(text: &str, max_chars: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while rest.chars().count() > max_chars {
        let mut boundary = None;
        for (index, c) in rest.char_indices().take(max_chars) {
            if matches!(c, '。' | '！' | '？' | '.' | '!' | '?' | '；' | ';' | '\n') {
                boundary = Some(index + c.len_utf8());
            }
        }
        let boundary = boundary
            .or_else(|| rest.char_indices().nth(max_chars).map(|(index, _)| index))
            .unwrap_or(rest.len());
        let (piece, remain) = rest.split_at(boundary);
        let piece = piece.trim();
        if !piece.is_empty() {
            pieces.push(piece.to_owned());
        }
        rest = remain.trim_start();
    }
    if !rest.is_empty() {
        pieces.push(rest.to_owned());
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_by_headings_and_keeps_heading() {
        let md = "# 前言\n\n开场白内容。\n\n## 安装\n\n第一步。\n\n第二步。\n\n## 结论\n\n结束了。";
        let chunks = split_markdown(md);
        // 段落级粒度：每个正文段落一个块。
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0].heading, "前言");
        assert!(chunks[0].content.contains("开场白内容"));
        assert_eq!(chunks[1].heading, "安装");
        assert!(chunks[1].content.contains("第一步"));
        assert_eq!(chunks[2].heading, "安装");
        assert!(chunks[2].content.contains("第二步"));
        assert_eq!(chunks[3].heading, "结论");
    }

    #[test]
    fn heading_strips_markdown_syntax() {
        let chunks = split_markdown("## **粗体** 与 `code`\n\n内容。");
        assert_eq!(chunks[0].heading, "粗体 与 code");
    }

    #[test]
    fn link_text_is_kept_url_dropped() {
        let chunks = split_markdown("# [链接文字](https://example.com)\n\n正文");
        assert_eq!(chunks[0].heading, "链接文字");
    }

    #[test]
    fn oversized_section_is_split_by_paragraph_then_hard() {
        // 17 字 × 60 ≈ 1020 字，单段超过 800 字上限，触发硬切。
        let paragraph = "这是一句很长的话，用来撑满一个块。".repeat(60);
        let md = format!("# 大节\n\n{paragraph}\n\n{paragraph}");
        let chunks = split_markdown(&md);
        assert!(chunks.len() > 2);
        assert!(chunks.iter().all(|chunk| chunk.heading == "大节"));
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.content.chars().count() <= MAX_CHUNK_CHARS)
        );
    }

    #[test]
    fn empty_document_yields_no_chunks() {
        assert!(split_markdown("").is_empty());
        assert!(split_markdown("\n\n  \n").is_empty());
    }

    #[test]
    fn to_embed_text_prefixes_heading() {
        let chunk = ContentChunk {
            heading: "安装".to_owned(),
            content: "第一步。".to_owned(),
        };
        assert_eq!(chunk.to_embed_text(), "安装\n第一步。");
        let bare = ContentChunk {
            heading: String::new(),
            content: "无标题。".to_owned(),
        };
        assert_eq!(bare.to_embed_text(), "无标题。");
    }
}
