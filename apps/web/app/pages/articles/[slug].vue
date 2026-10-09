<script setup lang="ts">
import type { PublicArticleDetail } from '~/composables/usePublicApi'

// 后端并行实现中的新契约字段：AI 导读（未生成时为 null）。
// @aries/api-client 重新生成后此交叉类型应并入 PublicArticleDetail
type ArticleDetailWithAi = PublicArticleDetail & { ai_brief?: string | null }

// GET /articles/{slug}/related?limit=6 的条目
interface RelatedArticleItem {
  slug: string
  title: string
  cover_url: string | null
  published_at: string | null
}

const route = useRoute()
const site = await useSite()
const siteUrl = useSiteUrl()

const slug = computed(() => String(route.params.slug))

const { data: article, status } = await usePublicApi<ArticleDetailWithAi>(
  'article-detail',
  () => `/articles/${slug.value}`,
  {
    watch: [slug],
    // 密码文章解锁后浏览器持有 HttpOnly Cookie，SSR 需透传才能拿到正文
    forwardCookies: true,
  },
)

// 相关阅读：功能未开启时后端 404（AI_RETRIEVAL_DISABLED），ignoreError 兜底不渲染区块，
// 也不能让整页落入 404
const { data: related } = await usePublicApi<RelatedArticleItem[]>(
  'article-related',
  () => `/articles/${slug.value}/related`,
  { query: { limit: 6 }, watch: [slug], ignoreError: true },
)
const relatedArticles = computed(() => related.value ?? [])

const locked = computed(
  () => !!article.value && article.value.password_protected && !article.value.rendered_html,
)

// 阅读时长/字数：由后端渲染好的正文 HTML 统计（公开 API 不含 markdown_source），
// SSR 与客户端输入一致，无 hydration 差异
const reading = computed(() => readingTimeFromHtml(article.value?.rendered_html))

// 浏览量上报：每会话每篇只上报一次，服务端另有 30 分钟滑动窗口去重
watch(
  () => article.value?.slug,
  (currentSlug) => {
    if (!currentSlug || !import.meta.client) return
    const marker = `aries_viewed_${currentSlug}`
    if (sessionStorage.getItem(marker)) return
    sessionStorage.setItem(marker, '1')
    postPublicApi(`/articles/${currentSlug}/views`).catch(() => {
      // 浏览量上报失败不影响阅读，清除标记允许下次重试
      sessionStorage.removeItem(marker)
    })
  },
  { immediate: true },
)

const coverImage = computed(() =>
  toAbsoluteUrl(article.value?.cover_url || site.value.default_cover_url, siteUrl.value),
)

useSeoMeta({
  title: () => article.value?.title ?? '',
  description: () => article.value?.summary || site.value.site_description,
  ogTitle: () => article.value?.title,
  ogDescription: () => article.value?.summary,
  ogType: 'article',
  ogImage: coverImage,
  articlePublishedTime: () => article.value?.published_at ?? undefined,
})
useCanonical(computed(() => `/articles/${slug.value}`))

// BlogPosting 结构化数据；'<' 转义防止内容截断 script 标签
useHead(
  computed(() => {
    const a = article.value
    if (!a) return {}
    const jsonLd = {
      '@context': 'https://schema.org',
      '@type': 'BlogPosting',
      headline: a.title,
      description: a.summary,
      image: coverImage.value ? [coverImage.value] : undefined,
      datePublished: a.published_at ?? undefined,
      dateModified: a.updated_at,
      keywords: a.seo_keywords.join(', '),
      mainEntityOfPage: siteUrl.value
        ? { '@type': 'WebPage', '@id': `${siteUrl.value}/articles/${a.slug}` }
        : undefined,
    }
    return {
      meta:
        a.seo_keywords.length > 0
          ? [{ name: 'keywords', content: a.seo_keywords.join(', ') }]
          : [],
      script: [
        {
          type: 'application/ld+json',
          innerHTML: JSON.stringify(jsonLd).replace(/</g, '\\u003c'),
        },
      ],
    }
  }),
)
</script>

<template>
  <div v-if="status === 'pending'" class="empty-articles">加载中…</div>

  <template v-else-if="article">
    <PasswordChallenge v-if="locked" :slug="article.slug" />

    <div v-else class="xl:flex xl:items-start xl:gap-12">
      <article class="min-w-0 max-w-3xl flex-1">
        <header>
          <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span
              v-if="article.is_pinned"
              class="rounded-sm bg-primary/10 px-1.5 py-0.5 font-medium text-primary"
            >置顶</span>
            <NuxtLink
              v-if="article.category"
              :to="`/categories/${article.category.slug}`"
              class="rounded-sm bg-muted px-1.5 py-0.5 no-underline hover:bg-accent hover:text-accent-foreground"
            >{{ article.category.name }}</NuxtLink>
          </div>
          <h1 class="mt-3 text-4xl font-semibold leading-tight tracking-tight sm:text-5xl">{{ article.title }}</h1>
          <div class="mt-4 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
            <time v-if="article.published_at" :datetime="article.published_at">
              发布于 {{ formatDate(article.published_at) }}
            </time>
            <span aria-hidden="true" class="text-border">·</span>
            <span>{{ article.visit_count }} 次阅读</span>
            <span aria-hidden="true" class="text-border">·</span>
            <span>约 {{ reading.minutes }} 分钟 · {{ reading.count }} 字</span>
            <template v-if="article.comment_count > 0">
              <span aria-hidden="true" class="text-border">·</span>
              <span>{{ article.comment_count }} 条评论</span>
            </template>
          </div>
          <div v-if="article.tags.length > 0" class="mt-3 flex flex-wrap gap-2">
            <NuxtLink
              v-for="tag in article.tags"
              :key="tag.id"
              :to="`/tags/${tag.slug}`"
              class="rounded-full border px-2.5 py-0.5 text-xs text-muted-foreground no-underline hover:border-primary hover:text-primary"
            >{{ tag.name }}</NuxtLink>
          </div>
        </header>

        <!-- 详情封面：原来直出原图，改为 ?w= 缩略图 + srcset（正文栏最大 768px）；
             view-transition-name 与列表卡片封面配对，实现共享元素过渡。
             公开 API 无封面尺寸元数据，用 aspect-ratio: auto 21/9 折衷防 CLS：
             加载前按 21/9 预留高度，加载后 auto 让位给图片固有比例，
             CLS 从「0 → 全高」缩减为「21/9 → 实际比例」的小幅校正；
             此处高度始终由 aspect-ratio 决定，object-cover 不产生裁切，仅为过渡配对保留 -->
        <img
          v-if="article.cover_url"
          :src="thumbUrl(article.cover_url, 768)"
          :srcset="thumbSrcset(article.cover_url, [480, 768, 1200])"
          sizes="(max-width: 800px) calc(100vw - 2rem), 768px"
          :alt="article.title"
          :style="{ viewTransitionName: `cover-${article.id}` }"
          class="mt-8 aspect-[auto_21/9] w-full rounded-lg object-cover"
        />

        <!-- AI 导读：后端生成摘要式导读，非空时展示在正文之前 -->
        <div
          v-if="article.ai_brief"
          class="mt-8 border-l-2 border-primary bg-muted/50 px-4 py-3"
        >
          <p class="m-0 text-xs font-medium tracking-wide text-muted-foreground">AI 导读</p>
          <p class="m-0 mt-1.5 text-sm leading-relaxed">{{ article.ai_brief }}</p>
        </div>

        <!-- 后端已完成 comrak 渲染与 ammonia 消毒，可直接输出 -->
        <HighlightedContent :html="article.rendered_html ?? ''" />

        <!-- 相关阅读：未开启或 Embedding 未配置时后端 404、无相近文章时为空数组，两者都不渲染 -->
        <section v-if="relatedArticles.length > 0" class="mt-14 border-t pt-10" aria-label="相关阅读">
          <h2 class="m-0 text-lg font-semibold">相关阅读</h2>
          <div class="mt-5 grid gap-x-8 gap-y-4 sm:grid-cols-2">
            <NuxtLink
              v-for="item in relatedArticles"
              :key="item.slug"
              :to="`/articles/${item.slug}`"
              class="group flex min-w-0 items-center gap-3 no-underline"
            >
              <img
                v-if="item.cover_url"
                :src="thumbUrl(item.cover_url, 160)"
                :srcset="thumbSrcset(item.cover_url, [160, 320])"
                sizes="80px"
                :alt="item.title"
                loading="lazy"
                class="h-14 w-20 shrink-0 rounded-md object-cover"
              />
              <span class="min-w-0 flex-1 truncate text-sm font-medium text-title transition-colors group-hover:text-primary">{{ item.title }}</span>
            </NuxtLink>
          </div>
        </section>

        <nav
          v-if="article.previous || article.next"
          class="mt-14 grid gap-8 border-t pt-10 sm:grid-cols-2"
          aria-label="上一篇 / 下一篇"
        >
          <NuxtLink
            v-if="article.previous"
            :to="`/articles/${article.previous.slug}`"
            class="group block min-w-0 no-underline"
            rel="prev"
          >
            <span class="text-xs tracking-wide text-muted-foreground">← 上一篇</span>
            <span class="mt-2 block truncate text-lg font-semibold text-title transition-colors group-hover:text-primary">{{ article.previous.title }}</span>
          </NuxtLink>
          <NuxtLink
            v-if="article.next"
            :to="`/articles/${article.next.slug}`"
            class="group block min-w-0 no-underline sm:text-right"
            rel="next"
          >
            <span class="text-xs tracking-wide text-muted-foreground">下一篇 →</span>
            <span class="mt-2 block truncate text-lg font-semibold text-title transition-colors group-hover:text-primary">{{ article.next.title }}</span>
          </NuxtLink>
        </nav>

        <!-- 密码文章在上方 locked 分支拦截，走到这里说明允许展示评论 -->
        <section class="mt-14 border-t pt-8" aria-label="评论">
          <CommentSection
            v-if="article.allow_comments"
            target-type="article"
            :target-slug="article.slug"
            :list-path="`/articles/${article.slug}/comments`"
          />
          <p v-else class="m-0 text-sm text-muted-foreground">评论已关闭</p>
        </section>
      </article>

      <aside class="mt-10 hidden w-56 shrink-0 xl:sticky xl:top-24 xl:mt-0 xl:block">
        <ArticleToc />
      </aside>

      <!-- 移动端目录抽屉：由浮动工具栏在 xl 以下视口唤出 -->
      <TocDrawer />
    </div>
  </template>
</template>
