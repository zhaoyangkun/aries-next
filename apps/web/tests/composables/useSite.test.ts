import { afterEach, describe, expect, it, vi } from 'vitest'

import { toAbsoluteUrl, useSite } from '../../app/composables/useSite'
import { stubRuntimeConfig, stubUseState } from '../helpers/nuxt-globals'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('toAbsoluteUrl', () => {
  it('should return undefined for null / undefined / empty input', () => {
    expect(toAbsoluteUrl(null, 'https://example.com')).toBeUndefined()
    expect(toAbsoluteUrl(undefined, 'https://example.com')).toBeUndefined()
    expect(toAbsoluteUrl('', 'https://example.com')).toBeUndefined()
  })

  it('should keep absolute http(s) URLs unchanged', () => {
    expect(toAbsoluteUrl('https://cdn.example.com/a.png', 'https://example.com')).toBe(
      'https://cdn.example.com/a.png',
    )
    expect(toAbsoluteUrl('http://example.com/a.png', 'https://blog.example.com')).toBe(
      'http://example.com/a.png',
    )
  })

  it('should resolve relative media URLs against the site URL', () => {
    expect(toAbsoluteUrl('/api/media/files/a.png', 'https://example.com')).toBe(
      'https://example.com/api/media/files/a.png',
    )
  })

  it('should return undefined for relative URLs when site URL is empty', () => {
    expect(toAbsoluteUrl('/api/media/files/a.png', '')).toBeUndefined()
  })
})

describe('useSite', () => {
  it('should fetch and return the public site settings', async () => {
    stubUseState()
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    const site = {
      site_name: 'Aries Blog',
      site_description: 'desc',
      site_url: 'https://example.com',
      logo_url: '',
      icp_text: '',
      default_cover_url: '',
    }
    const fetchMock = vi.fn().mockResolvedValue(site)
    vi.stubGlobal('$fetch', fetchMock)

    const result = await useSite()

    expect(result.value).toEqual(site)
    // vitest 下 import.meta.server 为假，走浏览器端的同源代理分支
    expect(fetchMock).toHaveBeenCalledWith('/site', { baseURL: '/api/public' })
  })

  it('should fall back to default site when the backend is unavailable', async () => {
    stubUseState()
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    vi.stubGlobal('$fetch', vi.fn().mockRejectedValue(new Error('connection refused')))

    const result = await useSite()

    // 兜底值必须保证整站可渲染：站点名固定为 Aries，其余字段为空串
    expect(result.value).toEqual({
      site_name: 'Aries',
      site_description: '',
      site_url: '',
      logo_url: '',
      icp_text: '',
      default_cover_url: '',
    })
  })

  it('should cache the site in useState and not refetch on second call', async () => {
    stubUseState()
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    const fetchMock = vi.fn().mockResolvedValue({
      site_name: 'Aries Blog',
      site_description: '',
      site_url: '',
      logo_url: '',
      icp_text: '',
      default_cover_url: '',
    })
    vi.stubGlobal('$fetch', fetchMock)

    await useSite()
    const second = await useSite()

    expect(fetchMock).toHaveBeenCalledTimes(1)
    expect(second.value.site_name).toBe('Aries Blog')
  })
})
