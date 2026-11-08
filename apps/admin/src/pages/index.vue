<script setup lang="ts">
/** 后台首页：默认重定向到仪表盘。 */
import { useRouter } from 'vue-router'

import Loading from '@/components/loading.vue'

const router = useRouter()
router.push({ name: '/dashboard/' })
</script>

<template>
  <div class="flex items-center justify-center w-screen h-screen">
    <Loading />
  </div>
</template>

<route lang="yaml">
meta:
  layout: false
</route>
