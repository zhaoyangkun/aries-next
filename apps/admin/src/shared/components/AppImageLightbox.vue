<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { Maximize2Icon, MinusIcon, PlusIcon, XIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'

const props = defineProps<{
  open: boolean
  src: string
  alt?: string
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
}>()

const MIN_SCALE = 0.1
const MAX_SCALE = 20
const WHEEL_ZOOM_SPEED = 0.002

const scale = ref(1)
const translateX = ref(0)
const translateY = ref(0)
const isDragging = ref(false)
const dragStartX = ref(0)
const dragStartY = ref(0)
const dragStartTranslateX = ref(0)
const dragStartTranslateY = ref(0)
const containerRef = ref<HTMLDivElement | null>(null)

const scalePercent = computed(() => Math.round(scale.value * 100))

function close() {
  emit('update:open', false)
  resetView()
}

function zoomIn() {
  setScale(scale.value * 1.25)
}

function zoomOut() {
  setScale(scale.value / 1.25)
}

function resetView() {
  scale.value = 1
  translateX.value = 0
  translateY.value = 0
}

function setScale(newScale: number, centerX?: number, centerY?: number) {
  const clampedScale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, newScale))
  if (clampedScale === scale.value) return

  if (centerX !== undefined && centerY !== undefined) {
    const ratio = clampedScale / scale.value
    translateX.value = centerX - ratio * (centerX - translateX.value)
    translateY.value = centerY - ratio * (centerY - translateY.value)
  }

  scale.value = clampedScale
}

function handleWheel(event: WheelEvent) {
  event.preventDefault()
  const delta = -event.deltaY * WHEEL_ZOOM_SPEED
  const newScale = scale.value * Math.exp(delta)

  const container = containerRef.value
  if (container) {
    const rect = container.getBoundingClientRect()
    const centerX = event.clientX - rect.left - rect.width / 2
    const centerY = event.clientY - rect.top - rect.height / 2
    setScale(newScale, centerX, centerY)
  }
  else {
    setScale(newScale)
  }
}

function handleMouseDown(event: MouseEvent) {
  if (event.button !== 0) return
  isDragging.value = true
  dragStartX.value = event.clientX
  dragStartY.value = event.clientY
  dragStartTranslateX.value = translateX.value
  dragStartTranslateY.value = translateY.value
  event.preventDefault()
}

function handleMouseMove(event: MouseEvent) {
  if (!isDragging.value) return
  translateX.value = dragStartTranslateX.value + (event.clientX - dragStartX.value)
  translateY.value = dragStartTranslateY.value + (event.clientY - dragStartY.value)
}

function handleMouseUp() {
  isDragging.value = false
}

function handleKeyDown(event: KeyboardEvent) {
  if (!props.open) return
  switch (event.key) {
    case 'Escape':
      close()
      break
    case '+':
    case '=':
      zoomIn()
      break
    case '-':
      zoomOut()
      break
    case '0':
      resetView()
      break
  }
}

onMounted(() => {
  window.addEventListener('keydown', handleKeyDown)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleKeyDown)
})
</script>

<template>
  <Teleport to="body">
    <Transition name="lightbox">
      <div
        v-if="open"
        class="fixed inset-0 z-[60] flex items-center justify-center bg-black/90"
        @pointerdown.stop
        @mouseup="handleMouseUp"
        @mousemove="handleMouseMove"
      >
        <!-- 顶栏 -->
        <div class="absolute left-0 right-0 top-0 z-10 flex items-center justify-between px-4 py-3">
          <span class="text-sm text-white/70">{{ scalePercent }}%</span>
          <Button
            variant="ghost"
            size="icon"
            class="text-white hover:bg-white/10 hover:text-white"
            aria-label="关闭预览"
            @click="close"
          >
            <XIcon class="size-5" />
          </Button>
        </div>

        <!-- 图片容器：h-full w-full + object-contain 让图片默认填满屏幕 -->
        <div
          ref="containerRef"
          class="h-full w-full overflow-hidden"
          :class="isDragging ? 'cursor-grabbing' : 'cursor-grab'"
          @wheel.prevent="handleWheel"
          @mousedown="handleMouseDown"
        >
          <img
            :src="src"
            :alt="alt || ''"
            :style="{
              transform: `translate(${translateX}px, ${translateY}px) scale(${scale})`,
              transformOrigin: 'center center',
            }"
            class="pointer-events-none mx-auto h-full w-full select-none object-contain"
            draggable="false"
          />
        </div>

        <!-- 底栏：缩放控制 -->
        <div class="absolute bottom-4 left-1/2 z-10 flex -translate-x-1/2 items-center gap-1 rounded-lg bg-black/60 p-1 backdrop-blur">
          <Button
            variant="ghost"
            size="icon"
            class="size-8 text-white hover:bg-white/15 hover:text-white"
            aria-label="缩小"
            @click="zoomOut"
          >
            <MinusIcon class="size-4" />
          </Button>

          <span class="w-14 text-center text-xs font-mono text-white/80">{{ scalePercent }}%</span>

          <Button
            variant="ghost"
            size="icon"
            class="size-8 text-white hover:bg-white/15 hover:text-white"
            aria-label="放大"
            @click="zoomIn"
          >
            <PlusIcon class="size-4" />
          </Button>

          <div class="mx-1 h-4 w-px bg-white/20" />

          <Button
            variant="ghost"
            size="icon"
            class="size-8 text-white hover:bg-white/15 hover:text-white"
            aria-label="适应屏幕"
            title="适应屏幕 (0)"
            @click="resetView"
          >
            <Maximize2Icon class="size-4" />
          </Button>
        </div>

        <!-- 快捷键提示 -->
        <div class="absolute bottom-4 right-4 z-10 hidden text-xs text-white/40 lg:block">
          滚轮缩放 · 拖拽平移 · ESC 关闭
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.lightbox-enter-active,
.lightbox-leave-active {
  transition: opacity 0.2s ease;
}
.lightbox-enter-from,
.lightbox-leave-to {
  opacity: 0;
}
</style>
