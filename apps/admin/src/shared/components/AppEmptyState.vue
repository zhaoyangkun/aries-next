<script setup lang="ts">
import type { Component, HTMLAttributes } from 'vue'
import { cn } from '@/lib/utils'

const props = defineProps<{
  title: string
  description: string
  icon: Component
  class?: HTMLAttributes['class']
}>()
</script>

<template>
  <div
    data-slot="empty"
    :class="cn('flex min-h-56 flex-col items-center justify-center px-6 py-10 text-center', props.class)"
  >
    <div class="mb-4 flex size-10 items-center justify-center rounded-lg bg-muted text-muted-foreground">
      <component :is="icon" class="size-5" />
    </div>
    <h3 class="text-sm font-medium">{{ title }}</h3>
    <p class="mt-1.5 max-w-sm text-xs leading-5 text-muted-foreground">{{ description }}</p>
    <div v-if="$slots.actions" class="mt-5 flex items-center gap-2">
      <slot name="actions" />
    </div>
  </div>
</template>
