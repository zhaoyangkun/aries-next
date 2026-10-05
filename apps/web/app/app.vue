<template>
  <!-- 路由切换加载指示条：客户端导航期间顶部推进，颜色跟随主题 primary -->
  <NuxtLoadingIndicator color="var(--primary)" :height="2" />
  <NuxtLayout>
    <NuxtPage />
  </NuxtLayout>
</template>

