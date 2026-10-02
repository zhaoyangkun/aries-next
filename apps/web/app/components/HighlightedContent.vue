<script setup lang="ts">
// 正文容器：SSR 输出后端已消毒的 HTML，客户端挂载后对代码块做增强：
// 语法高亮（highlight.js）+ Mac 风格窗口栏（圆点/语言名）+ 行号 + 一键复制。
// 高亮不放服务端的原因：ammonia 消毒会剥离内联样式，syntect 高亮产物无法存活到浏览器。
import 'highlight.js/styles/atom-one-dark.css'
import type { LightboxImage } from '~/components/AppImageLightbox.vue'

// html 可空：加密文章未解锁时后端不返回正文
const props = defineProps<{ html: string | null }>()
const root = ref<HTMLElement | null>(null)

// 正文图片灯箱：images 与正文渲染同步重建，点击第 N 张即以 N 为初始索引打开
const zoomImages = ref<LightboxImage[]>([])
const lightboxOpen = ref(false)
const lightboxIndex = ref(0)

type Hljs = (typeof import('highlight.js/lib/common'))['default']
type HljsWithLineNumbers = Hljs & { lineNumbersBlock: (el: HTMLElement) => void }

async function loadHighlighter(): Promise<HljsWithLineNumbers> {
  const hljs = (await import('highlight.js/lib/common')).default as HljsWithLineNumbers
  if (!hljs.lineNumbersBlock) {
    // highlightjs-line-numbers.js 是 UMD 插件，只在全局 hljs 上挂载能力
    const w = window as unknown as { hljs?: Hljs }
    w.hljs ??= hljs
    await import('highlightjs-line-numbers.js')
  }
  return hljs
}

// 行号插件会把代码拆成表格，复制用增强前捕获的原始文本，保证完整且换行不丢
async function copyCode(rawText: string, button: HTMLButtonElement) {
  try {
    await navigator.clipboard.writeText(rawText)
    button.textContent = '已复制'
  }
  catch {
    button.textContent = '复制失败'
  }
  setTimeout(() => {
    button.textContent = '复制'
  }, 1500)
}

function enhancePre(pre: HTMLElement, hljs: HljsWithLineNumbers) {
  pre.setAttribute('data-code-enhanced', '')
  const code = pre.querySelector('code')
  const rawText = code?.textContent ?? ''
  const language = code?.className.match(/language-(\w+)/)?.[1] ?? ''
  if (code instanceof HTMLElement) {
    if (!code.classList.contains('hljs')) hljs.highlightElement(code)
    hljs.lineNumbersBlock(code)
  }

  // 外包一层 Mac 风格窗口：三个圆点 + 语言名 + 复制按钮
  const wrap = document.createElement('div')
  wrap.className = 'code-window'
  const header = document.createElement('div')
  header.className = 'code-window-header'
  for (const color of ['#ff5f57', '#febc2e', '#28c840']) {
    const dot = document.createElement('span')
    dot.className = 'code-window-dot'
    dot.style.backgroundColor = color
    header.append(dot)
  }
  if (language) {
    const lang = document.createElement('span')
    lang.className = 'code-window-lang'
    lang.textContent = language
    header.append(lang)
  }
  const button = document.createElement('button')
  button.type = 'button'
  button.className = 'code-copy'
  button.textContent = '复制'
  button.addEventListener('click', () => copyCode(rawText, button))
  header.append(button)

  pre.parentNode?.insertBefore(wrap, pre)
  wrap.append(header, pre)
}

async function enhance() {
  if (!root.value) return
  const pres = root.value.querySelectorAll('pre:not([data-code-enhanced])')
  if (pres.length > 0) {
    // 动态导入，避免高亮库拖慢首屏
    const hljs = await loadHighlighter()
    pres.forEach((pre) => {
      if (pre instanceof HTMLElement) enhancePre(pre, hljs)
    })
  }
  bindImageZoom()
}

// 给正文图片挂点击放大：收集当前全部图片重建播放列表，点击时以自身索引打开灯箱。
// html 替换（客户端路由切换）后旧监听随 DOM 丢弃，这里统一重绑并重建列表
function bindImageZoom() {
  if (!root.value) return
  const imgs = Array.from(root.value.querySelectorAll('img'))
  zoomImages.value = imgs.map((img) => ({
    url: img.currentSrc || img.src,
    alt: img.alt || '',
  }))
  imgs.forEach((img, index) => {
    if (img.dataset.zoomBound) return
    img.dataset.zoomBound = '1'
    img.addEventListener('click', () => {
      lightboxIndex.value = index
      lightboxOpen.value = true
    })
  })
}

onMounted(enhance)
// 客户端路由切换复用组件实例时 html 会变，需要重新增强
watch(() => props.html, async () => {
  await nextTick()
  await enhance()
})
</script>

<template>
  <div ref="root" class="article-content" v-html="html ?? ''" />
  <AppImageLightbox
    v-model:open="lightboxOpen"
    v-model:index="lightboxIndex"
    :images="zoomImages"
  />
</template>
