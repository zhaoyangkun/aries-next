<script setup lang="ts">
import type { PublicArticlePage } from '~/composables/usePublicApi'
import { parseKeywords } from '~/utils/highlight'

// 问 AI（SSE）：局部类型，待 @aries/api-client 统一生成后迁移
interface AskSource {
  slug: string
  title: string
}

type SearchMode = 'articles' | 'ask'

const route = useRoute()

const q = computed(() => String(route.query.q ?? '').trim())
const page = computed(() => Math.max(1, Number(route.query.page) || 1))
const keyword = ref(q.value)
// 浏览器回退/前进时同步搜索框
watch(q, (value) => {
  keyword.value = value
})

const hasKeyword = computed(() => q.value.length > 0)
// 高亮用关键词列表（空格分词、去重）
const keywords = computed(() => parseKeywords(q.value))

const { data, status } = await usePublicApi<PublicArticlePage>(
  'search-results',
  '/search',
  { query: { q, page }, watch: [q, page], enabled: hasKeyword },
)
// 标签表供结果卡片的标签 chips 查名
const { resolveTags } = await useTags()

function submit() {
  const value = keyword.value.trim()
  navigateTo({ path: '/search', query: value ? { q: value } : {} })
}

// —— 问 AI：纯客户端交互（点击触发 POST + SSE），不做 SSR 预取 ——
const mode = ref<SearchMode>(route.query.tab === 'ai' ? 'ask' : 'articles')
// 支持 /search?q=xxx&tab=ai 直达问 AI（全局搜索弹层 ⌘/Ctrl+Enter 跳转到此）
watch(
  () => route.query.tab,
  (tab) => {
    mode.value = tab === 'ai' ? 'ask' : 'articles'
  },
)
const question = ref('')
const answer = ref('')
const sources = ref<AskSource[]>([])
const asking = ref(false)
const askError = ref('')
let askController: AbortController | null = null

function parseEventData<T>(data: string): T | null {
  try {
    return JSON.parse(data) as T
  } catch {
    return null
  }
}

async function submitAsk() {
  const value = question.value.trim()
  if (!value || asking.value) return
  asking.value = true
  askError.value = ''
  answer.value = ''
  sources.value = []
  askController = new AbortController()
  try {
    const response = await fetch('/api/public/search/ask', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ question: value }),
      signal: askController.signal,
    })
    if (!response.ok || !response.body) {
      askError.value =
        response.status === 404
          ? '站点未开启 AI 问答'
          : response.status === 429
            ? '提问太频繁，请稍后再试'
            : 'AI 回答失败，请稍后再试'
      return
    }
    for await (const event of readSseEvents(response.body)) {
      if (event.event === 'delta') {
        const payload = parseEventData<{ text?: string }>(event.data)
        if (payload?.text) answer.value += payload.text
      }
      else if (event.event === 'sources') {
        const payload = parseEventData<{ items?: AskSource[] }>(event.data)
        sources.value = payload?.items ?? []
      }
      else if (event.event === 'error') {
        askError.value = 'AI 回答失败，请稍后再试'
        break
      }
      else if (event.event === 'done') {
        break
      }
      // start / usage 事件无展示需求，忽略
    }
  } catch (error) {
    // AbortError：用户点击停止，保留已生成的部分内容
    if ((error as Error).name !== 'AbortError') {
      askError.value = 'AI 回答失败，请稍后再试'
    }
  } finally {
    asking.value = false
    askController = null
  }
}

function stopAsk() {
  askController?.abort()
}

useSeoMeta({
  title: () => (q.value ? `搜索「${q.value}」` : '搜索'),
  description: '站内文章搜索',
  // 搜索结果页是动态薄内容，不参与索引
  robots: 'noindex',
})
useCanonical('/search')
</script>

<template>
  <PageHero title="搜索" label="站内搜索" />

  <form class="mt-8 flex max-w-xl gap-2" role="search" @submit.prevent="submit">
    <input
      v-model="keyword"
      type="search"
      maxlength="100"
      placeholder="输入关键词搜索文章…"
      aria-label="搜索关键词"
      class="min-w-0 flex-1 rounded-md border bg-background px-3 py-2 text-sm outline-none transition-colors focus:border-primary"
    />
    <button
      type="submit"
      class="shrink-0 cursor-pointer rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-opacity hover:opacity-90"
    >搜索</button>
  </form>

  <!-- 文章搜索 / 问 AI 切换 -->
  <div class="mt-4 flex gap-2" role="tablist" aria-label="搜索方式">
    <button
      type="button"
      role="tab"
      :aria-selected="mode === 'articles'"
      class="cursor-pointer rounded-md px-3 py-1.5 text-sm transition-colors"
      :class="
        mode === 'articles'
          ? 'bg-primary/10 font-medium text-primary'
          : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground'
      "
      @click="mode = 'articles'"
    >文章</button>
    <button
      type="button"
      role="tab"
      :aria-selected="mode === 'ask'"
      class="cursor-pointer rounded-md px-3 py-1.5 text-sm transition-colors"
      :class="
        mode === 'ask'
          ? 'bg-primary/10 font-medium text-primary'
          : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground'
      "
      @click="mode = 'ask'"
    >问 AI</button>
  </div>

  <!-- 问 AI：纯客户端 SSE 交互 -->
  <section v-if="mode === 'ask'" class="prose-page mt-8" aria-label="问 AI">
    <form class="flex max-w-xl gap-2" @submit.prevent="submitAsk">
      <input
        v-model="question"
        type="text"
        maxlength="200"
        placeholder="用一句话描述你想了解的问题…"
        aria-label="向 AI 提问"
        class="min-w-0 flex-1 rounded-md border bg-background px-3 py-2 text-sm outline-none transition-colors focus:border-primary"
      />
      <button
        v-if="!asking"
        type="submit"
        :disabled="!question.trim()"
        class="shrink-0 cursor-pointer rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
      >提问</button>
      <button
        v-else
        type="button"
        class="shrink-0 cursor-pointer rounded-md border px-4 py-2 text-sm transition-colors hover:bg-accent"
        @click="stopAsk"
      >停止</button>
    </form>

    <p v-if="askError" class="mt-6 rounded-md bg-primary/10 px-4 py-2.5 text-sm text-primary">{{ askError }}</p>
    <div v-if="asking && !answer" class="mt-6 text-sm text-muted-foreground">思考中…</div>

    <template v-if="answer">
      <!-- delta 为纯文本（引用形如 [1]），按原文换行展示 -->
      <p class="mt-6 whitespace-pre-wrap text-sm leading-relaxed">{{ answer }}</p>
      <span v-if="asking" class="mt-1 inline-block h-4 w-1.5 animate-pulse bg-primary" aria-hidden="true" />
    </template>

    <section v-if="sources.length > 0" class="mt-8 border-t pt-6" aria-label="参考文章">
      <h2 class="m-0 text-sm font-semibold text-muted-foreground">参考文章</h2>
      <ul class="m-0 mt-3 list-none space-y-2 p-0">
        <li v-for="item in sources" :key="item.slug">
          <NuxtLink
            :to="`/articles/${item.slug}`"
            class="text-sm text-primary underline-offset-4 hover:underline"
          >{{ item.title }}</NuxtLink>
        </li>
      </ul>
    </section>
  </section>

  <section v-else class="article-list" aria-label="搜索结果" aria-live="polite">
    <div v-if="!hasKeyword" class="empty-articles">输入关键词开始搜索</div>
    <div v-else-if="status === 'pending'" class="empty-articles">搜索中…</div>
    <div v-else-if="!data || data.items.length === 0" class="empty-articles">
      没有找到与「{{ q }}」相关的文章
    </div>
    <template v-else>
      <p class="m-0 text-sm text-muted-foreground">
        共找到 {{ data.total }} 篇与「{{ q }}」相关的文章
      </p>
      <ArticleCard
        v-for="article in data.items"
        :key="article.id"
        :article="article"
        :tags="resolveTags(article.tag_ids)"
        :keywords="keywords"
      />
      <AppPagination :page="data.page" :page-size="data.page_size" :total="data.total" />
    </template>
  </section>
</template>
