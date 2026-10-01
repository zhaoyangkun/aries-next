<script setup lang="ts">
import { useCookies } from '@vueuse/integrations/useCookies'
import { ExternalLinkIcon } from '@lucide/vue'
import { storeToRefs } from 'pinia'

import AppSidebar from '@/components/app-sidebar/index.vue'
import CommandMenuPanel from '@/components/command-menu-panel/index.vue'
import ThemePopover from '@/components/custom-theme/theme-popover.vue'
import TabsBar from '@/components/tabs-bar/tabs-bar.vue'
import ToggleTheme from '@/components/toggle-theme.vue'
import { Button } from '@/components/ui/button'
import { Separator } from '@/components/ui/separator'
import { SidebarInset, SidebarProvider, SidebarTrigger } from '@/components/ui/sidebar'
import { SIDEBAR_COOKIE_NAME } from '@/components/ui/sidebar/utils'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { usePublicSiteUrl } from '@/composables/use-public-site-url'
import { cn } from '@/lib/utils'
import { useThemeStore } from '@/stores/theme'

const defaultOpen = useCookies([SIDEBAR_COOKIE_NAME])
const themeStore = useThemeStore()
const { contentLayout } = storeToRefs(themeStore)

// 「打开公开站」地址：运行期从站点设置 site_url 读取（见 composable），未配置时隐藏入口
const publicSiteUrl = usePublicSiteUrl()
</script>

<template>
  <SidebarProvider :default-open="defaultOpen.get(SIDEBAR_COOKIE_NAME)">
    <AppSidebar />
    <SidebarInset class="w-full max-w-full peer-data-[state=collapsed]:w-[calc(100%-var(--sidebar-width-icon)-1rem)] peer-data-[state=expanded]:w-[calc(100%-var(--sidebar-width))]">
      <header
        class="flex items-center gap-3 sm:gap-4 h-14 p-4 shrink-0 transition-[width,height] ease-linear border-b sticky top-0 bg-background/95 backdrop-blur z-20 supports-[backdrop-filter]:bg-background/80"
      >
        <SidebarTrigger class="-ml-1" />
        <Separator orientation="vertical" />
        <CommandMenuPanel />
        <div class="flex-1" />
        <div class="ml-auto flex items-center space-x-2">
          <Tooltip v-if="publicSiteUrl">
            <TooltipTrigger as-child>
              <Button variant="ghost" size="icon" as-child>
                <a :href="publicSiteUrl" target="_blank" rel="noreferrer">
                  <ExternalLinkIcon />
                  <span class="sr-only">打开公开站</span>
                </a>
              </Button>
            </TooltipTrigger>
            <TooltipContent>打开公开站</TooltipContent>
          </Tooltip>
          <ToggleTheme />
          <ThemePopover />
        </div>
      </header>

      <TabsBar />

      <main
        :class="cn(
          'p-4 grow relative',
          contentLayout === 'centered' ? 'container mx-auto ' : '',
        )"
      >
        <router-view />
      </main>
    </SidebarInset>
  </SidebarProvider>
</template>
