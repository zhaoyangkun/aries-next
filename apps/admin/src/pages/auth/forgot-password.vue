<script setup lang="ts">
import { CheckCircle2Icon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import { authApi, getApiError } from '@/modules/auth/api/auth'
import FieldMessage from '@/modules/auth/components/FieldMessage.vue'
import { validateEmail } from '@/modules/auth/validation'

import AuthShell from './components/auth-shell.vue'

const email = ref('')
const loading = ref(false)
const sent = ref(false)
const emailError = ref('')
const serverError = ref('')

async function submit() {
  emailError.value = validateEmail(email.value)
  if (emailError.value)
    return

  loading.value = true
  serverError.value = ''
  try {
    await authApi.forgotPassword(email.value)
    sent.value = true
  }
  catch (requestError) {
    serverError.value = getApiError(requestError, '请求失败，请稍后重试')
  }
  finally {
    loading.value = false
  }
}
</script>

<template>
  <AuthShell title="找回密码" description="输入管理员邮箱以申请密码重置。">
    <div v-if="sent" class="grid justify-items-center gap-5 py-4 text-center">
      <CheckCircle2Icon class="size-10 text-primary" />
      <div>
        <p class="font-medium">请求已受理</p>
        <p class="mt-1 text-sm leading-6 text-muted-foreground">如果账号存在，系统会按当前 Email 配置处理重置说明。</p>
      </div>
      <Button class="w-full" as-child>
        <RouterLink to="/auth/sign-in">返回登录</RouterLink>
      </Button>
    </div>
    <form v-else class="grid gap-5" novalidate @submit.prevent="submit">
      <div class="grid gap-2">
        <label for="email" class="text-sm font-medium leading-none">管理员邮箱</label>
        <Input id="email" v-model="email" type="email" autocomplete="email" placeholder="owner@example.com" :disabled="loading" :aria-invalid="Boolean(emailError)" @update:model-value="emailError = ''" />
        <FieldMessage :message="emailError" error />
      </div>
      <FieldMessage :message="serverError" error />
      <Button class="w-full" type="submit" :disabled="loading">
        <Spinner v-if="loading" class="mr-2" />
        {{ loading ? '正在提交' : '继续' }}
      </Button>
      <p class="text-center text-xs text-muted-foreground">
        <RouterLink to="/auth/sign-in" class="font-medium text-foreground underline-offset-4 hover:underline">返回登录</RouterLink>
      </p>
    </form>
  </AuthShell>
</template>
