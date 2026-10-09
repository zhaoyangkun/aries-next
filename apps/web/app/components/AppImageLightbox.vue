<script setup lang="ts">
// 展示端图片灯箱：移植自管理端 AppImageLightbox（滚轮缩放/拖拽平移/键盘操作），
// 但 web 端没有 UI 库与图标库，按钮用原生元素 + 内联 SVG，并增加左右切换。
// 两种数据源：图库条目 items（PublicGalleryItem）或通用图片 images（文章正文放大），
// 内部统一归一化为 slides，caption 分别来自 alt+location 或显式 caption。
//
// 指针交互用 Pointer Events 统一处理（鼠标行为与原来一致：任意缩放级别可拖拽平移）：
// - 双指 pinch：按两指间距比例缩放，缩放中心为两指中点（pinchTransform 保持中点锚定）
// - 触摸单指：放大时拖拽平移；未放大时横向滑动切换（跟手位移，松手按阈值切换或回弹）、
//   向下滑动幅度大时关闭灯箱；方向未越过判定阈值前处于 undecided，不上锁任何手势
// - 双击/双指点按：1x ↔ 2.5x，以点击点为缩放中心（灯箱本来就没有单击空白关闭，不冲突）
// 手势数学在 utils/lightboxGestures.ts（纯函数，有单测）。
import type { PublicGalleryItem } from '~/composables/usePublicApi'

/** 通用图片模式入参（文章正文图片放大等场景） */
export interface LightboxImage {
  url: string
  alt?: string
  caption?: string
}

const props = defineProps<{
  open: boolean
  index: number
  /** 图库模式：图库条目列表（原有用法，保持兼容） */
  items?: PublicGalleryItem[]
  /** 通用模式：任意图片列表；与 items 二选一，优先取 images */
  images?: LightboxImage[]
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
  'update:index': [index: number]
}>()

const MIN_SCALE = 0.1
const MAX_SCALE = 20
const WHEEL_ZOOM_SPEED = 0.002
const DOUBLE_TAP_SCALE = 2.5
// 单指滑动的方向判定阈值：越过才进入 swipe/close-swipe，在此之前不消耗位移
const SWIPE_DECIDE_PX = 8
// 轻点判定：短促且几乎没动，用于双击缩放识别
const TAP_MAX_DURATION_MS = 300
const TAP_MAX_MOVE_PX = 10
const DOUBLE_TAP_WINDOW_MS = 300
const DOUBLE_TAP_RADIUS_PX = 32

const scale = ref(1)
const translateX = ref(0)
const translateY = ref(0)
const isDragging = ref(false)
// 松手回弹/切换/双击缩放期间为 true，给 img 加 transform 过渡；跟手阶段必须为 false
const settling = ref(false)
const containerRef = ref<HTMLDivElement | null>(null)

// 手势状态机：idle →（单指）undecided → swipe / close-swipe / pan / locked
//                     →（双指）pinch →（剩单指）重新锚定回 pan / undecided
// locked：未放大时的上滑或单图横滑，无对应手势，松手前不做任何位移
type GestureMode = 'idle' | 'undecided' | 'pan' | 'swipe' | 'close-swipe' | 'pinch' | 'locked'

// 以下为手势进行中的瞬态，不需要响应式
const activePointers = new Map<number, { x: number; y: number }>()
let gestureMode: GestureMode = 'idle'
let gestureStartX = 0
let gestureStartY = 0
let gestureStartTime = 0
let gestureStartTranslateX = 0
let gestureStartTranslateY = 0
let pinchStartDistance = 0
let pinchStartCenter = { x: 0, y: 0 }
let pinchStartScale = 1
let pinchStartTranslateX = 0
let pinchStartTranslateY = 0
// 上一次轻点的时间与位置，用于双击判定
let lastTapTime = 0
let lastTapX = 0
let lastTapY = 0

const current = computed(() => slides.value[props.index])
const scalePercent = computed(() => Math.round(scale.value * 100))
const caption = computed(() => current.value?.caption ?? '')

/** 统一后的播放列表：图库条目取 url/alt，caption 拼 alt · location；通用图片原样使用 */
const slides = computed<LightboxImage[]>(() => {
  if (props.images) return props.images
  return (props.items ?? []).map((item) => ({
    url: item.url,
    alt: item.alt,
    caption: [item.alt, item.location].filter(Boolean).join(' · '),
  }))
})

function resetGesture() {
  activePointers.clear()
  gestureMode = 'idle'
  isDragging.value = false
  lastTapTime = 0
}

function resetView() {
  scale.value = 1
  translateX.value = 0
  translateY.value = 0
  resetGesture()
}

function close() {
  emit('update:open', false)
}

function go(step: number) {
  const next = (props.index + step + slides.value.length) % slides.value.length
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
  // 滚轮是高频连续输入，不走过渡动画，避免缩放滞后
  settling.value = false
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

/** client 坐标换算为相对容器中心的坐标（与 transformOrigin: center 对应） */
function toContainerPoint(clientX: number, clientY: number) {
  const rect = containerRef.value?.getBoundingClientRect()
  if (!rect) return { x: 0, y: 0 }
  return {
    x: clientX - rect.left - rect.width / 2,
    y: clientY - rect.top - rect.height / 2,
  }
}

/** 当前两指的间距与中点（中点已换算到容器中心坐标系） */
function pinchState() {
  const points = [...activePointers.values()]
  if (points.length < 2) return null
  const [a, b] = points as [{ x: number; y: number }, { x: number; y: number }]
  return {
    distance: Math.hypot(a.x - b.x, a.y - b.y),
    center: toContainerPoint((a.x + b.x) / 2, (a.y + b.y) / 2),
  }
}

function handlePointerDown(event: PointerEvent) {
  if (event.pointerType === 'mouse' && event.button !== 0) return
  containerRef.value?.setPointerCapture(event.pointerId)
  activePointers.set(event.pointerId, { x: event.clientX, y: event.clientY })
  settling.value = false

  if (activePointers.size === 1) {
    gestureStartX = event.clientX
    gestureStartY = event.clientY
    gestureStartTime = performance.now()
    gestureStartTranslateX = translateX.value
    gestureStartTranslateY = translateY.value
    // 鼠标保持原有行为（任意缩放级别可拖拽）；触摸在放大时拖拽，未放大时等方向判定
    if (event.pointerType === 'mouse' || scale.value > 1) {
      gestureMode = 'pan'
      isDragging.value = event.pointerType === 'mouse'
    }
    else {
      gestureMode = 'undecided'
    }
  }
  else if (activePointers.size === 2) {
    // 第二指落下：丢弃进行中的单指手势，以当前视图状态为基准进入 pinch
    const pinch = pinchState()
    if (pinch) {
      gestureMode = 'pinch'
      isDragging.value = false
      pinchStartDistance = pinch.distance
      pinchStartCenter = pinch.center
      pinchStartScale = scale.value
      pinchStartTranslateX = translateX.value
      pinchStartTranslateY = translateY.value
    }
    // 捏合后不触发双击缩放
    lastTapTime = 0
  }
  // 阻止图片拖拽/选中与兼容鼠标事件（触摸同时触发 pointer + mouse）
  event.preventDefault()
}

function handlePointerMove(event: PointerEvent) {
  if (!activePointers.has(event.pointerId)) return
  activePointers.set(event.pointerId, { x: event.clientX, y: event.clientY })

  if (gestureMode === 'pinch') {
    const pinch = pinchState()
    if (!pinch) return
    const next = pinchTransform({
      startDistance: pinchStartDistance,
      currentDistance: pinch.distance,
      startCenter: pinchStartCenter,
      currentCenter: pinch.center,
      startScale: pinchStartScale,
      startTranslate: { x: pinchStartTranslateX, y: pinchStartTranslateY },
      minScale: MIN_SCALE,
      maxScale: MAX_SCALE,
    })
    scale.value = next.scale
    translateX.value = next.translateX
    translateY.value = next.translateY
    return
  }

  if (activePointers.size !== 1) return
  const dx = event.clientX - gestureStartX
  const dy = event.clientY - gestureStartY

  if (gestureMode === 'undecided') {
    if (Math.abs(dx) < SWIPE_DECIDE_PX && Math.abs(dy) < SWIPE_DECIDE_PX) return
    if (Math.abs(dx) >= Math.abs(dy)) {
      // 单图没有可切换对象，横滑直接锁定
      gestureMode = slides.value.length > 1 ? 'swipe' : 'locked'
    }
    else {
      gestureMode = dy > 0 ? 'close-swipe' : 'locked'
    }
  }

  if (gestureMode === 'pan') {
    translateX.value = gestureStartTranslateX + dx
    translateY.value = gestureStartTranslateY + dy
  }
  else if (gestureMode === 'swipe' || gestureMode === 'close-swipe') {
    // 跟手位移反馈：swipe 走横向，close-swipe 走纵向，松手统一裁决
    if (gestureMode === 'swipe') translateX.value = gestureStartTranslateX + dx
    else translateY.value = gestureStartTranslateY + dy
  }
}

function handlePointerEnd(event: PointerEvent) {
  if (!activePointers.has(event.pointerId)) return
  const cancelled = event.type === 'pointercancel'
  const finishedMode = gestureMode
  activePointers.delete(event.pointerId)

  // 双指变单指：以剩余手指当前位置重新锚定基准，避免视图跳变或手势卡死
  if (finishedMode === 'pinch') {
    if (activePointers.size >= 2) {
      // 三指以上退回双指：以当前视图状态重新快照 pinch 基准，留在 pinch
      const pinch = pinchState()
      if (pinch) {
        pinchStartDistance = pinch.distance
        pinchStartCenter = pinch.center
        pinchStartScale = scale.value
        pinchStartTranslateX = translateX.value
        pinchStartTranslateY = translateY.value
      }
      return
    }
    const remaining = activePointers.values().next().value
    if (remaining) {
      gestureStartX = remaining.x
      gestureStartY = remaining.y
      gestureStartTime = performance.now()
      gestureStartTranslateX = translateX.value
      gestureStartTranslateY = translateY.value
      gestureMode = scale.value > 1 ? 'pan' : 'undecided'
    }
    else {
      gestureMode = 'idle'
    }
    return
  }

  if (activePointers.size > 0) return
  gestureMode = 'idle'
  isDragging.value = false

  // pointercancel：swipe 的位移是瞬态，收敛回弹；pan 的位置是有效状态，保留
  if (cancelled) {
    if (finishedMode === 'swipe' || finishedMode === 'close-swipe') settleView()
    return
  }

  const dx = event.clientX - gestureStartX
  const dy = event.clientY - gestureStartY
  const duration = performance.now() - gestureStartTime

  if (finishedMode === 'swipe') {
    const step = swipeStep(dx, dy, duration)
    if (step !== 0) {
      // go() 触发 watch(index) → resetView，位移经回弹过渡归零，形成滑入效果
      settling.value = true
      go(step)
    }
    else {
      settleView()
    }
  }
  else if (finishedMode === 'close-swipe') {
    if (shouldCloseSwipe(dy, dx, duration)) close()
    else settleView()
  }
  else if (finishedMode === 'pan' || finishedMode === 'undecided') {
    // 短促且几乎没动 = 轻点，进入双击判定；否则是拖拽结束，保持位置（与原鼠标行为一致）
    if (duration <= TAP_MAX_DURATION_MS && Math.hypot(dx, dy) <= TAP_MAX_MOVE_PX) {
      handleTap(event)
    }
  }
}

/** 松手回弹：swipe/close-swipe 的跟手位移归零 */
function settleView() {
  settling.value = true
  translateX.value = 0
  translateY.value = 0
}

function handleTap(event: PointerEvent) {
  const now = performance.now()
  const tapDistance = Math.hypot(event.clientX - lastTapX, event.clientY - lastTapY)
  if (isDoubleTap(now - lastTapTime, tapDistance, DOUBLE_TAP_WINDOW_MS, DOUBLE_TAP_RADIUS_PX)) {
    lastTapTime = 0
    settling.value = true
    if (scale.value > 1) {
      resetView()
    }
    else {
      const point = toContainerPoint(event.clientX, event.clientY)
      setScale(DOUBLE_TAP_SCALE, point.x, point.y)
    }
  }
  else {
    lastTapTime = now
    lastTapX = event.clientX
    lastTapY = event.clientY
  }
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
  if (!open) resetGesture()
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
      >
        <!-- 顶栏：计数 + 关闭 -->
        <div class="absolute left-0 right-0 top-0 z-10 flex items-center justify-between px-4 py-3">
          <span class="text-sm text-white/70">{{ index + 1 }} / {{ slides.length }}</span>
          <button
            type="button"
            class="rounded-md p-2 text-white transition-colors hover:bg-white/10"
            aria-label="关闭预览"
            @click="close"
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-5"><path d="M18 6 6 18" /><path d="m6 6 12 12" /></svg>
          </button>
        </div>

        <!-- 图片容器：h-full w-full + object-contain 让图片默认填满屏幕；
             touch-none 关闭浏览器原生触摸滚动/缩放，手势全由 Pointer Events 处理 -->
        <div
          ref="containerRef"
          class="h-full w-full touch-none overflow-hidden"
          :class="isDragging ? 'cursor-grabbing' : 'cursor-grab'"
          @wheel.prevent="handleWheel"
          @pointerdown="handlePointerDown"
          @pointermove="handlePointerMove"
          @pointerup="handlePointerEnd"
          @pointercancel="handlePointerEnd"
        >
          <img
            :src="current.url"
            :alt="current.alt || ''"
            :style="{
              transform: `translate(${translateX}px, ${translateY}px) scale(${scale})`,
              transformOrigin: 'center center',
            }"
            class="lightbox-img pointer-events-none mx-auto h-full w-full select-none object-contain"
            :class="{ 'is-settling': settling }"
            draggable="false"
          >
        </div>

        <!-- 左右切换（多图时显示） -->
        <template v-if="slides.length > 1">
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

        <!-- 快捷键提示（触屏手势：双指缩放 · 滑动切换 · 下滑关闭 · 双击放大） -->
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

/* 手势松手后的回弹/切换/双击缩放过渡；跟手阶段（无 is-settling）不加 transition，避免拖拽滞后 */
.lightbox-img.is-settling {
  transition: transform 0.18s ease;
}

@media (prefers-reduced-motion: reduce) {
  .lightbox-enter-active,
  .lightbox-leave-active,
  .lightbox-img.is-settling {
    transition: none;
  }
}
</style>
