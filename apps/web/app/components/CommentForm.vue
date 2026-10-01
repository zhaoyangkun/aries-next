<script setup lang="ts">
// 评论表单：根评论与回复共用。身份信息（昵称/邮箱/网址）存 localStorage 下次自动填（Twikoo 行为）；
// 提交路径按模式区分：根评论 POST /comments，回复 POST /comments/{id}/replies
import type { PublicCommentCreated } from '~/composables/usePublicApi'

const props = defineProps<{
  /** 根评论模式必填 */
  targetType?: 'article' | 'page'
  targetSlug?: string
  /** 回复模式：被回复评论的 id 与昵称 */
  replyTo?: { id: number, nickname: string }
}>()

const emit = defineEmits<{
  submitted: [status: 'pending' | 'approved']
  cancel: []
}>()

const IDENTITY_STORAGE_KEY = 'aries_comment_identity'

const nickname = ref('')
const email = ref('')
const website = ref('')
const content = ref('')
const submitting = ref(false)
const errorMessage = ref('')

// localStorage 仅客户端可用，onMounted 里读取避免 SSR 报错
onMounted(() => {
  try {
    const raw = localStorage.getItem(IDENTITY_STORAGE_KEY)
    if (!raw) return
    const saved = JSON.parse(raw) as { nickname?: string, email?: string, website?: string }
    nickname.value = saved.nickname ?? ''
    email.value = saved.email ?? ''
    website.value = saved.website ?? ''
  }
  catch {
    // 损坏的本地数据直接忽略，不阻塞评论
  }
})

const contentLength = computed(() => content.value.length)

// 前端先挡一轮明显错误，减少无效请求（服务端仍会完整校验）
function validate(): string | null {
  if (!nickname.value.trim()) return '请填写昵称'
  if (nickname.value.trim().length > 60) return '昵称最长 60 字'
  if (!email.value.trim()) return '请填写邮箱'
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.value.trim())) return '邮箱格式不正确'
  if (website.value.trim() && !/^https?:\/\//.test(website.value.trim()))
    return '网址只支持 http/https 链接'
  const length = content.value.trim().length
  if (length < 1) return '评论内容不能为空'
  if (length > 2000) return '评论内容最长 2000 字'
  return null
}

// 后端错误体为统一 {"error": {"code", "message"}} 结构；优先展示服务端文案，给常见 code 中文兜底
function resolveError(error: unknown): string {
  const data = (error as { data?: { error?: { code?: string, message?: string } } }).data?.error
  if (data?.code === 'COMMENTS_CLOSED') return '评论已关闭'
  if (data?.code === 'RATE_LIMITED') return data.message || '提交太频繁，请稍后再试'
  if (data?.code === 'COMMENT_DUPLICATE') return data.message || '相同内容已提交过，请勿重复发送'
  return data?.message || '提交失败，请稍后重试'
}

async function submit() {
  errorMessage.value = validate() ?? ''
  if (errorMessage.value) return
  if (submitting.value) return
  submitting.value = true

  // 字段形状对应 openapi 的 ReplyPublicCommentRequest；根评论再补 target_type/target_slug
  const base = {
    nickname: nickname.value.trim(),
    email: email.value.trim(),
    website: website.value.trim() || null,
    content: content.value.trim(),
  }

  try {
    const created = props.replyTo
      ? await postPublicApi<PublicCommentCreated>(`/comments/${props.replyTo.id}/replies`, base)
      : await postPublicApi<PublicCommentCreated>('/comments', {
          ...base,
          target_type: props.targetType,
          target_slug: props.targetSlug,
        })

    // 提交成功才持久化身份信息；清空内容框但保留身份（Twikoo 行为）
    try {
      localStorage.setItem(
        IDENTITY_STORAGE_KEY,
        JSON.stringify({ nickname: base.nickname, email: base.email, website: base.website ?? '' }),
      )
    }
    catch {
      // 隐私模式等写入失败不影响提交结果
    }
    content.value = ''
    emit('submitted', created.status)
  }
  catch (error) {
    errorMessage.value = resolveError(error)
  }
  finally {
    submitting.value = false
  }
}

const inputClass =
  'w-full rounded-md border bg-background px-3 py-2 text-sm outline-none transition-colors placeholder:text-muted-foreground focus:border-primary'
</script>

<template>
  <form class="rounded-lg border bg-card p-4" @submit.prevent="submit">
    <p v-if="replyTo" class="m-0 mb-3 flex items-center justify-between text-sm text-muted-foreground">
      <span>回复 <span class="font-medium text-foreground">@{{ replyTo.nickname }}</span></span>
      <button
        type="button"
        class="text-xs text-muted-foreground underline-offset-4 hover:text-primary hover:underline"
        @click="emit('cancel')"
      >取消回复</button>
    </p>

    <div class="grid gap-3 sm:grid-cols-3">
      <input v-model="nickname" :class="inputClass" type="text" placeholder="昵称 *" maxlength="60" aria-label="昵称">
      <input v-model="email" :class="inputClass" type="email" placeholder="邮箱 *（不会公开）" maxlength="254" aria-label="邮箱">
      <input v-model="website" :class="inputClass" type="url" placeholder="网址（可选）" maxlength="2048" aria-label="网址">
    </div>

    <textarea
      v-model="content"
      :class="[inputClass, 'mt-3 min-h-28 resize-y']"
      placeholder="写下你的评论… 支持 Markdown"
      maxlength="2000"
      aria-label="评论内容"
    />

    <div class="mt-3 flex flex-wrap items-center justify-between gap-2">
      <span class="text-xs tabular-nums text-muted-foreground">{{ contentLength }}/2000</span>
      <button
        type="submit"
        class="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        :disabled="submitting"
      >{{ submitting ? '提交中…' : '提交评论' }}</button>
    </div>

    <p v-if="errorMessage" class="m-0 mt-3 rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive">
      {{ errorMessage }}
    </p>
  </form>
</template>
