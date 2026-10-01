// 标签表：全站共享一份（useState 缓存），供列表卡片把 tag_ids 解析为名称/Slug，
// 以及标签页按 Slug 反查标签。标签属于辅助数据，请求失败时兜底为空数组（卡片只缺标签 chips），
// 不得拖垮整页渲染；与 useSite / useNavigation 同理：SSR 直连后端，浏览器端走同源代理
import type { Ref } from 'vue'
import type { PublicTag, PublicTaxonomyRef } from './usePublicApi'

export async function useTags(): Promise<{
  tags: Ref<PublicTag[]>
  resolveTags: (tagIds: number[]) => PublicTaxonomyRef[]
}> {
  const tags = useState<PublicTag[] | null>('public-tags', () => null)
  if (!tags.value) {
    const config = useRuntimeConfig()
    const baseURL = import.meta.server ? (config.internalApiBase as string) : '/api/public'
    try {
      tags.value = await $fetch<PublicTag[]>('/tags', { baseURL })
    } catch {
      tags.value = []
    }
  }
  const tagMap = computed(() => {
    const map = new Map<number, PublicTag>()
    for (const t of tags.value ?? []) map.set(t.id, t)
    return map
  })

  function resolveTags(tagIds: number[]): PublicTaxonomyRef[] {
    return tagIds
      .map((id) => tagMap.value.get(id))
      .filter((t): t is PublicTag => !!t)
  }

  return { tags: tags as Ref<PublicTag[]>, resolveTags }
}
