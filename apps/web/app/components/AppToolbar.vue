<script setup lang="ts">
// 右下角浮动工具栏：主按钮 44px 圆形 + SVG 进度环显示滚动进度，
// 滚动超过 300px 浮现，点击平滑回顶；桌面悬停 / 移动端点击展开竖排子项
// （回到底部、目录、搜索）。样式复用 --card/--border token 与 .theme-toggle 圆形按钮模式。
const { show: showSearch } = useSearchPalette()
const { open: tocOpen, hasToc, show: showToc } = useTocDrawer()

const route = useRoute()

const scrolled = ref(false)
const progress = ref(0)
const canScrollDown = ref(false)
const isNarrow = ref(false)
const expanded = ref(false)

// 进度环：r=20 的圆周，stroke-dashoffset 按滚动比例推进
const RADIUS = 20
const CIRCUMFERENCE = 2 * Math.PI * RADIUS
const dashOffset = computed(() => CIRCUMFERENCE * (1 - Math.min(1, progress.value)))

// 目录子项：仅文章页、正文存在标题且视口 xl 以下（桌面有侧栏目录）时展示
const showTocButton = computed(
  () => route.path.startsWith('/articles/') && hasToc.value && isNarrow.value,
)
const hasSubItems = computed(() => canScrollDown.value || showTocButton.value)

onMounted(() => {
  const narrowMedia = window.matchMedia('(max-width: 1279.98px)')
  const onMediaChange = () => {
    isNarrow.value = narrowMedia.matches
  }
  onMediaChange()
  narrowMedia.addEventListener('change', onMediaChange)

  const onScroll = () => {
    scrolled.value = window.scrollY > 300
    const max = document.documentElement.scrollHeight - window.innerHeight
    progress.value = max > 0 ? Math.min(1, window.scrollY / max) : 0
    canScrollDown.value = max - window.scrollY > 4
  }
  onScroll()
  window.addEventListener('scroll', onScroll, { passive: true })

  onBeforeUnmount(() => {
    window.removeEventListener('scroll', onScroll)
    narrowMedia.removeEventListener('change', onMediaChange)
  })
})

function scrollToTop() {
  window.scrollTo({ top: 0, behavior: 'smooth' })
}

function scrollToBottom() {
  window.scrollTo({ top: document.documentElement.scrollHeight, behavior: 'smooth' })
}

// 桌面主按钮点击即回顶；触屏设备首次点击展开子项，再次点击回顶
function onMainClick() {
  const coarse = window.matchMedia('(hover: none), (pointer: coarse)').matches
  if (coarse && !expanded.value && hasSubItems.value) {
    expanded.value = true
    return
  }
  expanded.value = false
  scrollToTop()
}

function onSubClick(action: () => void) {
  expanded.value = false
  action()
}

watch(tocOpen, (open) => {
  if (open) expanded.value = false
})
</script>

<template>
  <div v-if="scrolled" class="app-toolbar" :class="{ 'is-expanded': expanded }">
    <!-- 竖排子项：桌面悬停（CSS）或移动端点击展开；DOM 顺序反转让视觉上从主按钮向上生长 -->
    <div class="toolbar-subs">
      <button
        type="button"
        class="toolbar-btn"
        aria-label="打开搜索"
        @click="onSubClick(showSearch)"
      >
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4" aria-hidden="true">
          <circle cx="11" cy="11" r="8" />
          <path d="m21 21-4.3-4.3" />
        </svg>
      </button>
      <button
        v-if="showTocButton"
        type="button"
        class="toolbar-btn"
        aria-label="打开文章目录"
        @click="onSubClick(showToc)"
      >
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4" aria-hidden="true">
          <path d="M4 6h16" />
          <path d="M4 12h10" />
          <path d="M4 18h16" />
        </svg>
      </button>
      <button
        v-if="canScrollDown"
        type="button"
        class="toolbar-btn"
        aria-label="回到底部"
        @click="onSubClick(scrollToBottom)"
      >
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4" aria-hidden="true">
          <path d="m6 9 6 6 6-6" />
        </svg>
      </button>
    </div>

    <!-- 主按钮：进度环 + 回顶箭头 -->
    <button
      type="button"
      class="toolbar-main"
      aria-label="回到顶部"
      @click="onMainClick"
    >
      <svg class="toolbar-ring" width="44" height="44" viewBox="0 0 44 44" aria-hidden="true">
        <circle cx="22" cy="22" :r="RADIUS" fill="none" stroke-width="2" class="toolbar-ring-track" />
        <circle
          cx="22"
          cy="22"
          :r="RADIUS"
          fill="none"
          stroke-width="2"
          stroke-linecap="round"
          class="toolbar-ring-progress"
          :stroke-dasharray="CIRCUMFERENCE"
          :stroke-dashoffset="dashOffset"
          transform="rotate(-90 22 22)"
        />
      </svg>
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4" aria-hidden="true">
        <path d="m18 15-6-6-6 6" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.app-toolbar {
  position: fixed;
  right: 1.5rem;
  bottom: 1.5rem;
  z-index: 40;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  opacity: 1;
  transform: translateY(0);
  transition:
    opacity 0.25s ease,
    transform 0.25s ease;
}

.app-toolbar {
  animation: toolbar-in 0.25s ease;
}

@keyframes toolbar-in {
  from {
    opacity: 0;
    transform: translateY(12px);
  }
}

/* 子项竖排（DOM 正序 = 视觉上从上到下，搜索在最上），默认收起 */
.toolbar-subs {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.5rem;
  opacity: 0;
  pointer-events: none;
  transform: translateY(8px);
  transition:
    opacity 0.2s ease,
    transform 0.2s ease;
}

/* 桌面端纯 CSS 悬停展开，不经过 Vue 状态 */
@media (hover: hover) and (pointer: fine) {
  .app-toolbar:hover .toolbar-subs,
  .app-toolbar:focus-within .toolbar-subs {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(0);
  }
}

/* 移动端点击展开 */
.app-toolbar.is-expanded .toolbar-subs {
  opacity: 1;
  pointer-events: auto;
  transform: translateY(0);
}

.toolbar-btn {
  display: grid;
  width: 40px;
  height: 40px;
  cursor: pointer;
  place-items: center;
  color: var(--muted-foreground);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 9999px;
  box-shadow: 0 2px 10px rgb(0 0 0 / 0.08);
  transition:
    color 0.2s ease,
    border-color 0.2s ease;
}

.toolbar-btn:hover {
  color: var(--primary);
  border-color: var(--primary);
}

.toolbar-main {
  position: relative;
  display: grid;
  width: 44px;
  height: 44px;
  cursor: pointer;
  place-items: center;
  color: var(--title);
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 9999px;
  box-shadow: 0 2px 10px rgb(0 0 0 / 0.1);
  transition: color 0.2s ease;
}

.toolbar-main:hover {
  color: var(--primary);
}

.toolbar-ring {
  position: absolute;
  inset: 0;
}

.toolbar-ring-track {
  stroke: var(--border);
}

.toolbar-ring-progress {
  stroke: var(--primary);
}
</style>
