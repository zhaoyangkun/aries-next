<script setup lang="ts">
// 顶栏搜索框（桌面端）：输入防抖请求 /search/suggest 下拉建议，
// Enter 进搜索页、↑↓ 选择建议、Esc 关闭；Ctrl/⌘+K 全局聚焦。
import type { PublicSearchSuggestion } from '~/composables/usePublicApi'

const keyword = ref('')
const suggestions = ref<PublicSearchSuggestion[]>([])
const open = ref(false)
const activeIndex = ref(-1)
const inputRef = ref<HTMLInputElement | null>(null)

let timer: ReturnType<typeof setTimeout> | undefined
let seq = 0

async function fetchSuggestions() {
  const q = keyword.value.trim()
  if (!q) {
    suggestions.value = []
    open.value = false
    return
  }
  const current = ++seq
  try {
    const data = (await $fetch('/search/suggest', {
      baseURL: '/api/public',
      query: { q },
    })) as PublicSearchSuggestion[]
    // 丢弃过期响应（防抖窗口内输入已变化）
    if (current !== seq) return
    suggestions.value = data
    open.value = data.length > 0
    activeIndex.value = -1
  } catch {
    suggestions.value = []
    open.value = false
  }
}

function onInput() {
  clearTimeout(timer)
  timer = setTimeout(fetchSuggestions, 250)
}

function submit() {
  const q = keyword.value.trim()
  open.value = false
  if (q) navigateTo({ path: '/search', query: { q } })
}

function goTo(suggestion: PublicSearchSuggestion) {
  open.value = false
  keyword.value = ''
  navigateTo(`/articles/${suggestion.slug}`)
}

function onKeydown(event: KeyboardEvent) {
  if (!open.value) return
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    activeIndex.value = (activeIndex.value + 1) % suggestions.value.length
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    activeIndex.value =
      (activeIndex.value - 1 + suggestions.value.length) % suggestions.value.length
  } else if (event.key === 'Enter' && activeIndex.value >= 0) {
    event.preventDefault()
    const target = suggestions.value[activeIndex.value]
    if (target) goTo(target)
  } else if (event.key === 'Escape') {
    open.value = false
  }
}

// Ctrl/⌘+K：全局聚焦搜索框（输入框自身的 keydown 之外，挂在 window 上）
onMounted(() => {
  const onGlobalKey = (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault()
      inputRef.value?.focus()
      inputRef.value?.select()
    }
  }
  window.addEventListener('keydown', onGlobalKey)
  onBeforeUnmount(() => {
    window.removeEventListener('keydown', onGlobalKey)
    clearTimeout(timer)
  })
})
</script>

<template>
  <form
    class="search-box"
    role="search"
    @submit.prevent="submit"
  >
    <svg
      class="search-box-icon"
      xmlns="http://www.w3.org/2000/svg"
      width="15"
      height="15"
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
    <input
      ref="inputRef"
      v-model="keyword"
      type="search"
      maxlength="100"
      placeholder="搜索…"
      aria-label="站内搜索"
      autocomplete="off"
      class="search-box-input"
      @input="onInput"
      @keydown="onKeydown"
      @blur="open = false"
    />
    <kbd class="search-box-kbd" aria-hidden="true">Ctrl K</kbd>

    <div v-if="open" class="search-box-dropdown" role="listbox" aria-label="搜索建议">
      <button
        v-for="(suggestion, index) in suggestions"
        :key="suggestion.slug"
        type="button"
        class="search-box-item"
        :class="{ 'is-active': index === activeIndex }"
        role="option"
        :aria-selected="index === activeIndex"
        @mousedown.prevent="goTo(suggestion)"
        @mouseenter="activeIndex = index"
      >
        <HighlightText :text="suggestion.title" :keywords="[keyword.trim()]" />
      </button>
      <div class="search-box-hint">Enter 搜索全文 · ↑↓ 选择 · Esc 关闭</div>
    </div>
  </form>
</template>
