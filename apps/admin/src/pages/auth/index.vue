<script setup lang="ts">
/** 认证入口：已登录跳回首页，未登录跳登录页。 */
const router = useRouter()
router.replace('/auth/sign-in')
</script>

<template>
  <div />
</template>
