<script setup lang="ts">
// 全局搜索弹层（Ctrl/⌘+K 唤起，Esc 关闭）：输入复用 /search/suggest 建议接口
// （与 AppSearchBox 同源），Enter 进 /search?q=，⌘/Ctrl+Enter 直接切到问 AI Tab。
import type { PublicSearchSuggestion } from '~/composables/usePublicApi'

const { open, hide } = useSearchPalette()

const keyword = ref('')
const suggestions = ref<PublicSearchSuggestion[]>([])
const activeIndex = ref(-1)
const inputRef = ref<HTMLInputElement | null>(null)

let timer: ReturnType<typeof setTimeout> | undefined
let seq = 0

async function fetchSuggestions() {
  const q = keyword.value.trim()
  if (!q) {
    suggestions.value = []
    activeIndex.value = -1
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
    activeIndex.value = -1
  } catch {
    suggestions.value = []
  }
}

function onInput() {
  clearTimeout(timer)
  timer = setTimeout(fetchSuggestions, 250)
}

function submit(ai = false) {
  const q = keyword.value.trim()
  hide()
  keyword.value = ''
  suggestions.value = []
  if (!q) return
  navigateTo({ path: '/search', query: ai ? { q, tab: 'ai' } : { q } })
}

function goTo(suggestion: PublicSearchSuggestion) {
  hide()
  keyword.value = ''
  suggestions.value = []
  navigateTo(`/articles/${suggestion.slug}`)
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'ArrowDown' && suggestions.value.length > 0) {
    event.preventDefault()
    activeIndex.value = (activeIndex.value + 1) % suggestions.value.length
  } else if (event.key === 'ArrowUp' && suggestions.value.length > 0) {
    event.preventDefault()
    activeIndex.value =
      (activeIndex.value - 1 + suggestions.value.length) % suggestions.value.length
  } else if (event.key === 'Enter') {
    event.preventDefault()
    if (activeIndex.value >= 0) {
      const target = suggestions.value[activeIndex.value]
      if (target) goTo(target)
    } else {
      // ⌘/Ctrl+Enter 直接带着关键词进问 AI Tab
      submit(event.metaKey || event.ctrlKey)
    }
  } else if (event.key === 'Escape') {
    hide()
  }
}

// 打开时锁 body 滚动并自动聚焦输入框
watch(open, async (value) => {
  if (!import.meta.client) return
  document.body.style.overflow = value ? 'hidden' : ''
  if (value) {
    await nextTick()
    inputRef.value?.focus()
  }
})

onBeforeUnmount(() => {
  if (!import.meta.client) return
  clearTimeout(timer)
  document.body.style.overflow = ''
})
</script>

<template>
  <Teleport to="body">
    <Transition name="palette">
      <div v-if="open" class="palette-backdrop" @click="hide">
        <div
          class="palette-panel"
          role="dialog"
          aria-label="站内搜索"
          @click.stop
        >
          <div class="palette-input-row">
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="palette-icon" aria-hidden="true">
              <circle cx="11" cy="11" r="8" />
              <path d="m21 21-4.3-4.3" />
            </svg>
            <input
              ref="inputRef"
              v-model="keyword"
              type="search"
              maxlength="100"
              placeholder="搜索文章…"
              aria-label="站内搜索"
              autocomplete="off"
              class="palette-input"
              @input="onInput"
              @keydown="onKeydown"
            />
            <kbd class="palette-kbd" aria-hidden="true">Esc</kbd>
          </div>

          <div v-if="suggestions.length > 0" class="palette-list" role="listbox" aria-label="搜索建议">
            <button
              v-for="(suggestion, index) in suggestions"
              :key="suggestion.slug"
              type="button"
              class="palette-item"
              :class="{ 'is-active': index === activeIndex }"
              role="option"
              :aria-selected="index === activeIndex"
              @click="goTo(suggestion)"
              @mouseenter="activeIndex = index"
            >
              <HighlightText :text="suggestion.title" :keywords="[keyword.trim()]" />
            </button>
          </div>
          <div v-else-if="keyword.trim()" class="palette-empty">无匹配文章</div>

          <div class="palette-hint">
            <span>Enter 搜索全文</span>
            <span>⌘/Ctrl+Enter 问 AI</span>
            <span>↑↓ 选择 · Esc 关闭</span>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.palette-backdrop {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 12vh 1rem 1rem;
  background: rgb(0 0 0 / 0.5);
}

.palette-panel {
  width: min(34rem, 100%);
  overflow: hidden;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 0.75rem;
  box-shadow: 0 12px 40px rgb(0 0 0 / 0.2);
}

.palette-input-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--border);
}

.palette-icon {
  width: 1rem;
  height: 1rem;
  color: var(--muted-foreground);
}

.palette-input {
  flex: 1;
  min-width: 0;
  font-size: 0.9375rem;
  color: var(--foreground);
  background: transparent;
  border: 0;
  outline: none;
}

.palette-input::placeholder {
  color: var(--muted-foreground);
}

.palette-kbd {
  padding: 0 0.375rem;
  font-size: 0.625rem;
  color: var(--muted-foreground);
  border: 1px solid var(--border);
  border-radius: 0.25rem;
}

.palette-list {
  max-height: 18rem;
  padding: 0.25rem 0;
  overflow-y: auto;
}

.palette-item {
  display: block;
  width: 100%;
  padding: 0.5rem 1rem;
  font-size: 0.875rem;
  text-align: left;
  cursor: pointer;
  background: transparent;
  border: 0;
  color: var(--foreground);
}

.palette-item.is-active {
  background: var(--muted);
}

.palette-empty {
  padding: 1.25rem 1rem;
  font-size: 0.875rem;
  text-align: center;
  color: var(--muted-foreground);
}

.palette-hint {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  padding: 0.5rem 1rem;
  font-size: 0.625rem;
  color: var(--muted-foreground);
  border-top: 1px solid var(--border);
}

.palette-enter-active,
.palette-leave-active {
  transition: opacity 0.2s ease;
}

.palette-enter-active .palette-panel,
.palette-leave-active .palette-panel {
  transition: transform 0.2s ease;
}

.palette-enter-from,
.palette-leave-to {
  opacity: 0;
}

.palette-enter-from .palette-panel,
.palette-leave-to .palette-panel {
  transform: translateY(-12px);
}
</style>
