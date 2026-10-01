<script setup lang="ts">
// 兼容旧版 /login 书签：跳到新的登录页。
const router = useRouter()
const redirect = router.currentRoute.value.query.redirect as string
router.replace({ path: '/auth/sign-in', query: redirect ? { redirect } : {} })
</script>

<template>
  <div />
</template>

<route lang="yaml">
meta:
  layout: false
  public: true
</route>
