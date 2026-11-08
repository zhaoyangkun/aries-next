<script setup lang="ts">
/**
 * 极简时序折线图（SVG polyline）：viewBox 固定宽度按容器拉伸，
 * `vector-effect="non-scaling-stroke"` 保证线宽不随拉伸变形。
 * 用于系统监控页的实时序列与历史趋势，不引入图表库。
 */
import { computed } from 'vue'

const props = withDefaults(defineProps<{
  /** 时序数值（追加顺序即时间顺序）。 */
  points: number[]
  /** 纵轴上限：缺省取数据最大值（至少为 1，避免除零）。 */
  max?: number
  height?: number
  stroke?: string
}>(), {
  max: undefined,
  height: 48,
  stroke: 'hsl(var(--primary))',
})

// viewBox 宽度固定 100，横坐标只表达相对位置，渲染时拉伸到容器宽。
const WIDTH = 100
const PAD = 2

const polyline = computed(() => {
  if (props.points.length < 2)
    return ''
  const peak = Math.max(props.max ?? 0, ...props.points, 1)
  const stepX = (WIDTH - PAD * 2) / (props.points.length - 1)
  return props.points
    .map((value, index) => {
      const x = PAD + index * stepX
      const ratio = Math.min(Math.max(value, 0), peak) / peak
      const y = props.height - PAD - ratio * (props.height - PAD * 2)
      return `${x.toFixed(2)},${y.toFixed(2)}`
    })
    .join(' ')
})
</script>

<template>
  <svg
    :viewBox="`0 0 ${WIDTH} ${height}`"
    preserveAspectRatio="none"
    :style="{ height: `${height}px` }"
    class="block w-full"
    role="img"
  >
    <polyline
      v-if="polyline"
      :points="polyline"
      fill="none"
      :stroke="stroke"
      stroke-width="1.5"
      stroke-linejoin="round"
      stroke-linecap="round"
      vector-effect="non-scaling-stroke"
    />
    <text
      v-else
      x="50"
      :y="height / 2"
      text-anchor="middle"
      dominant-baseline="middle"
      font-size="9"
      class="fill-muted-foreground"
    >
      等待数据
    </text>
  </svg>
</template>
