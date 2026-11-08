<script setup lang="ts">
/** 404 兜底路由：未匹配路径渲染自定义错误页。 */
import Error from '@/components/custom-error.vue'
</script>

<template>
  <div class="flex items-center justify-center h-screen">
    <Error
      :code="404"
      subtitle="Page Not Found"
      error="The page you are looking for might have been removed, had its name changed, or is temporarily unavailable."
    />
  </div>
</template>

<route lang="yaml">
meta:
  layout: false
  public: true
</route>
