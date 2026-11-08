<script setup lang="ts">
/** 应用根组件：Suspense 包裹的路由出口、全局 Toaster（sonner）与系统主题初始化。 */
import Loading from '@/components/loading.vue'
import { Toaster } from '@/components/ui/sonner'
import { useSystemTheme } from '@/composables/use-system-theme'

useSystemTheme()
</script>

<template>
  <Suspense>
    <router-view v-slot="{ Component }">
      <component :is="Component" />
    </router-view>

    <template #fallback>
      <Loading />
    </template>
  </Suspense>
  <Toaster position="top-center" rich-colors close-button />
</template>
