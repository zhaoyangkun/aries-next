<script setup lang="ts">
// 移动端（sm 以下）全屏导航抽屉：替代 header 里横向滚动的平铺导航。
// 打开时锁定页面滚动，Esc / 点击空白 / 路由变化关闭；焦点进入抽屉并圈在内部，
// 关闭后还给汉堡按钮。搜索项复用全局 SearchPalette：打开弹层时把滚动锁交接给它，
// 避免抽屉关闭的恢复 overflow 把搜索弹层的锁定清掉（两个 watch 的执行顺序不可依赖）。
import type { PublicNavigationNode } from '~/composables/usePublicApi'

const props = defineProps<{
  open: boolean
  items: PublicNavigationNode[]
  siteName: string
}>()

const emit = defineEmits<{
  'update:open': [open: boolean]
}>()

const route = useRoute()
const { show: showSearchPalette } = useSearchPalette()

const closeButtonRef = ref<HTMLButtonElement | null>(null)
// 交接标记：跳搜索弹层时为 true，本次关闭不恢复 body overflow（弹层自己锁定）
let handoffToPalette = false

function close() {
  emit('update:open', false)
}

function openSearch() {
  handoffToPalette = true
  close()
  showSearchPalette()
}

// 抽屉内点链接跳转后自动关闭
watch(() => route.fullPath, () => {
  if (props.open) close()
})

watch(() => props.open, async (value) => {
  if (!import.meta.client) return
  if (value) {
    document.body.style.overflow = 'hidden'
    await nextTick()
    closeButtonRef.value?.focus()
  } else {
    if (handoffToPalette) handoffToPalette = false
    else document.body.style.overflow = ''
    // 焦点还给触发按钮，避免落到 body 丢失键盘上下文
    document.getElementById('site-nav-toggle')?.focus()
  }
})

onBeforeUnmount(() => {
  if (!import.meta.client || handoffToPalette) return
  document.body.style.overflow = ''
})

// Esc 关闭；Tab 在抽屉内首尾循环，把焦点圈在抽屉里
function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    close()
    return
  }
  if (event.key !== 'Tab') return
  const panel = event.currentTarget as HTMLElement
  const focusables = panel.querySelectorAll<HTMLElement>('a[href], button:not([disabled])')
  if (focusables.length === 0) return
  const first = focusables[0]
  const last = focusables[focusables.length - 1]
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault()
    last?.focus()
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault()
    first?.focus()
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="nav-drawer">
      <div
        v-if="open"
        id="site-nav-drawer"
        class="nav-drawer"
        role="dialog"
        aria-modal="true"
        aria-label="站点导航"
        @keydown="onKeydown"
        @click.self="close"
      >
        <div class="nav-drawer-head">
          <span class="nav-drawer-title">{{ siteName }}</span>
          <button
            ref="closeButtonRef"
            type="button"
            class="theme-toggle"
            aria-label="关闭导航菜单"
            @click="close"
          >
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M18 6 6 18" />
              <path d="m6 6 12 12" />
            </svg>
          </button>
        </div>
        <nav class="nav-drawer-nav" aria-label="移动端导航">
          <button type="button" class="nav-drawer-link nav-drawer-search" @click="openSearch">
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <circle cx="11" cy="11" r="8" />
              <path d="m21 21-4.3-4.3" />
            </svg>
            搜索
          </button>
          <template v-for="(item, index) in items" :key="index">
            <!-- 含子菜单的节点：父项为大号链接（无 href 时 AppNavLink 渲染 span，即分组标题），子项缩进小一号 -->
            <div v-if="item.children.length > 0" class="nav-drawer-group">
              <AppNavLink :node="item" class="nav-drawer-link" />
              <div class="nav-drawer-children">
                <AppNavLink
                  v-for="(child, childIndex) in item.children"
                  :key="childIndex"
                  :node="child"
                  class="nav-drawer-sublink"
                />
              </div>
            </div>
            <AppNavLink v-else :node="item" class="nav-drawer-link" />
          </template>
        </nav>
      </div>
    </Transition>
  </Teleport>
</template>
