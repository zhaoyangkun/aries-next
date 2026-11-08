<script setup lang="ts">
/** 页面标题区：标题 + 描述。Props: title/description。 */
defineProps<{
  title: string
  description: string
}>()
</script>

<template>
  <header class="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
    <div class="min-w-0">
      <h1 class="text-2xl font-semibold">{{ title }}</h1>
      <p class="mt-1.5 max-w-2xl text-sm leading-6 text-muted-foreground">{{ description }}</p>
    </div>
    <div v-if="$slots.actions" class="flex shrink-0 items-center gap-2">
      <slot name="actions" />
    </div>
  </header>
</template>
