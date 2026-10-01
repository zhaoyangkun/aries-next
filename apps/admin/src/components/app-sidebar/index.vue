<script setup lang="ts">
import { Sidebar, SidebarContent, SidebarFooter, SidebarHeader, SidebarMenu, SidebarMenuButton, SidebarMenuItem, SidebarRail } from '@/components/ui/sidebar'
import { navData } from '@/constants/sidebar-data'
import { useSessionStore } from '@/modules/auth/stores/session'

import type { NavGroup, NavItem } from './types'

import NavFooter from './nav-footer.vue'
import NavTeam from './nav-team.vue'

const session = useSessionStore()

// 按权限过滤导航项；空分组整体隐藏。授权以后端 Permission Guard 为准，这里只做入口隐藏。
const visibleGroups = computed<NavGroup[]>(() =>
  navData
    .map((group) => {
      const items: NavItem[] = group.items.flatMap((item): NavItem[] => {
        if (item.permission && !session.hasPermission(item.permission))
          return []
        if (!item.items)
          return [item]
        const visibleChildren = item.items.filter(child =>
          !child.permission || session.hasPermission(child.permission),
        )
        return visibleChildren.length > 0 ? [{ ...item, items: visibleChildren }] : []
      })
      return { ...group, items }
    })
    .filter(group => group.items.length > 0),
)

const user = computed(() => ({
  name: session.user?.display_name || session.user?.username || '管理员',
  email: session.user?.email ?? '',
  avatar: session.user?.avatar_url ?? '',
}))
</script>

<template>
  <Sidebar collapsible="icon" class="z-50">
    <SidebarHeader class="border-b h-14 py-1 group-data-[collapsible=icon]:justify-center">
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton size="lg" as-child tooltip="Aries 内容管理">
            <RouterLink to="/">
              <span class="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary text-sm font-bold text-primary-foreground">A</span>
              <span class="grid min-w-0 flex-1 text-left leading-tight">
                <span class="truncate text-sm font-semibold">Aries</span>
                <span class="truncate text-[11px] text-muted-foreground">内容管理</span>
              </span>
            </RouterLink>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
    </SidebarHeader>

    <SidebarContent>
      <NavTeam :nav-main="visibleGroups" />
    </SidebarContent>

    <SidebarFooter class="border-t h-14 py-1 group-data-[collapsible=icon]:justify-center">
      <NavFooter :user="user" />
    </SidebarFooter>

    <SidebarRail />
  </Sidebar>
</template>
