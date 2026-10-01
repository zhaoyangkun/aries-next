// robots.txt：放行公开页面，屏蔽 API，声明 Sitemap 绝对地址
export default defineEventHandler(async (event) => {
  const siteUrl = await resolveSiteUrl(getRequestURL(event).origin)
  setResponseHeader(event, 'content-type', 'text/plain; charset=utf-8')
  return `User-agent: *
Allow: /
Disallow: /api/
Disallow: /search

Sitemap: ${siteUrl}/sitemap.xml
`
})
