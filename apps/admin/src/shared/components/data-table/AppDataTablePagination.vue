<script setup lang="ts">
import { computed, ref, watch } from 'vue'
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

/**
 * 页码窗口：首尾恒显，中间以当前页为中心各扩一页，
 * 间隙用 null 表示省略号。总页数 ≤ 7 时全量展示无省略。
 */
const pageItems = computed<(number | null)[]>(() => {
  const count = pageCount.value
  const current = page.value
  if (count <= 7) return Array.from({ length: count }, (_, index) => index + 1)
  const wanted = new Set<number>([1, 2, count - 1, count, current - 1, current, current + 1])
  const pages = Array.from(wanted)
    .filter((item) => item >= 1 && item <= count)
    .sort((a, b) => a - b)
  const items: (number | null)[] = []
  let previous = 0
  for (const item of pages) {
    if (item - previous > 1) items.push(null)
    items.push(item)
    previous = item
  }
  return items
})

// 跳转输入：仅接受 1..pageCount 的整数，回车或失焦提交；非法输入回落到当前页。
const jumpValue = ref('')
watch(page, () => {
  jumpValue.value = ''
})

function commitJump() {
  const target = Number.parseInt(jumpValue.value, 10)
  if (Number.isFinite(target) && target >= 1 && target <= pageCount.value) {
    page.value = target
  }
  jumpValue.value = ''
}
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
      <template v-for="(item, index) in pageItems" :key="index">
        <span v-if="item === null" class="px-1 text-xs text-muted-foreground" aria-hidden="true">…</span>
        <Button
          v-else
          size="icon-sm"
          class="min-w-8 tabular-nums"
          :aria-label="`第 ${item} 页`"
          :aria-current="item === page ? 'page' : undefined"
          :disabled="loading"
          :variant="item === page ? 'default' : 'outline'"
          @click="page = item"
        >
          {{ item }}
        </Button>
      </template>
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
    <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
      前往
      <input
        v-model="jumpValue"
        type="text"
        inputmode="numeric"
        :placeholder="String(page)"
        aria-label="跳转页码"
        class="h-8 w-14 rounded-md border border-input bg-background px-2 text-center text-xs tabular-nums outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
        :disabled="loading"
        @keyup.enter="commitJump"
        @blur="commitJump"
      />
      页
    </div>
  </div>
</template>
