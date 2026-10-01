import type { LucideProps } from '@lucide/vue'
import type { FunctionalComponent } from 'vue'

import type { PermissionName } from '@/modules/auth/api/auth'

type NavIcon = FunctionalComponent<LucideProps, Record<any, any>, any, Record<any, any>>

interface BaseNavItem {
  title: string
  icon?: NavIcon
  /** 需要的权限；缺省表示所有登录用户可见。 */
  permission?: PermissionName
}

export type NavItem
  = | BaseNavItem & {
    items: (BaseNavItem & { url?: string })[]
    url?: never
    isActive?: boolean
  } | BaseNavItem & {
    url: string
    items?: never
  }

export interface NavGroup {
  title: string
  items: NavItem[]
}

export interface User {
  name: string
  avatar: string
  email: string
}
