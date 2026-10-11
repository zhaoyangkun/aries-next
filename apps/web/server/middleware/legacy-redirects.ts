// 旧版 Aries（Go + Gin）公开路由的 301 兼容层，见 docs/phases/phase-07-migration-and-cutover.md。
// 两类规则：
// 1. PATTERN_RULES：路径形态变化但标识符不变的分页路由，正则转换后 301；
// 2. EXACT_REDIRECTS：迁移导致标识符变化（如中文标签名 slug 化为 `tag-{id}`、slug 冲突改名）
//    的精确映射表，正式切换彩排时由迁移数据生成填充，当前为空。
// 注意：/p/:page 由 server/routes/p/[page].ts 处理，不在此重复；
// 文章 /articles/:url、自定义页 /custom/:url 路径形态与新版一致，无需规则。

interface PatternRule {
  match: RegExp
  build: (page: number, id: string | null, query: URLSearchParams) => string
}

// page <= 1 时去掉分页参数，收敛到规范 URL
function withPage(base: string, page: number, query: URLSearchParams): string {
  if (page > 1) query.set('page', String(page))
  else query.delete('page')
  const qs = query.toString()
  return qs ? `${base}?${qs}` : base
}

const PATTERN_RULES: PatternRule[] = [
  // /categories/:url/p/:page → /categories/:url?page=N
  {
    match: /^\/categories\/([^/]+)\/p\/(\d+)\/?$/,
    build: (page, id, query) => withPage(`/categories/${id}`, page, query),
  },
  // /category/p/:page → /categories?page=N（旧版分类列表分页）
  {
    match: /^\/category\/p\/(\d+)\/?$/,
    build: (page, _id, query) => withPage('/categories', page, query),
  },
  // /tags/:name/p/:page → /tags/:name?page=N
  // （name 与 slug 大小写差异由后端 citext 兼容；中文名等改名情形走 EXACT_REDIRECTS）
  {
    match: /^\/tags\/([^/]+)\/p\/(\d+)\/?$/,
    build: (page, id, query) => withPage(`/tags/${id}`, page, query),
  },
  // /tag/p/:page → /tags?page=N（旧版标签列表分页）
  {
    match: /^\/tag\/p\/(\d+)\/?$/,
    build: (page, _id, query) => withPage('/tags', page, query),
  },
  // /search/p/:page → /search?page=N（保留 keyword 等原有查询参数）
  {
    match: /^\/search\/p\/(\d+)\/?$/,
    build: (page, _id, query) => withPage('/search', page, query),
  },
]

// 精确映射：旧路径（不含查询串）→ 新路径。切换彩排时按迁移实际数据填充。
const EXACT_REDIRECTS: Record<string, string> = {}

export default defineEventHandler((event) => {
  // 旧路由均为 GET 页面，非 GET/HEAD 请求不重定向（避免 301 丢失请求体语义）
  if (event.method !== 'GET' && event.method !== 'HEAD') return
  const url = getRequestURL(event)
  const pathname = url.pathname

  const exact = EXACT_REDIRECTS[pathname]
  if (exact) {
    const qs = url.searchParams.toString()
    return sendRedirect(event, qs ? `${exact}?${qs}` : exact, 301)
  }

  for (const rule of PATTERN_RULES) {
    const m = pathname.match(rule.match)
    if (!m) continue
    // 各规则的分组顺序统一为 (id?, page)；无 id 的规则第一组即页码
    const hasId = rule.match.source.includes('([^/]+)')
    const id = hasId ? decodeURIComponent(m[1]) : null
    const page = Number(hasId ? m[2] : m[1])
    if (!Number.isInteger(page) || page < 1) continue
    return sendRedirect(event, rule.build(page, id, new URLSearchParams(url.searchParams)), 301)
  }
})
