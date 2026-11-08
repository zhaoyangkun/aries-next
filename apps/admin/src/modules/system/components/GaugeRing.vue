<script setup lang="ts">
/**
 * 环形仪表（SVG）：圆环进度 + 中心内容插槽，参考 Grafana stat panel /
 * 1Panel 资源监控的圆环呈现。从 12 点方向顺时针填充，进度变化带过渡动画。
 */
import { computed } from 'vue'

import type { UsageTone } from '@/modules/system/utils/format'
import { toneRingClass } from '@/modules/system/utils/format'

const props = withDefaults(defineProps<{
  /** 使用率（0–100，自动钳位）。 */
  percent: number
  /** 外径（px）。 */
  size?: number
  /** 环厚度（px）。 */
  stroke?: number
  tone?: UsageTone
}>(), {
  size: 112,
  stroke: 10,
  tone: 'ok',
})

// 圆环几何：内缩半个线宽，避免描边被裁切。
const radius = computed(() => (props.size - props.stroke) / 2)
const center = computed(() => props.size / 2)
const circumference = computed(() => 2 * Math.PI * radius.value)

// 归一化进度：非法输入按 0，超出钳位到 100。
const clamped = computed(() => {
  if (!Number.isFinite(props.percent))
    return 0
  return Math.min(100, Math.max(0, props.percent))
})

const dashOffset = computed(
  () => circumference.value * (1 - clamped.value / 100),
)
</script>

<template>
  <div class="relative inline-flex shrink-0 items-center justify-center" :style="{ width: `${size}px`, height: `${size}px` }">
    <svg :width="size" :height="size" class="-rotate-90">
      <circle
        :cx="center"
        :cy="center"
        :r="radius"
        fill="none"
        :stroke-width="stroke"
        class="stroke-muted/40"
      />
      <circle
        :cx="center"
        :cy="center"
        :r="radius"
        fill="none"
        :stroke-width="stroke"
        stroke-linecap="round"
        :stroke-dasharray="circumference"
        :stroke-dashoffset="dashOffset"
        class="transition-[stroke-dashoffset] duration-700 ease-out"
        :class="toneRingClass[tone]"
      />
    </svg>
    <div class="absolute inset-0 flex flex-col items-center justify-center">
      <slot />
    </div>
  </div>
</template>
