<script setup lang="ts">
import { computed } from 'vue'
import { ChevronLeftIcon, ChevronRightIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'

const props = withDefaults(
  defineProps<{
    total: number
    loading?: boolean
    /** 总数单位文案，如「篇文章」「项」。 */
    unit?: string
    pageSizeOptions?: number[]
  }>(),
  { loading: false, unit: '条记录', pageSizeOptions: () => [10, 20, 50] },
)

const page = defineModel<number>('page', { required: true })
const pageSize = defineModel<number>('pageSize', { required: true })

const pageCount = computed(() => Math.max(1, Math.ceil(props.total / pageSize.value)))
</script>

<template>
  <div class="flex flex-wrap items-center justify-end gap-x-4 gap-y-2">
    <slot />
    <p class="text-xs text-muted-foreground">共 {{ total }} {{ unit }}</p>
    <select
      v-model.number="pageSize"
      aria-label="每页数量"
      class="flex h-8 rounded-md border border-input bg-background px-2 text-xs outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
      :disabled="loading"
    >
      <option v-for="option in pageSizeOptions" :key="option" :value="option">{{ option }} / 页</option>
    </select>
    <div class="flex items-center gap-1">
      <Button
        variant="outline"
        size="icon-sm"
        aria-label="上一页"
        :disabled="page <= 1 || loading"
        @click="page -= 1"
      >
        <ChevronLeftIcon />
      </Button>
      <span class="min-w-14 text-center text-xs tabular-nums text-muted-foreground">{{ page }} / {{ pageCount }}</span>
      <Button
        variant="outline"
        size="icon-sm"
        aria-label="下一页"
        :disabled="page >= pageCount || loading"
        @click="page += 1"
      >
        <ChevronRightIcon />
      </Button>
    </div>
  </div>
</template>
