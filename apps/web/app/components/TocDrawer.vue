<script setup lang="ts">
// 移动端（< xl）目录 bottom-sheet：圆角上滑抽屉，章节点击平滑跳转后自动关闭。
// 目录数据与桌面侧栏 ArticleToc 共用 useArticleToc；正文标题锚点已被任一实例补齐。
const { open, hide } = useTocDrawer()
const { items, activeId } = useArticleToc()

// 抽屉打开期间锁定页面滚动
watch(open, (value) => {
  if (!import.meta.client) return
  document.body.style.overflow = value ? 'hidden' : ''
})

onBeforeUnmount(() => {
  if (!import.meta.client) return
  document.body.style.overflow = ''
})

function jump(id: string) {
  document.getElementById(id)?.scrollIntoView({ behavior: 'smooth' })
  hide()
}
</script>

<template>
  <Teleport to="body">
    <Transition name="toc-sheet">
      <div v-if="open" class="toc-sheet-backdrop" @click="hide">
        <div
          class="toc-sheet-panel"
          role="dialog"
          aria-label="文章目录"
          @click.stop
        >
          <div class="toc-sheet-handle" aria-hidden="true" />
          <p class="toc-sheet-title">目录</p>
          <ul v-if="items.length > 0" class="toc-sheet-list">
            <li v-for="item in items" :key="item.id">
              <a
                :href="`#${item.id}`"
                class="toc-sheet-link"
                :class="[
                  item.level === 3 ? 'pl-8' : 'pl-4',
                  activeId === item.id ? 'is-active' : '',
                ]"
                @click.prevent="jump(item.id)"
              >{{ item.text }}</a>
            </li>
          </ul>
          <p v-else class="toc-sheet-empty">本文没有目录</p>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.toc-sheet-backdrop {
  position: fixed;
  inset: 0;
  z-index: 55;
  background: rgb(0 0 0 / 0.5);
}

.toc-sheet-panel {
  position: absolute;
  right: 0;
  bottom: 0;
  left: 0;
  max-height: 70vh;
  padding: 0.25rem 0 1.5rem;
  overflow-y: auto;
  background: var(--card);
  border-top: 1px solid var(--border);
  border-radius: 1rem 1rem 0 0;
}

.toc-sheet-handle {
  width: 2.5rem;
  height: 0.25rem;
  margin: 0.5rem auto;
  background: var(--border);
  border-radius: 9999px;
}

.toc-sheet-title {
  margin: 0;
  padding: 0 1rem;
  font-size: 0.75rem;
  font-weight: 500;
  letter-spacing: 0.1em;
  color: var(--primary);
  text-transform: uppercase;
}

.toc-sheet-list {
  margin: 0.5rem 0 0;
  padding: 0;
  list-style: none;
}

.toc-sheet-link {
  display: block;
  padding-top: 0.5rem;
  padding-bottom: 0.5rem;
  font-size: 0.875rem;
  color: var(--muted-foreground);
  text-decoration: none;
  transition: color 0.2s ease;
}

.toc-sheet-link.is-active {
  font-weight: 500;
  color: var(--title);
}

.toc-sheet-empty {
  margin: 0.5rem 0 0;
  padding: 0 1rem;
  font-size: 0.875rem;
  color: var(--muted-foreground);
}

.toc-sheet-enter-active,
.toc-sheet-leave-active {
  transition: opacity 0.25s ease;
}

.toc-sheet-enter-active .toc-sheet-panel,
.toc-sheet-leave-active .toc-sheet-panel {
  transition: transform 0.25s ease;
}

.toc-sheet-enter-from,
.toc-sheet-leave-to {
  opacity: 0;
}

.toc-sheet-enter-from .toc-sheet-panel,
.toc-sheet-leave-to .toc-sheet-panel {
  transform: translateY(100%);
}

@media (prefers-reduced-motion: reduce) {
  .toc-sheet-enter-active,
  .toc-sheet-leave-active,
  .toc-sheet-enter-active .toc-sheet-panel,
  .toc-sheet-leave-active .toc-sheet-panel {
    transition: none;
  }
}
</style>
