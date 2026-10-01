// Sitemap：循环分页拉取全部已发布文章与图库，输出标准 XML Sitemap
export default defineEventHandler(async (event) => {
  const siteUrl = await resolveSiteUrl(getRequestURL(event).origin)

  const articles: PublicArticleListItem[] = []
  let page = 1
  const pageSize = 100
  let total = Number.POSITIVE_INFINITY
  while (articles.length < total) {
    const res = await $fetch<PublicArticlePage>('/articles', {
      baseURL: publicApiBase(),
      query: { page, page_size: pageSize },
    })
    total = res.total
    articles.push(...res.items)
    if (res.items.length < pageSize) break
    page += 1
  }

  // 图库与文章同款循环分页拉取；摘要接口不含 updated_at，不写 lastmod
  const galleries: PublicGallerySummary[] = []
  let galleryPage = 1
  let galleryTotal = Number.POSITIVE_INFINITY
  while (galleries.length < galleryTotal) {
    const res = await $fetch<PublicGalleryPage>('/galleries', {
      baseURL: publicApiBase(),
      query: { page: galleryPage, page_size: pageSize },
    })
    galleryTotal = res.total
    galleries.push(...res.items)
    if (res.items.length < pageSize) break
    galleryPage += 1
  }

  const urls: string[] = []

  // 首页与固定页面（日志为时间线流，无详情页，仅收录列表页）
  const staticPages: Array<{ path: string; changefreq: string; priority: string }> = [
    { path: '/', changefreq: 'daily', priority: '1.0' },
    { path: '/archives', changefreq: 'weekly', priority: '0.5' },
    { path: '/categories', changefreq: 'weekly', priority: '0.5' },
    { path: '/tags', changefreq: 'weekly', priority: '0.5' },
    { path: '/links', changefreq: 'weekly', priority: '0.5' },
    { path: '/journals', changefreq: 'daily', priority: '0.5' },
    { path: '/galleries', changefreq: 'weekly', priority: '0.5' },
    { path: '/about', changefreq: 'monthly', priority: '0.3' },
  ]
  for (const item of staticPages) {
    urls.push(
      `  <url><loc>${escapeXml(siteUrl + item.path)}</loc><changefreq>${item.changefreq}</changefreq><priority>${item.priority}</priority></url>`,
    )
  }

  // 文章详情页；列表接口不含 updated_at，lastmod 用发布时间
  for (const article of articles) {
    const lastmod = article.published_at ? article.published_at.slice(0, 10) : ''
    urls.push(
      `  <url><loc>${escapeXml(`${siteUrl}/articles/${article.slug}`)}</loc>${
        lastmod ? `<lastmod>${lastmod}</lastmod>` : ''
      }<changefreq>daily</changefreq><priority>0.8</priority></url>`,
    )
  }

  // 图库详情页
  for (const gallery of galleries) {
    urls.push(
      `  <url><loc>${escapeXml(`${siteUrl}/galleries/${gallery.slug}`)}</loc><changefreq>weekly</changefreq><priority>0.6</priority></url>`,
    )
  }

  // 分类与标签详情页；列表接口一次返回全量（含 slug），不可用时整组跳过（与下方导航同理）
  try {
    const [categories, tags] = await Promise.all([
      $fetch<PublicCategory[]>('/categories', { baseURL: publicApiBase() }),
      $fetch<PublicTag[]>('/tags', { baseURL: publicApiBase() }),
    ])
    for (const category of categories) {
      urls.push(
        `  <url><loc>${escapeXml(`${siteUrl}/categories/${category.slug}`)}</loc><changefreq>weekly</changefreq><priority>0.6</priority></url>`,
      )
    }
    for (const tag of tags) {
      urls.push(
        `  <url><loc>${escapeXml(`${siteUrl}/tags/${tag.slug}`)}</loc><changefreq>weekly</changefreq><priority>0.5</priority></url>`,
      )
    }
  } catch {
    // 分类/标签接口不可用时跳过详情页，不影响 sitemap 主体
  }

  // 自定义页面没有公开列表接口，从导航中收集 page 类型节点：
  // 未挂载到导航的页面没有公开入口，不进入 sitemap 也避免暴露孤立地址
  try {
    const navigation = await $fetch<PublicNavigationNode[]>('/navigation', {
      baseURL: publicApiBase(),
    })
    const pageHrefs = new Set<string>()
    const walk = (nodes: PublicNavigationNode[]) => {
      for (const node of nodes) {
        if (node.target_type === 'page' && node.href) pageHrefs.add(node.href)
        walk(node.children ?? [])
      }
    }
    walk(navigation)
    for (const href of pageHrefs) {
      urls.push(
        `  <url><loc>${escapeXml(siteUrl + href)}</loc><changefreq>monthly</changefreq><priority>0.4</priority></url>`,
      )
    }
  } catch {
    // 导航接口不可用时跳过自定义页面，不影响 sitemap 主体
  }

  setResponseHeader(event, 'content-type', 'application/xml; charset=utf-8')
  return `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.join('\n')}\n</urlset>\n`
})
