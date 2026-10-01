<script setup lang="ts">
// 全局错误页：404 与服务器错误差异化展示
import type { NuxtError } from '#app'

const props = defineProps<{ error: NuxtError }>()

// 错误页渲染在默认布局之外，需单独注入主题 init script 防闪烁
useTheme()

const is404 = computed(() => props.error.statusCode === 404)

useSeoMeta({
  title: () => (is404.value ? '页面不存在' : '出错了'),
  robots: 'noindex',
})

function goHome() {
  clearError({ redirect: '/' })
}
</script>

<template>
  <div class="grid min-h-screen place-items-center bg-background px-6 text-foreground">
    <div class="max-w-md text-center">
      <p class="m-0 text-6xl font-bold text-primary">{{ error.statusCode }}</p>
      <h1 class="mt-4 text-xl font-semibold">
        {{ is404 ? '页面不存在或已被移除' : '服务器出了点问题' }}
      </h1>
      <p class="mt-3 text-sm text-muted-foreground">
        {{ is404 ? '你访问的地址没有对应的内容，或许它已被移动或删除。' : '请稍后重试，若问题持续存在请联系站长。' }}
      </p>
      <button
        type="button"
        class="mt-8 cursor-pointer rounded-full bg-primary px-6 py-2 text-sm font-medium text-primary-foreground transition-opacity hover:opacity-90"
        @click="goHome"
      >返回首页</button>
    </div>
  </div>
</template>
