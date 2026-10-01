<script setup lang="ts">
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'

withDefaults(defineProps<{
  title: string
  description?: string
  wide?: boolean
}>(), {
  wide: false,
  description: '',
})
</script>

<template>
  <div class="flex min-h-screen items-center justify-center bg-muted/30 p-4">
    <main class="flex w-full flex-col items-center gap-6" :class="wide ? 'max-w-lg' : 'max-w-sm'">
      <div class="flex items-center gap-2.5">
        <span class="flex size-8 items-center justify-center rounded-md bg-primary text-sm font-bold text-primary-foreground">A</span>
        <span class="text-sm font-semibold">Aries 管理后台</span>
      </div>
      <Card class="w-full border-border/70 shadow-sm">
        <CardHeader class="text-center">
          <CardTitle class="text-xl">{{ title }}</CardTitle>
          <CardDescription v-if="description" class="leading-6">
            {{ description }}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <slot />
        </CardContent>
      </Card>
      <p class="text-center text-xs text-muted-foreground">Aries Content Management</p>
    </main>
  </div>
</template>
