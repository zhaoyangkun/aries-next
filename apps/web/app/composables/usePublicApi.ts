// 与 docs/openapi.yaml 的 Public* Schema 一一对应的前端类型（由 @aries/api-client 从 openapi.yaml 生成），
// 以及统一的数据获取封装
import type {
  CreatePublicCommentRequest,
  PublicArchiveMonth,
  PublicArticleDetail,
  PublicArticleListItem,
  PublicArticlePage,
  PublicCategory,
  PublicComment,
  PublicCommentCreated,
  PublicCommentPage,
  PublicGalleryDetail,
  PublicGalleryItem,
  PublicGalleryPage,
  PublicGallerySummary,
  PublicJournalItem,
  PublicJournalPage,
  PublicLink,
  PublicNavigationNode,
  PublicPageDetail,
  PublicPhoto,
  PublicSearchSuggestion,
  PublicSite,
  PublicTag,
  PublicTaxonomyRef,
  PublicArticleNeighbor,
  ReplyPublicCommentRequest,
} from '@aries/api-client'
import type { Ref, WatchSource } from 'vue'

export type {
  CreatePublicCommentRequest as CreateCommentPayload,
  PublicArchiveMonth,
  PublicArticleDetail,
  PublicArticleListItem,
  PublicArticlePage,
  PublicCategory,
  PublicComment,
  PublicCommentCreated,
  PublicCommentPage,
  PublicGalleryDetail,
  PublicGalleryItem,
  PublicGalleryPage,
  PublicGallerySummary,
  PublicJournalItem,
  PublicJournalPage,
  PublicLink,
  PublicNavigationNode,
  PublicPageDetail,
  PublicPhoto,
  PublicSearchSuggestion,
  PublicSite,
  PublicTag,
  PublicTaxonomyRef,
  PublicArticleNeighbor,
  ReplyPublicCommentRequest as ReplyCommentPayload,
}

type QueryValue = string | number | undefined | null
type QueryInput = Record<string, QueryValue | Ref<QueryValue>>

function resolveBaseURL(): string {
  const config = useRuntimeConfig()
  // SSR 期间服务端直连后端（私有 runtimeConfig，容器内网地址运行期注入）；
  // 浏览器端走同源 /api（生产由 Caddy 转发到后端，开发由 nitro devProxy 转发），
  // 因为后端 CORS 只放行 Admin 来源，跨端口直连会被浏览器拦截
  return import.meta.server ? (config.internalApiBase as string) : '/api/public'
}

function resolveQuery(query?: QueryInput): Record<string, QueryValue> | undefined {
  if (!query) return undefined
  const resolved: Record<string, QueryValue> = {}
  for (const [key, value] of Object.entries(query)) {
    const raw = unref(value)
    if (raw !== undefined && raw !== null && raw !== '') resolved[key] = raw
  }
  return resolved
}

interface UsePublicApiOptions {
  query?: QueryInput
  watch?: WatchSource[]
  /** SSR 时透传浏览器 Cookie（密码文章解锁后再次渲染详情时需要） */
  forwardCookies?: boolean
  /** 条件请求：为 false 时跳过请求，data 为 null（如搜索词为空） */
  enabled?: Ref<boolean>
  /** 请求失败时不抛错（不落入 error.vue），错误暴露在返回值 error 中由调用方兜底 */
  ignoreError?: boolean
}

/** SSR 数据获取封装；请求失败时抛出 Nuxt 错误，由 error.vue 统一承接 */
export async function usePublicApi<T>(
  key: string,
  path: string | (() => string),
  options: UsePublicApiOptions = {},
) {
  const headers =
    options.forwardCookies && import.meta.server ? useRequestHeaders(['cookie']) : undefined

  const { data, status, error, refresh } = await useAsyncData<T | null>(
    key,
    () => {
      if (options.enabled && !options.enabled.value) return Promise.resolve(null)
      const resolvedPath = typeof path === 'function' ? path() : path
      return $fetch(resolvedPath, {
        baseURL: resolveBaseURL(),
        query: resolveQuery(options.query),
        headers,
      }) as Promise<T>
    },
    { watch: options.watch ?? [] },
  )

  if (error.value && !options.ignoreError) {
    const statusCode = (error.value as { statusCode?: number }).statusCode ?? 500
    throw createError({ statusCode, message: error.value.message, fatal: true })
  }

  return { data: data as Ref<T | null>, status, error, refresh }
}

/** 浏览器端写请求（解锁密码、上报浏览量）：同源代理 + 携带 HttpOnly Cookie */
export async function postPublicApi<T>(path: string, body?: Record<string, unknown>): Promise<T> {
  return (await $fetch(path, {
    baseURL: '/api/public',
    method: 'POST',
    body,
    credentials: 'include',
  })) as T
}
