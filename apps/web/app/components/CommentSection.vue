<script setup lang="ts">
// 评论区（Twikoo 式体验，完全自建）：根评论分页列表（两级树）+ 表单 + 内联回复。
// listPath 为空时只渲染表单——公开端目前只有文章有评论列表接口，页面（page）只能提交
import type { PublicComment, PublicCommentPage } from '~/composables/usePublicApi'

const props = defineProps<{
  targetType: 'article' | 'page'
  targetSlug: string
  /** 评论列表接口路径（不含 baseURL）；不传则跳过列表请求 */
  listPath?: string
}>()

const route = useRoute()
const page = computed(() => Math.max(1, Number(route.query.page) || 1))

// 页面无列表接口时用 enabled 跳过请求（data 为 null），与普通条件请求同一语义
const listEnabled = computed(() => !!props.listPath)

const { data, status, refresh } = await usePublicApi<PublicCommentPage>(
  `comments-${props.targetType}-${props.targetSlug}`,
  () => props.listPath ?? '',
  { query: { page }, watch: [page], enabled: listEnabled },
)

// 内联回复目标；-1 表示没有展开中的回复表单
const replyTargetId = ref(-1)

// 提交结果提示（待审核 / 已发布），可手动关闭
const notice = ref('')

function handleReply(comment: PublicComment) {
  // 再点同一条的回复按钮时表单已被替换内容，直接切换目标即可
  replyTargetId.value = comment.id
}

// 审核制：pending 的评论不做乐观插入，只提示待审核；approved 立即刷新列表
function handleSubmitted(status: 'pending' | 'approved') {
  if (status === 'pending') {
    notice.value = '评论已提交，待博主审核通过后展示'
  }
  else {
    notice.value = '评论已发布'
    refresh()
  }
}

function handleReplied(status: 'pending' | 'approved') {
  replyTargetId.value = -1
  handleSubmitted(status)
}
</script>

<template>
  <div>
    <h2 class="m-0 text-lg font-semibold">
      评论<template v-if="data && data.total > 0">（{{ data.total }}）</template>
    </h2>

    <p
      v-if="notice"
      class="mt-4 flex items-center justify-between rounded-md bg-primary/10 px-4 py-2.5 text-sm text-primary"
    >
      <span>{{ notice }}</span>
      <button
        type="button"
        class="ml-3 shrink-0 text-xs underline-offset-4 hover:underline"
        aria-label="关闭提示"
        @click="notice = ''"
      >知道了</button>
    </p>

    <!-- 发表评论（根评论表单） -->
    <CommentForm
      class="mt-5"
      :target-type="targetType"
      :target-slug="targetSlug"
      @submitted="handleSubmitted"
    />

    <template v-if="listPath">
      <!-- 加载中骨架：脉冲占位块，避免布局跳动 -->
      <div v-if="status === 'pending'" class="mt-8 space-y-5" aria-hidden="true">
        <div v-for="i in 3" :key="i" class="flex items-start gap-3">
          <div class="h-9 w-9 shrink-0 animate-pulse rounded-full bg-muted" />
          <div class="flex-1 space-y-2">
            <div class="h-3 w-32 animate-pulse rounded bg-muted" />
            <div class="h-3 w-full animate-pulse rounded bg-muted" />
            <div class="h-3 w-2/3 animate-pulse rounded bg-muted" />
          </div>
        </div>
      </div>

      <div v-else-if="!data || data.items.length === 0" class="empty-articles mt-6">
        还没有评论，来抢沙发～
      </div>

      <template v-else>
        <ol class="mt-8 list-none space-y-6 border-t pt-6">
          <li v-for="comment in data.items" :key="comment.id">
            <CommentItem
              :comment="comment"
              :reply-target-id="replyTargetId"
              @reply="handleReply"
              @replied="handleReplied"
              @cancel-reply="replyTargetId = -1"
            />
          </li>
        </ol>
        <!-- 分页只作用于根评论（?page=N，与全站分页同一语义） -->
        <AppPagination :page="data.page" :page-size="data.page_size" :total="data.total" />
      </template>
    </template>
  </div>
</template>
