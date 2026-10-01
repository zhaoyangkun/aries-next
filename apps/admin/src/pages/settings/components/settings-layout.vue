<script setup lang="ts">
import { BasicPage } from '@/components/global-layout'
import { settingsNavItems } from '@/constants/sidebar-data'
import { useSessionStore } from '@/modules/auth/stores/session'

withDefaults(defineProps<{
  title: string
  description?: string
}>(), {
  description: '',
})

const route = useRoute()
const session = useSessionStore()

// 子导航同样按权限过滤；站点设置仅 settings:manage 可见。
const items = computed(() =>
  settingsNavItems.filter(item => !item.permission || session.hasPermission(item.permission)),
)

function isActive(url: string) {
  if (url === '/settings/')
    return route.path === '/settings' || route.path === '/settings/'
  return route.path.startsWith(url)
}
</script>

<template>
  <BasicPage :title="title" :description="description" sticky>
    <div class="flex flex-col gap-6 lg:flex-row">
      <aside class="shrink-0 lg:w-48">
        <nav class="flex gap-1 overflow-x-auto lg:flex-col" aria-label="设置导航">
          <RouterLink
            v-for="item in items"
            :key="item.url"
            :to="item.url"
            class="flex items-center gap-2 rounded-md px-3 py-2 text-sm whitespace-nowrap transition-colors"
            :class="isActive(item.url)
              ? 'bg-accent font-medium text-accent-foreground'
              : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'"
          >
            <component :is="item.icon" v-if="item.icon" class="size-4" />
            {{ item.title }}
          </RouterLink>
        </nav>
      </aside>
      <div class="min-w-0 flex-1">
        <slot />
      </div>
    </div>
  </BasicPage>
</template>
