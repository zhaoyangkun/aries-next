// 判断同源 href 是否命中 Nuxt 已注册的路由。命中才用 NuxtLink 做 SPA 跳转
// （保留客户端导航与 router-link-active 激活态）；未命中的同源路径（如 /admin，
// 由后端/Caddy 处理）退化为原生 <a>，避免 vue-router 在开发期对未知路径
// 反复刷 "No match found for location" 警告（VUE_ROUTER_R0004）。
//
// 刻意不用 router.resolve() 判断：它对未命中路径同样会刷该警告。
export function isRegisteredRoute(
  routes: readonly { path: string }[],
  href: string,
): boolean {
  const [rawPath = ''] = href.split(/[?#]/)
  const path = rawPath.replace(/\/+$/, '') || '/'
  return routes.some((route) => pathMatches(route.path, path))
}

function pathMatches(pattern: string, path: string): boolean {
  const patternSegments = pattern.split('/').filter(Boolean)
  const pathSegments = path.split('/').filter(Boolean)
  if (patternSegments.length !== pathSegments.length) return false
  return patternSegments.every((segment, index) => {
    const pathSegment = pathSegments[index]
    return pathSegment !== undefined && (segment.startsWith(':') || segment === pathSegment)
  })
}
