// Nitro 服务端访问后端 Public API 的公共封装（供 sitemap/rss/robots 等 Server Route 使用）
import type {
  PublicArticleListItem,
  PublicArticlePage,
  PublicCategory,
  PublicGalleryPage,
  PublicGallerySummary,
  PublicNavigationNode,
  PublicSite,
  PublicTag,
  PublicTaxonomyRef,
} from '@aries/api-client'

export type {
  PublicArticleListItem,
  PublicArticlePage,
  PublicCategory,
  PublicGalleryPage,
  PublicGallerySummary,
  PublicNavigationNode,
  PublicSite,
  PublicTag,
  PublicTaxonomyRef,
}

export function publicApiBase(): string {
  return useRuntimeConfig().internalApiBase as string
}

/** 手写 XML 转义，避免引入额外依赖 */
export function escapeXml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;')
}

export async function fetchPublicSite(): Promise<PublicSite | null> {
  try {
    return await $fetch<PublicSite>('/site', { baseURL: publicApiBase() })
  } catch {
    // 后端不可用时返回 null，由调用方走兜底
    return null
  }
}

/** 站点绝对地址：优先站点设置 site_url，其次 NUXT_PUBLIC_SITE_URL，最后回退到请求来源 */
export async function resolveSiteUrl(origin: string): Promise<string> {
  const config = useRuntimeConfig()
  const site = await fetchPublicSite()
  const raw = site?.site_url || (config.public.siteUrl as string) || origin
  return raw.replace(/\/+$/, '')
}
