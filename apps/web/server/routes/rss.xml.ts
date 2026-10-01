// RSS 2.0 订阅源：最新 20 篇已发布文章，全部使用绝对 URL
export default defineEventHandler(async (event) => {
  const siteUrl = await resolveSiteUrl(getRequestURL(event).origin)
  const site = await fetchPublicSite()
  const res = await $fetch<PublicArticlePage>('/articles', {
    baseURL: publicApiBase(),
    query: { page: 1, page_size: 20 },
  })

  const items = res.items
    .map((article) => {
      const link = `${siteUrl}/articles/${article.slug}`
      const pubDate = article.published_at ? new Date(article.published_at).toUTCString() : ''
      return [
        '    <item>',
        `      <title>${escapeXml(article.title)}</title>`,
        `      <link>${escapeXml(link)}</link>`,
        `      <guid isPermaLink="true">${escapeXml(link)}</guid>`,
        pubDate ? `      <pubDate>${pubDate}</pubDate>` : '',
        article.summary ? `      <description>${escapeXml(article.summary)}</description>` : '',
        '    </item>',
      ]
        .filter(Boolean)
        .join('\n')
    })
    .join('\n')

  const channelTitle = site?.site_name || 'Aries'
  const channelDescription = site?.site_description || ''

  setResponseHeader(event, 'content-type', 'application/rss+xml; charset=utf-8')
  return `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>${escapeXml(channelTitle)}</title>
    <link>${escapeXml(siteUrl)}</link>
    <description>${escapeXml(channelDescription)}</description>
    <language>zh-CN</language>
${items}
  </channel>
</rss>
`
})
