import { afterEach, describe, expect, it, vi } from 'vitest'

import { useNavigation } from '../../app/composables/useNavigation'
import { stubRuntimeConfig, stubUseState } from '../helpers/nuxt-globals'

afterEach(() => {
  vi.unstubAllGlobals()
})

const NAV_NODES = [
  {
    label: '首页',
    target_type: 'url',
    target_id: null,
    url: '/',
    href: '/',
    open_in_new_tab: false,
    children: [],
  },
]

describe('useNavigation', () => {
  it('should fetch and return navigation nodes', async () => {
    stubUseState()
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    const fetchMock = vi.fn().mockResolvedValue(NAV_NODES)
    vi.stubGlobal('$fetch', fetchMock)

    const result = await useNavigation()

    expect(result.value).toEqual(NAV_NODES)
    // vitest 下 import.meta.server 为假，走浏览器端的同源代理分支
    expect(fetchMock).toHaveBeenCalledWith('/navigation', { baseURL: '/api/public' })
  })

  it('should fall back to an empty array when the request fails', async () => {
    stubUseState()
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    vi.stubGlobal('$fetch', vi.fn().mockRejectedValue(new Error('connection refused')))

    const result = await useNavigation()

    // 导航是增强信息，失败兜底为空数组，由布局回退默认链接
    expect(result.value).toEqual([])
  })

  it('should cache nodes in useState and not refetch on second call', async () => {
    stubUseState()
    stubRuntimeConfig({}, { internalApiBase: 'http://backend.test/api/public' })
    const fetchMock = vi.fn().mockResolvedValue(NAV_NODES)
    vi.stubGlobal('$fetch', fetchMock)

    await useNavigation()
    await useNavigation()

    expect(fetchMock).toHaveBeenCalledTimes(1)
  })
})
