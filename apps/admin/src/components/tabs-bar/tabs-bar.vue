<script setup lang="ts">
import { ChevronDownIcon, XIcon } from '@lucide/vue'
import { storeToRefs } from 'pinia'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useTabsStore, type PageTab } from '@/stores/tabs'
import { navData } from '@/constants/sidebar-data'

import type { NavGroup } from '../app-sidebar/types'

const route = useRoute()
const router = useRouter()
const tabsStore = useTabsStore()

const { tabs } = storeToRefs(tabsStore)

// 标题解析顺序：路由 meta.title → 导航配置（含设置子页）→ 兜底。
const links = (() => {
  const flat: { title: string, url: string }[] = []
  const walk = (groups: NavGroup[]) =>
    groups.forEach(group => group.items.forEach((item) => {
      if (item.items)
        walk([item as unknown as NavGroup])
      else if (item.url)
        flat.push({ title: item.title, url: item.url })
    }))
  walk(navData)
  return flat
})()

function resolveTitle(path: string): string {
  const metaTitle = route.meta.title
  if (typeof metaTitle === 'string' && metaTitle)
    return metaTitle
  const normalize = (url: string) => url.replace(/\/+$/, '')
  const exact = links.find(link => normalize(link.url) === normalize(path))
  if (exact)
    return exact.title
  const candidates = links.filter(link => link.url !== '/dashboard' && path.startsWith(`${normalize(link.url)}/`))
  if (candidates.length)
    return candidates.sort((a, b) => b.url.length - a.url.length)[0].title
  return '页面'
}

// 访问即开标签；概览固定不可关闭。
watch(() => route.fullPath, () => {
  tabsStore.open({
    path: route.path,
    title: resolveTitle(route.path),
    closable: route.path !== '/dashboard',
  })
}, { immediate: true })

function isActive(tab: PageTab) {
  return route.path === tab.path
}

function go(tab: PageTab) {
  if (!isActive(tab))
    router.push(tab.path)
}

function closeTab(tab: PageTab) {
  const next = tabsStore.close(tab.path)
  // 关闭的是当前页时跳到相邻标签；无标签可去时回概览。
  if (isActive(tab))
    router.push(next ?? '/dashboard')
}

function closeOthers() {
  tabsStore.closeOthers(route.path)
  if (!tabsStore.tabs.some(tab => tab.path === route.path))
    router.push('/dashboard')
}

function closeAll() {
  tabsStore.closeAll()
  if (!tabsStore.tabs.some(tab => tab.path === route.path))
    router.push('/dashboard')
}

const otherClosable = computed(() => tabs.value.some(tab => tab.closable && tab.path !== route.path))
const anyClosable = computed(() => tabs.value.some(tab => tab.closable))
</script>

<template>
  <div class="flex h-10 shrink-0 items-center gap-1 border-b bg-background px-3" aria-label="页面标签">
    <div class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto" role="tablist">
      <div
        v-for="tab in tabs"
        :key="tab.path"
        role="tab"
        :aria-selected="isActive(tab)"
        class="group flex h-7 shrink-0 cursor-pointer items-center gap-1 rounded-md border px-2.5 text-xs transition-colors select-none"
        :class="isActive(tab)
          ? 'border-border bg-accent font-medium text-accent-foreground'
          : 'border-transparent text-muted-foreground hover:bg-accent/50 hover:text-foreground'"
        @click="go(tab)"
        @auxclick.middle.prevent="tab.closable && closeTab(tab)"
      >
        <span class="max-w-32 truncate">{{ tab.title }}</span>
        <button
          v-if="tab.closable"
          type="button"
          class="hover:bg-background/80 -mr-1 rounded-sm p-0.5 opacity-0 transition-opacity group-hover:opacity-100"
          :class="isActive(tab) ? 'opacity-60 hover:opacity-100' : ''"
          :aria-label="`关闭标签：${tab.title}`"
          @click.stop="closeTab(tab)"
        >
          <XIcon class="size-3" />
        </button>
      </div>
    </div>

    <DropdownMenu>
      <DropdownMenuTrigger as-child>
        <Button variant="ghost" size="icon-sm" class="size-7 shrink-0" aria-label="标签操作">
          <ChevronDownIcon />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" class="w-36">
        <DropdownMenuItem :disabled="!otherClosable" @click="closeOthers">
          关闭其他标签
        </DropdownMenuItem>
        <DropdownMenuItem :disabled="!anyClosable" @click="closeAll">
          关闭全部标签
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  </div>
</template>
