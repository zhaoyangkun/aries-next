<script setup lang='ts'>
/** 侧边栏菜单按钮：激活态高亮、Tooltip 与外链标识。Props: isActive/tooltip/isExternalUrl/menu。 */
import { ExternalLinkIcon } from '@lucide/vue'

import { SidebarMenuButton } from '@/components/ui/sidebar'

import type { NavItem } from './types'

defineProps<{
  isActive: boolean
  tooltip?: string
  isExternalUrl?: boolean
  menu: NavItem
}>()
</script>

<template>
  <SidebarMenuButton
    as-child
    :is-active="isActive"
    :tooltip="tooltip"
  >
    <a v-if="isExternalUrl" :href="menu.url" target="_blank" rel="noopener noreferrer" class="flex items-center gap-2">
      <component :is="menu.icon" v-if="menu.icon" />
      <span>{{ menu.title }}</span>
      <ExternalLinkIcon class="w-4 h-4 ml-auto" />
    </a>

    <router-link v-else :to="menu.url!" class="flex items-center gap-2">
      <component :is="menu.icon" v-if="menu.icon" />
      <span>{{ menu.title }}</span>
    </router-link>
  </SidebarMenuButton>
</template>
