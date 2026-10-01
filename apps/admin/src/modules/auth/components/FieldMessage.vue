<script setup lang="ts">
import { CircleAlertIcon } from '@lucide/vue'

withDefaults(
  defineProps<{
    id?: string
    message?: string
    error?: boolean
  }>(),
  {
    id: undefined,
    message: '',
    error: false,
  },
)
</script>

<template>
  <p
    v-if="message"
    :id="id"
    :role="error ? 'alert' : undefined"
    :class="[
      'flex items-start gap-1.5 text-xs leading-5',
      error ? 'font-medium text-destructive' : 'text-muted-foreground',
    ]"
  >
    <CircleAlertIcon v-if="error" class="mt-0.5 size-3.5 shrink-0" />
    <span>{{ message }}</span>
  </p>
</template>
