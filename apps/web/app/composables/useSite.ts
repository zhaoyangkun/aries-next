// 站点公开设置：全站共享一份（useState 缓存），供导航、页脚与 SEO 默认值使用
import type { MaybeRefOrGetter, Ref } from 'vue'
import type { PublicSite } from './usePublicApi'

const FALLBACK_SITE: PublicSite = {
  site_name: 'Aries',
  site_description: '',
  site_url: '',
  logo_url: '',
  icp_text: '',
  default_cover_url: '',
  // 后端暂不可用时没有建站时间，空串让 footer「已运行 X 天」按缺失处理（隐藏）
  created_at: '',
}

export async function useSite(): Promise<Ref<PublicSite>> {
  const site = useState<PublicSite | null>('public-site', () => null)
  if (!site.value) {
    const config = useRuntimeConfig()
    // 与 usePublicApi 同理：SSR 直连后端（私有 runtimeConfig），浏览器端走同源代理
    const baseURL = import.meta.server ? (config.internalApiBase as string) : '/api/public'
    try {
      site.value = await $fetch<PublicSite>('/site', { baseURL })
    } catch {
      // 后端暂不可用时使用兜底值，避免整站渲染失败
      site.value = FALLBACK_SITE
    }
  }
  return site as Ref<PublicSite>
}

/** Canonical 站点的绝对地址：优先站点设置 site_url，其次 NUXT_PUBLIC_SITE_URL */
export function useSiteUrl(): Ref<string> {
  const site = useState<PublicSite | null>('public-site')
  const config = useRuntimeConfig()
  return computed(() => site.value?.site_url || (config.public.siteUrl as string) || '')
}

/** 为当前页面写入 canonical link；分页页传自身地址以避免重复内容 */
export function useCanonical(path: Ref<string> | string) {
  const siteUrl = useSiteUrl()
  useHead(
    computed(() => {
      if (!siteUrl.value) return {}
      return { link: [{ rel: 'canonical', href: new URL(unref(path), siteUrl.value).toString() }] }
    }),
  )
}

interface PageSeoInput {
  title: MaybeRefOrGetter<string>
  description?: MaybeRefOrGetter<string | undefined>
}

/** 列表页/静态页的 SEO 兜底：description 缺省回退站点简介，og:image 取站点默认封面（绝对地址）。
 *  数据详情的页面（如文章页）请自行组织 useSeoMeta，不用此兜底。 */
export function usePageSeo(input: PageSeoInput) {
  const site = useState<PublicSite | null>('public-site')
  const siteUrl = useSiteUrl()
  useSeoMeta({
    title: () => toValue(input.title),
    description: () => toValue(input.description) || site.value?.site_description,
    ogTitle: () => toValue(input.title),
    ogType: 'website',
    ogImage: () => toAbsoluteUrl(site.value?.default_cover_url, siteUrl.value),
  })
}

/** 把后端返回的相对媒体 URL 转为绝对地址（og:image 等必须绝对地址的场景） */
export function toAbsoluteUrl(url: string | null | undefined, siteUrl: string): string | undefined {
  if (!url) return undefined
  if (/^https?:\/\//.test(url)) return url
  if (!siteUrl) return undefined
  return new URL(url, siteUrl).toString()
}
