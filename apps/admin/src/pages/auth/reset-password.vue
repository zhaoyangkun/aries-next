<script setup lang="ts">
import { CheckCircle2Icon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { authApi, getApiError } from '@/modules/auth/api/auth'
import FieldMessage from '@/modules/auth/components/FieldMessage.vue'
import PasswordField from '@/modules/auth/components/PasswordField.vue'
import { validatePassword } from '@/modules/auth/validation'

import AuthShell from './components/auth-shell.vue'

const route = useRoute()
const router = useRouter()
const token = computed(() => String(route.query.token ?? ''))
const password = ref('')
const loading = ref(false)
const done = ref(false)
const passwordError = ref('')
const serverError = ref('')

async function submit() {
  passwordError.value = validatePassword(password.value)
  if (passwordError.value)
    return
  if (!token.value) {
    serverError.value = '重置链接无效，请重新申请'
    return
  }
  loading.value = true
  serverError.value = ''
  try {
    await authApi.resetPassword(token.value, password.value)
    done.value = true
    window.setTimeout(() => router.replace('/auth/sign-in'), 1200)
  }
  catch (requestError) {
    serverError.value = getApiError(requestError, '重置失败，请重新申请链接')
  }
  finally {
    loading.value = false
  }
}
</script>

<template>
  <AuthShell title="设置新密码" description="更新后，其他已登录设备将立即退出。">
    <div v-if="done" class="grid justify-items-center gap-5 py-4 text-center">
      <CheckCircle2Icon class="size-10 text-primary" />
      <div>
        <p class="font-medium">密码已更新</p>
        <p class="mt-1 text-sm text-muted-foreground">正在返回登录页面。</p>
      </div>
      <Button class="w-full" as-child>
        <RouterLink to="/auth/sign-in">立即登录</RouterLink>
      </Button>
    </div>
    <form v-else class="grid gap-5" novalidate @submit.prevent="submit">
      <PasswordField
        id="password"
        v-model="password"
        label="新密码"
        autocomplete="new-password"
        hint="使用 10 至 128 个字符，并同时包含字母和数字。"
        :disabled="loading"
        :error="passwordError"
        :minlength="10"
        :maxlength="128"
        @update:model-value="passwordError = ''"
      />
      <FieldMessage :message="serverError" error />
      <Button class="w-full" type="submit" :disabled="loading">
        <Spinner v-if="loading" class="mr-2" />
        {{ loading ? '正在更新' : '更新密码' }}
      </Button>
    </form>
  </AuthShell>
</template>
