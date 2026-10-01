// 站点导航：全站共享一份（useState 缓存），供 Header 与页脚使用。
// 导航属于增强信息而非页面主体，请求失败时兜底为空数组（由布局回退默认链接），不得拖垮整站渲染
import type { Ref } from 'vue'
import type { PublicNavigationNode } from './usePublicApi'

export async function useNavigation(): Promise<Ref<PublicNavigationNode[]>> {
  const nodes = useState<PublicNavigationNode[] | null>('public-navigation', () => null)
  if (!nodes.value) {
    const config = useRuntimeConfig()
    // 与 useSite 同理：SSR 直连后端，浏览器端走同源代理（后端 CORS 只放行 Admin 来源）
    const baseURL = import.meta.server ? (config.internalApiBase as string) : '/api/public'
    try {
      nodes.value = await $fetch<PublicNavigationNode[]>('/navigation', { baseURL })
    } catch {
      nodes.value = []
    }
  }
  return nodes as Ref<PublicNavigationNode[]>
}
