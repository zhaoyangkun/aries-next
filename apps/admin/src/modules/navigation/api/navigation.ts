import { api } from '@/shared/api/client'

export type NavigationTargetType = 'article' | 'page' | 'category' | 'url'

export interface NavigationItem {
  id: number
  parent_id: number | null
  label: string
  target_type: NavigationTargetType
  target_id: number | null
  url: string | null
  open_in_new_tab: boolean
  visible: boolean
  sort_order: number
  created_at: string
  updated_at: string
}

export interface UpsertNavigationPayload {
  parent_id?: number | null
  label: string
  target_type: NavigationTargetType
  target_id?: number | null
  url?: string | null
  open_in_new_tab?: boolean
  visible?: boolean
  sort_order?: number
}

export const navigationApi = {
  // 平铺返回，前端按 parent_id 组装两级树；Backend 限制最多两级。
  async list() {
    const { data } = await api.get<NavigationItem[]>('/api/admin/navigation')
    return data
  },

  async create(payload: UpsertNavigationPayload) {
    const { data } = await api.post<NavigationItem>('/api/admin/navigation', payload)
    return data
  },

  async update(itemId: number, payload: UpsertNavigationPayload) {
    const { data } = await api.put<NavigationItem>(`/api/admin/navigation/${itemId}`, payload)
    return data
  },

  async remove(itemId: number) {
    await api.delete(`/api/admin/navigation/${itemId}`)
  },

  // 原子批量排序：item_ids 必须覆盖全部节点（父子按展示顺序平铺）。
  async reorder(itemIds: number[]) {
    await api.put('/api/admin/navigation/order', { item_ids: itemIds })
  },
}
