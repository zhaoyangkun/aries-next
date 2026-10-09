<script setup lang="ts">
// 正文容器：SSR 输出后端已消毒的 HTML，客户端挂载后对代码块做增强：
// 语法高亮（highlight.js）+ Mac 风格窗口栏（圆点/语言名）+ 行号 + 一键复制。
// 高亮不放服务端的原因：ammonia 消毒会剥离内联样式，syntect 高亮产物无法存活到浏览器。
// 数学公式（KaTeX）与 Mermaid 图表同样只在客户端渲染：
// 后端 comrak 输出 <span data-math-style="inline|display"> 与
// <pre><code class="language-math|language-mermaid"> 线索，这里按需动态加载渲染库。
import 'highlight.js/styles/atom-one-dark.css'
import type { LightboxImage } from '~/components/AppImageLightbox.vue'

// 多根节点（正文 div + 灯箱），调用方传入的 class（如 comment-content）需手动落到正文 div 上
defineOptions({ inheritAttrs: false })

// html 可空：加密文章未解锁时后端不返回正文
const props = defineProps<{ html: string | null }>()
const root = ref<HTMLElement | null>(null)

// 正文图片灯箱：images 与正文渲染同步重建，点击第 N 张即以 N 为初始索引打开
const zoomImages = ref<LightboxImage[]>([])
const lightboxOpen = ref(false)
const lightboxIndex = ref(0)

type Hljs = (typeof import('highlight.js/lib/common'))['default']
type HljsWithLineNumbers = Hljs & { lineNumbersBlock: (el: HTMLElement) => void }
type Mermaid = (typeof import('mermaid'))['default']

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

// ---------- 数学公式（KaTeX） ----------

async function renderMath() {
  if (!root.value) return
  // comrak math_dollars 产物：span 内容是剥离 $ 定界符后的原始 TeX
  const spans = Array.from(
    root.value.querySelectorAll<HTMLElement>('[data-math-style]:not([data-math-rendered])'),
  )
  // comrak math_code 产物：```math 代码块，按展示公式渲染并整体替换掉代码块
  const mathCodes = Array.from(
    root.value.querySelectorAll<HTMLElement>('pre:not([data-code-enhanced]) > code.language-math'),
  )
  if (spans.length === 0 && mathCodes.length === 0) return

  const katex = (await import('katex')).default
  await import('katex/dist/katex.min.css')

  for (const el of spans) {
    // throwOnError: false 时语法错误渲染为红色错误文本，保持可读
    katex.render(el.textContent ?? '', el, {
      displayMode: el.dataset.mathStyle === 'display',
      throwOnError: false,
    })
    el.setAttribute('data-math-rendered', '')
  }
  for (const code of mathCodes) {
    const pre = code.parentElement
    if (!(pre instanceof HTMLElement)) continue
    const container = document.createElement('div')
    katex.render(code.textContent ?? '', container, { displayMode: true, throwOnError: false })
    pre.replaceWith(container)
  }
}

// ---------- Mermaid 图表 ----------

// mermaid.render 要求每次调用 id 全局唯一，用模块级递增种子保证跨组件实例不重复
let mermaidIdSeed = 0

function isDarkTheme() {
  return document.documentElement.classList.contains('dark')
}

async function loadMermaid(): Promise<Mermaid> {
  const mermaid = (await import('mermaid')).default
  // securityLevel 保持 strict：评论等用户内容也可能携带 mermaid 代码块
  mermaid.initialize({
    startOnLoad: false,
    theme: isDarkTheme() ? 'dark' : 'default',
    securityLevel: 'strict',
  })
  return mermaid
}

async function renderMermaidDiagrams() {
  if (!root.value) return
  const codes = Array.from(
    root.value.querySelectorAll('pre:not([data-code-enhanced]) > code.language-mermaid'),
  )
  if (codes.length === 0) return
  const mermaid = await loadMermaid()

  for (const code of codes) {
    const pre = code.parentElement
    if (!(pre instanceof HTMLElement)) continue
    const source = code.textContent ?? ''
    const container = document.createElement('div')
    container.className = 'mermaid'
    // 原始源码存 data 属性，主题切换时据此重渲染
    container.dataset.source = source
    try {
      const { svg } = await mermaid.render(`mermaid-${mermaidIdSeed++}`, source)
      container.innerHTML = svg
      pre.replaceWith(container)
    }
    catch {
      // 渲染失败：保留原始 pre，交由 hljs 降级为普通代码块展示
    }
  }
}

// 主题切换（html.dark 翻转）时用对应主题重渲染已有图表
async function rerenderMermaidForTheme() {
  if (!root.value) return
  const diagrams = Array.from(root.value.querySelectorAll<HTMLElement>('.mermaid[data-source]'))
  if (diagrams.length === 0) return
  const mermaid = await loadMermaid()
  for (const container of diagrams) {
    try {
      const { svg } = await mermaid.render(`mermaid-${mermaidIdSeed++}`, container.dataset.source ?? '')
      container.innerHTML = svg
    }
    catch {
      // 重渲染失败保留旧 SVG，不影响阅读
    }
  }
}

let themeObserver: MutationObserver | null = null

async function enhance() {
  if (!root.value) return
  // mermaid / ```math 的 pre 先替换掉，剩下的 pre 才进 hljs 流程
  await renderMermaidDiagrams()
  await renderMath()
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

onMounted(() => {
  enhance()
  themeObserver = new MutationObserver(() => {
    rerenderMermaidForTheme()
  })
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
})
onBeforeUnmount(() => themeObserver?.disconnect())
// 客户端路由切换复用组件实例时 html 会变，需要重新增强
watch(() => props.html, async () => {
  await nextTick()
  await enhance()
})
</script>

<template>
  <div ref="root" class="article-content" v-bind="$attrs" v-html="html ?? ''" />
  <AppImageLightbox
    v-model:open="lightboxOpen"
    v-model:index="lightboxIndex"
    :images="zoomImages"
  />
</template>
