<script setup lang="ts">
// 顶部滚动进度条：全局挂载于默认布局，随滚动以 scaleX 推进（无过渡动画，天然兼容 reduced-motion）
const progress = ref(0)

onMounted(() => {
  const onScroll = () => {
    const total = document.documentElement.scrollHeight - window.innerHeight
    progress.value = total > 0 ? Math.min(1, window.scrollY / total) : 0
  }
  onScroll()
  window.addEventListener('scroll', onScroll, { passive: true })
  onBeforeUnmount(() => window.removeEventListener('scroll', onScroll))
})
</script>

<template>
  <div class="reading-progress" :style="{ transform: `scaleX(${progress})` }" aria-hidden="true" />
</template>
