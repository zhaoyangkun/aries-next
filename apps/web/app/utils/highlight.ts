// 搜索关键词高亮：把文本按多个关键词（不区分大小写）拆分成片段，
// 由 <HighlightText> 渲染为 <mark>。正则特殊字符转义后按长度降序，保证最长词优先匹配。

export interface TextSegment {
  text: string
  match: boolean
}

/** 把搜索词拆成关键词列表（空格分隔、去重、去空白项）。 */
export function parseKeywords(input: string | null | undefined): string[] {
  if (!input) return []
  return [...new Set(input.split(/\s+/).map((part) => part.trim()).filter(Boolean))]
}

/** 按关键词拆分文本，命中片段标记 match: true；无关键词时返回原文本单片段。 */
export function splitByKeywords(text: string, keywords: readonly string[]): TextSegment[] {
  const terms = [...new Set(keywords.map((kw) => kw.trim()).filter(Boolean))].sort(
    (a, b) => b.length - a.length,
  )
  if (!text || terms.length === 0) return [{ text, match: false }]

  const pattern = terms.map(escapeRegExp).join('|')
  const regex = new RegExp(`(${pattern})`, 'gi')
  const segments: TextSegment[] = []
  let last = 0
  for (const match of text.matchAll(regex)) {
    const index = match.index ?? 0
    if (index > last) segments.push({ text: text.slice(last, index), match: false })
    segments.push({ text: match[0], match: true })
    last = index + match[0].length
  }
  if (last < text.length) segments.push({ text: text.slice(last), match: false })
  return segments.length > 0 ? segments : [{ text, match: false }]
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
