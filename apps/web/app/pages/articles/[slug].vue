<script setup lang="ts">
import type { PublicArticleDetail } from '~/composables/usePublicApi'

const route = useRoute()
const site = await useSite()
const siteUrl = useSiteUrl()

const slug = computed(() => String(route.params.slug))

const { data: article, status } = await usePublicApi<PublicArticleDetail>(
  'article-detail',
  () => `/articles/${slug.value}`,
  {
    watch: [slug],
    // 密码文章解锁后浏览器持有 HttpOnly Cookie，SSR 需透传才能拿到正文
    forwardCookies: true,
  },
)

const locked = computed(
  () => !!article.value && article.value.password_protected && !article.value.rendered_html,
)

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
      <ReadingProgress />
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

        <img
          v-if="article.cover_url"
          :src="article.cover_url"
          :alt="article.title"
          class="mt-8 w-full rounded-lg object-cover"
        />

        <!-- 后端已完成 comrak 渲染与 ammonia 消毒，可直接输出 -->
        <HighlightedContent :html="article.rendered_html" />

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
    </div>
  </template>
</template>
