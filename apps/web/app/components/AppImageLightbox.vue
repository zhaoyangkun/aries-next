<script setup lang="ts">
// 展示端图片灯箱：移植自管理端 AppImageLightbox（滚轮缩放/拖拽平移/键盘操作），
// 但 web 端没有 UI 库与图标库，按钮用原生元素 + 内联 SVG，并增加左右切换。
import type { PublicGalleryItem } from '~/composables/usePublicApi'

const props = defineProps<{
  open: boolean
  items: PublicGalleryItem[]
  index: number
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  'update:index': [index: number]
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

const current = computed(() => props.items[props.index])
const scalePercent = computed(() => Math.round(scale.value * 100))
const caption = computed(() => {
  const item = current.value
  return item ? [item.alt, item.location].filter(Boolean).join(' · ') : ''
})

function resetView() {
  scale.value = 1
  translateX.value = 0
  translateY.value = 0
}

function close() {
  emit('update:open', false)
}

function go(step: number) {
  const next = (props.index + step + props.items.length) % props.items.length
  emit('update:index', next)
}

function zoomIn() {
  setScale(scale.value * 1.25)
}

function zoomOut() {
  setScale(scale.value / 1.25)
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
    case 'ArrowLeft':
      go(-1)
      break
    case 'ArrowRight':
      go(1)
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

// 切换图片时恢复默认视图，避免上一张的缩放/位移带到下一张
watch(() => props.index, resetView)

// 灯箱打开时锁定页面滚动
watch(() => props.open, (open) => {
  document.body.style.overflow = open ? 'hidden' : ''
})

onMounted(() => {
  window.addEventListener('keydown', handleKeyDown)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleKeyDown)
  document.body.style.overflow = ''
})
</script>

<template>
  <Teleport to="body">
    <Transition name="lightbox">
      <div
        v-if="open && current"
        class="fixed inset-0 z-[60] flex items-center justify-center bg-black/90"
        @mouseup="handleMouseUp"
        @mousemove="handleMouseMove"
      >
        <!-- 顶栏：计数 + 关闭 -->
        <div class="absolute left-0 right-0 top-0 z-10 flex items-center justify-between px-4 py-3">
          <span class="text-sm text-white/70">{{ index + 1 }} / {{ items.length }}</span>
          <button
            type="button"
            class="rounded-md p-2 text-white transition-colors hover:bg-white/10"
            aria-label="关闭预览"
            @click="close"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-5"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg>
          </button>
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
            :src="current.url"
            :alt="current.alt || ''"
            :style="{
              transform: `translate(${translateX}px, ${translateY}px) scale(${scale})`,
              transformOrigin: 'center center',
            }"
            class="pointer-events-none mx-auto h-full w-full select-none object-contain"
            draggable="false"
          >
        </div>

        <!-- 左右切换（多图时显示） -->
        <template v-if="items.length > 1">
          <button
            type="button"
            class="absolute left-3 top-1/2 z-10 -translate-y-1/2 rounded-full bg-black/50 p-2.5 text-white backdrop-blur transition-colors hover:bg-black/70"
            aria-label="上一张"
            @click="go(-1)"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-5"><path d="m15 18-6-6 6-6" /></svg>
          </button>
          <button
            type="button"
            class="absolute right-3 top-1/2 z-10 -translate-y-1/2 rounded-full bg-black/50 p-2.5 text-white backdrop-blur transition-colors hover:bg-black/70"
            aria-label="下一张"
            @click="go(1)"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-5"><path d="m9 18 6-6-6-6" /></svg>
          </button>
        </template>

        <!-- 底栏：缩放控制 -->
        <div class="absolute bottom-4 left-1/2 z-10 flex -translate-x-1/2 items-center gap-1 rounded-lg bg-black/60 p-1 backdrop-blur">
          <button
            type="button"
            class="rounded-md p-1.5 text-white transition-colors hover:bg-white/15"
            aria-label="缩小"
            @click="zoomOut"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M5 12h14" /></svg>
          </button>

          <span class="w-14 text-center font-mono text-xs text-white/80">{{ scalePercent }}%</span>

          <button
            type="button"
            class="rounded-md p-1.5 text-white transition-colors hover:bg-white/15"
            aria-label="放大"
            @click="zoomIn"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M5 12h14" /><path d="M12 5v14" /></svg>
          </button>

          <div class="mx-1 h-4 w-px bg-white/20" />

          <button
            type="button"
            class="rounded-md p-1.5 text-white transition-colors hover:bg-white/15"
            aria-label="适应屏幕"
            title="适应屏幕 (0)"
            @click="resetView"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4"><path d="M8 3H5a2 2 0 0 0-2 2v3" /><path d="M21 8V5a2 2 0 0 0-2-2h-3" /><path d="M3 16v3a2 2 0 0 0 2 2h3" /><path d="M16 21h3a2 2 0 0 0 2-2v-3" /></svg>
          </button>
        </div>

        <!-- 图片说明（alt · 地点） -->
        <div
          v-if="caption"
          class="absolute bottom-4 left-4 z-10 max-w-[40vw] truncate text-xs text-white/60"
        >{{ caption }}</div>

        <!-- 快捷键提示 -->
        <div class="absolute bottom-4 right-4 z-10 hidden text-xs text-white/40 lg:block">
          滚轮缩放 · 拖拽平移 · ←→ 切换 · ESC 关闭
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
