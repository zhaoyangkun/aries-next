<script setup lang="ts">
import { CommandGroup, CommandItem } from '@/components/ui/command'
import { navData } from '@/constants/sidebar-data'
import { useSessionStore } from '@/modules/auth/stores/session'

import type { NavGroup, NavItem } from '../app-sidebar/types'

import CommandItemHasIcon from './command-item-has-icon.vue'

const emit = defineEmits<{
  click: []
}>()

const session = useSessionStore()

function getFlatNavItems(groups: NavGroup[]): NavItem[] {
  const flatItems: NavItem[] = []
  groups.forEach((group) => {
    group.items.forEach((item) => {
      if (item.items) {
        flatItems.push(...getFlatNavItems([item as unknown as NavGroup]))
      }
      else {
        flatItems.push(item)
      }
    })
  })
  return flatItems
}

// 与侧边栏同一数据源；按权限过滤，外部链接不进命令面板。
const commands = computed(() =>
  getFlatNavItems(navData).filter(item =>
    item.url
    && item.url.startsWith('/')
    && (!item.permission || session.hasPermission(item.permission)),
  ),
)

const router = useRouter()
const route = useRoute()
function commandItemClick(url: string) {
  emit('click')
  if (route.fullPath !== url) {
    router.push(url)
  }
}
</script>

<template>
  <CommandGroup heading="页面">
    <CommandItem
      v-for="command in commands"
      :key="command.title"
      :value="command.title"
      @click="commandItemClick(command.url!)"
    >
      <CommandItemHasIcon :name="command.title" :icon="command.icon" />
    </CommandItem>
  </CommandGroup>
</template>
