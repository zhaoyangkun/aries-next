import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { navigationApi, type NavigationItem } from './navigation'

const navItem: NavigationItem = {
  id: 1,
  parent_id: null,
  label: '首页',
  target_type: 'url',
  target_id: null,
  url: '/',
  open_in_new_tab: false,
  visible: true,
  sort_order: 0,
  created_at: '2026-08-05T12:00:00Z',
  updated_at: '2026-08-05T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('navigationApi', () => {
  it('lists navigation items as a flat list', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: [navItem] })

    await expect(navigationApi.list()).resolves.toEqual([navItem])
    expect(get).toHaveBeenCalledWith('/api/admin/navigation')
  })

  it('creates and updates a navigation item', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: navItem })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: navItem })
    const payload = {
      parent_id: null,
      label: '首页',
      target_type: 'url' as const,
      target_id: null,
      url: '/',
      open_in_new_tab: false,
      visible: true,
      sort_order: 0,
    }

    await expect(navigationApi.create(payload)).resolves.toEqual(navItem)
    await expect(navigationApi.update(1, payload)).resolves.toEqual(navItem)
    expect(post).toHaveBeenCalledWith('/api/admin/navigation', payload)
    expect(put).toHaveBeenCalledWith('/api/admin/navigation/1', payload)
  })

  it('sends the full id list for atomic reordering', async () => {
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: undefined })

    await expect(navigationApi.reorder([3, 1, 2])).resolves.toBeUndefined()
    expect(put).toHaveBeenCalledWith('/api/admin/navigation/order', { item_ids: [3, 1, 2] })
  })

  it('deletes a navigation item', async () => {
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(navigationApi.remove(1)).resolves.toBeUndefined()
    expect(del).toHaveBeenCalledWith('/api/admin/navigation/1')
  })
})
