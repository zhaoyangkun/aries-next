<script setup lang="ts">
// 文章列表条目：无卡片包裹，列表分隔线由 .article-list 提供；
// 结构为 meta 行（日期/置顶/分类/标签）+ 标题 + 摘要 + 阅读全文，封面缩略图居右
import type { PublicArticleListItem, PublicTaxonomyRef } from '~/composables/usePublicApi'

const props = defineProps<{
  article: PublicArticleListItem
  /** 列表项只有 category_id，名称由页面侧查表后传入 */
  category?: PublicTaxonomyRef | null
  /** 列表项只有 tag_ids，名称同样由页面侧查表后传入 */
  tags?: PublicTaxonomyRef[]
  /** 搜索关键词：传入时标题/摘要/命中片段中的关键词高亮（仅搜索页使用） */
  keywords?: string[]
}>()

const dateText = computed(() => formatDate(props.article.published_at))
</script>

<template>
  <article class="post-item">
    <div class="post-item-body">
      <div class="post-meta">
        <time v-if="dateText" class="post-item-date">{{ dateText }}</time>
        <span v-if="article.is_pinned" class="badge-pinned">置顶</span>
        <span v-if="article.password_protected" class="badge-locked">加密</span>
        <NuxtLink
          v-if="category"
          :to="`/categories/${category.slug}`"
          class="post-chip"
        >{{ category.name }}</NuxtLink>
        <NuxtLink
          v-for="tag in tags"
          :key="tag.id"
          :to="`/tags/${tag.slug}`"
          class="post-chip"
        >{{ tag.name }}</NuxtLink>
      </div>
      <h2 class="post-title">
        <NuxtLink :to="`/articles/${article.slug}`">
          <HighlightText :text="article.title" :keywords />
        </NuxtLink>
      </h2>
      <!-- 摘要未覆盖的正文命中由后端给出纯文本片段，优先于摘要展示 -->
      <p v-if="article.matched_excerpt" class="post-summary">
        <HighlightText :text="article.matched_excerpt" :keywords />
      </p>
      <p v-else-if="article.summary" class="post-summary">
        <HighlightText :text="article.summary" :keywords />
      </p>
      <NuxtLink :to="`/articles/${article.slug}`" class="post-more">
        阅读全文
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M5 12h14" />
          <path d="m12 5 7 7-7 7" />
        </svg>
      </NuxtLink>
    </div>
    <NuxtLink
      v-if="article.cover_url"
      :to="`/articles/${article.slug}`"
      class="post-thumb"
      :aria-label="article.title"
    >
      <!-- 列表展示面小：本地托管图片走 ?w= 按需缩略图，外部地址用原图 -->
      <img :src="thumbUrl(article.cover_url, 480)" :alt="article.title" loading="lazy" />
    </NuxtLink>
  </article>
</template>
