// 阅读时长估算：中文按字符、英文按词的经验公式（chars/400 + words/200），分钟向上取整至少 1。
// 公开 API 出于安全不返回 markdown_source，因此从后端渲染好的 HTML 剥离标签后统计，
// 输入在 SSR 与客户端一致，无 hydration 差异。

export interface ReadingEstimate {
  /** 预计阅读分钟数 */
  minutes: number
  /** 字数统计：CJK 字符数 + 拉丁词数 */
  count: number
}

/** 从纯文本估算：CJK 字符逐个计数，连续拉丁字母/数字记为一个词 */
export function readingTimeFromText(text: string): ReadingEstimate {
  const cjkChars = (text.match(/[\u2e80-\u9fff\uf900-\ufaff\u3000-\u303f\uff00-\uffef]/g) ?? []).length
  const latinWords = (text.match(/[a-zA-Z0-9]+/g) ?? []).length
  const minutes = Math.max(1, Math.ceil(cjkChars / 400 + latinWords / 200))
  return { minutes, count: cjkChars + latinWords }
}

/** 从渲染后的文章 HTML 估算：剥离标签与脚本样式内容，只统计可见文本 */
export function readingTimeFromHtml(html: string | null | undefined): ReadingEstimate {
  if (!html) return { minutes: 1, count: 0 }
  const text = html
    .replace(/<(script|style)[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<[^>]+>/g, ' ')
  return readingTimeFromText(text)
}
