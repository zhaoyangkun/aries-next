import { afterEach, describe, expect, it, vi } from 'vitest'
import { ref, unref } from 'vue'

import { postPublicApi, usePublicApi } from '../../app/composables/usePublicApi'
import { stubRuntimeConfig } from '../helpers/nuxt-globals'

// useAsyncData 替身：直接执行 handler，把结果/异常包装成与 Nuxt 相同的 { data, status, error, refresh } 形状
function stubUseAsyncData() {
  vi.stubGlobal('useAsyncData', async (_key: string, handler: () => Promise<unknown>) => {
    try {
      const data = await handler()
      return {
        data: { value: data },
        status: { value: 'success' },
        error: { value: null },
        refresh: vi.fn(),
      }
    }
    catch (e) {
      return {
        data: { value: null },
        status: { value: 'error' },
        error: { value: e },
        refresh: vi.fn(),
      }
    }
  })
}

// createError 替身：构造携带 statusCode 的 Error，便于断言抛出的 Nuxt 错误
function stubCreateError() {
  vi.stubGlobal('createError', (input: { statusCode?: number, message?: string }) =>
    Object.assign(new Error(input.message), { statusCode: input.statusCode }),
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('usePublicApi', () => {
  it('should fetch data and filter out empty query values', async () => {
    // resolveQuery 内部用的是 Nuxt 自动导入的 unref，这里替换为 Vue 的真实实现
    vi.stubGlobal('unref', unref)
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    stubUseAsyncData()
    const fetchMock = vi.fn().mockResolvedValue({ items: [], total: 0 })
    vi.stubGlobal('$fetch', fetchMock)

    const { data, status } = await usePublicApi('articles', '/articles', {
      query: { page: 1, q: ref('vue'), empty: '', nil: null, missing: undefined },
    })

    expect(data.value).toEqual({ items: [], total: 0 })
    expect(status.value).toBe('success')
    // vitest 下 import.meta.server 为假，走浏览器端的同源代理分支；
    // 空串 / null / undefined 的查询参数必须被过滤，ref 需解包
    expect(fetchMock).toHaveBeenCalledWith('/articles', {
      baseURL: '/api/public',
      query: { page: 1, q: 'vue' },
      headers: undefined,
    })
  })

  it('should support a function path and pass undefined query when none given', async () => {
    vi.stubGlobal('unref', unref)
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    stubUseAsyncData()
    const fetchMock = vi.fn().mockResolvedValue({ slug: 'hello' })
    vi.stubGlobal('$fetch', fetchMock)

    const { data } = await usePublicApi('article', () => '/articles/hello')

    expect(data.value).toEqual({ slug: 'hello' })
    expect(fetchMock).toHaveBeenCalledWith('/articles/hello', {
      baseURL: '/api/public',
      query: undefined,
      headers: undefined,
    })
  })

  it('should skip the request and return null when enabled is false', async () => {
    vi.stubGlobal('unref', unref)
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    stubUseAsyncData()
    const fetchMock = vi.fn()
    vi.stubGlobal('$fetch', fetchMock)

    const { data } = await usePublicApi('search', '/search', { enabled: ref(false) })

    expect(data.value).toBeNull()
    expect(fetchMock).not.toHaveBeenCalled()
  })

  it('should throw a Nuxt error with the backend status code on failure', async () => {
    vi.stubGlobal('unref', unref)
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    stubUseAsyncData()
    stubCreateError()
    vi.stubGlobal(
      '$fetch',
      vi.fn().mockRejectedValue({ statusCode: 404, message: 'Not Found' }),
    )

    await expect(usePublicApi('article', '/articles/missing')).rejects.toMatchObject({
      statusCode: 404,
      message: 'Not Found',
    })
  })

  it('should expose the error without throwing when ignoreError is set', async () => {
    vi.stubGlobal('unref', unref)
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    stubUseAsyncData()
    stubCreateError()
    vi.stubGlobal(
      '$fetch',
      vi.fn().mockRejectedValue({ statusCode: 404, message: 'Not Found' }),
    )

    const { data, error } = await usePublicApi('about-page', '/pages/about', {
      ignoreError: true,
    })

    expect(data.value).toBeNull()
    expect((error.value as { statusCode?: number }).statusCode).toBe(404)
  })

  it('should default the error status code to 500 when the backend error has none', async () => {
    vi.stubGlobal('unref', unref)
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    stubUseAsyncData()
    stubCreateError()
    vi.stubGlobal('$fetch', vi.fn().mockRejectedValue(new Error('network down')))

    await expect(usePublicApi('articles', '/articles')).rejects.toMatchObject({
      statusCode: 500,
    })
  })
})

describe('postPublicApi', () => {
  it('should post through the same-origin proxy with cookies included', async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true })
    vi.stubGlobal('$fetch', fetchMock)

    const result = await postPublicApi('/articles/hello/unlock', { password: 'secret' })

    expect(result).toEqual({ ok: true })
    // 浏览器端写请求固定走 /api/public 同源代理并携带 HttpOnly Cookie
    expect(fetchMock).toHaveBeenCalledWith('/articles/hello/unlock', {
      baseURL: '/api/public',
      method: 'POST',
      body: { password: 'secret' },
      credentials: 'include',
    })
  })

  it('should allow omitting the request body', async () => {
    const fetchMock = vi.fn().mockResolvedValue(null)
    vi.stubGlobal('$fetch', fetchMock)

    await postPublicApi('/articles/hello/visit')

    expect(fetchMock).toHaveBeenCalledWith('/articles/hello/visit', {
      baseURL: '/api/public',
      method: 'POST',
      body: undefined,
      credentials: 'include',
    })
  })
})
