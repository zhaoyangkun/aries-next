import { afterEach, describe, expect, it, vi } from 'vitest'

import { siteSettingsApi, type SiteSettings } from '@/modules/settings/api/settings'
import { usePublicSiteUrl } from '../use-public-site-url'

vi.mock('@/modules/settings/api/settings', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/modules/settings/api/settings')>()
  return { ...actual, siteSettingsApi: { get: vi.fn() } }
})

describe('usePublicSiteUrl', () => {
  afterEach(() => {
    vi.clearAllMocks()
  })

  it('读取站点设置 site_url 作为公开站地址', async () => {
    vi.mocked(siteSettingsApi.get).mockResolvedValue({
      site_url: 'https://blog.example.com',
    } as SiteSettings)

    const url = usePublicSiteUrl()

    await vi.waitFor(() => expect(url.value).toBe('https://blog.example.com'))
  })

  it('site_url 为空时保持空串', async () => {
    vi.mocked(siteSettingsApi.get).mockResolvedValue({ site_url: '' } as SiteSettings)

    const url = usePublicSiteUrl()

    await vi.waitFor(() => expect(siteSettingsApi.get).toHaveBeenCalledTimes(1))
    expect(url.value).toBe('')
  })

  it('后端不可用时保持空串（调用方隐藏入口）', async () => {
    vi.mocked(siteSettingsApi.get).mockRejectedValue(new Error('network'))

    const url = usePublicSiteUrl()

    await vi.waitFor(() => expect(siteSettingsApi.get).toHaveBeenCalledTimes(1))
    expect(url.value).toBe('')
  })
})
