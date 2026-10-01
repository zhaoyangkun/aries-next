import { afterEach, describe, expect, it, vi } from 'vitest'

import { escapeXml, fetchPublicSite, resolveSiteUrl } from '../../server/utils/publicApi'
import { stubRuntimeConfig } from '../helpers/nuxt-globals'

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('escapeXml', () => {
  it('should escape all five XML special characters', () => {
    expect(escapeXml(`a & b <c> "d" 'e'`)).toBe('a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;')
  })

  it('should escape ampersand first so existing entities are not double-processed', () => {
    // 若顺序错误，&lt; 中的 & 会被再次转义成 &amp;lt;
    expect(escapeXml('&lt;')).toBe('&amp;lt;')
  })

  it('should leave plain text unchanged', () => {
    expect(escapeXml('你好，世界 123')).toBe('你好，世界 123')
  })

  it('should return empty string for empty input', () => {
    expect(escapeXml('')).toBe('')
  })
})

describe('fetchPublicSite', () => {
  it('should fetch the public site settings from the configured API base', async () => {
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    const site = { site_name: 'Aries', site_url: 'https://example.com' }
    const fetchMock = vi.fn().mockResolvedValue(site)
    vi.stubGlobal('$fetch', fetchMock)

    const result = await fetchPublicSite()

    expect(result).toEqual(site)
    expect(fetchMock).toHaveBeenCalledWith('/site', {
      baseURL: 'http://backend.test/api/public',
    })
  })

  it('should return null when the backend is unavailable', async () => {
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    vi.stubGlobal('$fetch', vi.fn().mockRejectedValue(new Error('connection refused')))

    await expect(fetchPublicSite()).resolves.toBeNull()
  })
})

describe('resolveSiteUrl', () => {
  it('should prefer site_url from site settings and strip trailing slashes', async () => {
    stubRuntimeConfig({ siteUrl: 'https://fallback.example.com' })
    vi.stubGlobal(
      '$fetch',
      vi.fn().mockResolvedValue({ site_name: 'Aries', site_url: 'https://example.com///' }),
    )

    await expect(resolveSiteUrl('https://origin.example.com')).resolves.toBe(
      'https://example.com',
    )
  })

  it('should fall back to NUXT_PUBLIC_SITE_URL when site settings have no site_url', async () => {
    stubRuntimeConfig({ siteUrl: 'https://fallback.example.com/' })
    // 后端返回空 site_url（ falsy ），走配置兜底
    vi.stubGlobal('$fetch', vi.fn().mockResolvedValue({ site_name: 'Aries', site_url: '' }))

    await expect(resolveSiteUrl('https://origin.example.com')).resolves.toBe(
      'https://fallback.example.com',
    )
  })

  it('should fall back to the request origin when the backend is unavailable and no config set', async () => {
    stubRuntimeConfig({ siteUrl: '' })
    vi.stubGlobal('$fetch', vi.fn().mockRejectedValue(new Error('connection refused')))

    await expect(resolveSiteUrl('https://origin.example.com/')).resolves.toBe(
      'https://origin.example.com',
    )
  })
})
