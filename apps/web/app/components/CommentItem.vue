<script setup lang="ts">
// 单条评论（含一级回复递归渲染）：头像加载失败退化为昵称首字符圆块；
// 回复表单内联展开在该评论下方，事件透传给 CommentSection 统一处理。
// 有 website 的访客昵称渲染为 nofollow ugc 外链（对齐 Twikoo 行为）
import type { PublicComment } from '~/composables/usePublicApi'

const props = defineProps<{
  comment: PublicComment
  /** 当前展开回复表单的评论 id（-1 表示无） */
  replyTargetId: number
  /** 一级回复用小缩进 + 竖线，避免小屏层层缩进挤爆 */
  depth?: number
}>()

const emit = defineEmits<{
  reply: [comment: PublicComment]
  replied: [status: 'pending' | 'approved']
  cancelReply: []
}>()

// 头像 URL 失效（Cravatar 不可达等）时退化为首字符占位
const avatarFailed = ref(false)
const showAvatarImage = computed(() => !!props.comment.avatar_url && !avatarFailed.value)

const isReplyTarget = computed(() => props.replyTargetId === props.comment.id)
</script>

<template>
  <div :class="depth ? 'mt-4 border-l-2 pl-3 sm:pl-4' : ''">
    <div class="flex items-start gap-3">
      <img
        v-if="showAvatarImage"
        :src="comment.avatar_url!"
        :alt="`${comment.nickname} 的头像`"
        width="36"
        height="36"
        loading="lazy"
        class="h-9 w-9 shrink-0 rounded-full object-cover"
        @error="avatarFailed = true"
      >
      <span
        v-else
        class="grid h-9 w-9 shrink-0 place-items-center rounded-full bg-muted text-sm font-semibold text-muted-foreground"
        aria-hidden="true"
      >{{ comment.nickname.charAt(0) }}</span>

      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <!-- 有 website 的访客昵称渲染为外链（Twikoo 行为）；nofollow ugc 声明用户生成内容，不传递权重 -->
          <a
            v-if="comment.website"
            :href="comment.website"
            target="_blank"
            rel="nofollow ugc noopener"
            class="text-sm font-medium text-foreground no-underline transition-colors hover:text-primary"
          >{{ comment.nickname }}</a>
          <span v-else class="text-sm font-medium">{{ comment.nickname }}</span>
          <span
            v-if="comment.is_admin"
            class="rounded-sm bg-primary/10 px-1.5 py-0.5 text-xs font-medium text-primary"
          >博主</span>
          <!-- 用确定性日期而非相对时间，避免 SSR 与客户端渲染时刻不同导致 hydration 不一致 -->
          <time :datetime="comment.created_at" class="text-xs text-muted-foreground">
            {{ formatDate(comment.created_at) }}
          </time>
        </div>

        <!-- 后端已渲染并消毒的 Markdown HTML -->
        <div class="article-content comment-content" v-html="comment.content_html" />

        <button
          type="button"
          class="mt-1 text-xs text-muted-foreground underline-offset-4 transition-colors hover:text-primary hover:underline"
          @click="emit('reply', comment)"
        >回复</button>

        <CommentForm
          v-if="isReplyTarget"
          :reply-to="{ id: comment.id, nickname: comment.nickname }"
          class="mt-3"
          @submitted="emit('replied', $event)"
          @cancel="emit('cancelReply')"
        />
      </div>
    </div>

    <!-- 回复的回复已由后端平铺到同一根下，这里最多递归一级 -->
    <CommentItem
      v-for="child in comment.children"
      :key="child.id"
      :comment="child"
      :reply-target-id="replyTargetId"
      :depth="(depth ?? 0) + 1"
      @reply="emit('reply', $event)"
      @replied="emit('replied', $event)"
      @cancel-reply="emit('cancelReply')"
    />
  </div>
</template>
