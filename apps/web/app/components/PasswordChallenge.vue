<script setup lang="ts">
// 密码文章解锁表单：成功后后端下发 HttpOnly Cookie，整页刷新让 SSR 携带 Cookie 重新渲染正文
const props = defineProps<{ slug: string }>()

const password = ref('')
const pending = ref(false)
const errorMessage = ref('')

async function submit() {
  if (!password.value || pending.value) return
  pending.value = true
  errorMessage.value = ''
  try {
    await postPublicApi(`/articles/${props.slug}/access`, { password: password.value })
    window.location.reload()
  } catch (error) {
    const statusCode = (error as { statusCode?: number }).statusCode
    if (statusCode === 401) errorMessage.value = '密码错误，请重试'
    else if (statusCode === 429) errorMessage.value = '尝试过于频繁，请稍后再试'
    else errorMessage.value = '解锁失败，请稍后再试'
    pending.value = false
  }
}
</script>

<template>
  <div class="mx-auto mt-14 max-w-sm rounded-lg border bg-card p-8 text-center">
    <p class="text-lg font-semibold">这是一篇受保护的文章</p>
    <p class="mt-2 text-sm text-muted-foreground">请输入访问密码后继续阅读</p>
    <form class="mt-6 flex flex-col gap-3" @submit.prevent="submit">
      <input
        v-model="password"
        type="password"
        required
        autocomplete="off"
        placeholder="访问密码"
        aria-label="访问密码"
        class="w-full rounded-md border bg-background px-3 py-2 text-sm outline-none focus:border-primary"
      />
      <p v-if="errorMessage" class="m-0 text-sm text-destructive" role="alert">{{ errorMessage }}</p>
      <button
        type="submit"
        :disabled="pending || !password"
        class="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-opacity disabled:cursor-not-allowed disabled:opacity-50"
      >
        {{ pending ? '验证中…' : '解锁阅读' }}
      </button>
    </form>
  </div>
</template>
