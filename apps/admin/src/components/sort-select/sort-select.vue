<script setup lang="ts">
/** 排序选择器：排序字段 + 升降序组合下拉，文章/媒体等列表复用。 */
import { ArrowDownAZIcon, ArrowDownZAIcon, SlidersHorizontalIcon } from '@lucide/vue'

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'

export type TSort = 'asc' | 'desc'

const sort = defineModel<TSort>('sort', { default: 'asc' })
</script>

<template>
  <Select v-model:model-value="sort">
    <SelectTrigger class="w-16">
      <SelectValue>
        <SlidersHorizontalIcon :size="16" />
      </SelectValue>
    </SelectTrigger>
    <SelectContent align="end">
      <SelectItem value="asc">
        <div class="flex items-center gap-4">
          <ArrowDownAZIcon :size="16" />
          <span>Ascending</span>
        </div>
      </SelectItem>
      <SelectItem value="desc">
        <div class="flex items-center gap-4">
          <ArrowDownZAIcon :size="16" />
          <span>Descending</span>
        </div>
      </SelectItem>
    </SelectContent>
  </Select>
</template>
