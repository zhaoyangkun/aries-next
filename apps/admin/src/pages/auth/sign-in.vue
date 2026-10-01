<script setup lang="ts">
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import { getApiError } from '@/modules/auth/api/auth'
import FieldMessage from '@/modules/auth/components/FieldMessage.vue'
import PasswordField from '@/modules/auth/components/PasswordField.vue'
import { useSessionStore } from '@/modules/auth/stores/session'
import { validateRequired } from '@/modules/auth/validation'

import AuthShell from './components/auth-shell.vue'

const router = useRouter()
const session = useSessionStore()
const loginName = ref('')
const password = ref('')
const loading = ref(false)
const loginError = ref('')
const passwordError = ref('')
const serverError = ref('')

async function submit() {
  loginError.value = validateRequired(loginName.value, '用户名或邮箱')
  passwordError.value = validateRequired(password.value, '密码')
  if (loginError.value || passwordError.value)
    return

  loading.value = true
  serverError.value = ''
  try {
    await session.login(loginName.value, password.value)
    const redirect = router.currentRoute.value.query.redirect as string
    // 防开放重定向：只接受站内路径。
    const target = redirect?.startsWith('/') && !redirect.startsWith('//') ? redirect : '/'
    await router.replace(target)
  }
  catch (requestError) {
    serverError.value = getApiError(requestError, '登录失败，请检查账号和密码')
  }
  finally {
    loading.value = false
  }
}
</script>

<template>
  <AuthShell title="欢迎回来" description="使用管理员账号登录 Aries。">
    <form class="grid gap-5" novalidate @submit.prevent="submit">
      <div class="grid gap-2">
        <label for="login" class="text-sm font-medium leading-none">用户名或邮箱</label>
        <Input
          id="login"
          v-model="loginName"
          autocomplete="username"
          placeholder="owner@example.com"
          :disabled="loading"
          :aria-invalid="Boolean(loginError)"
          :aria-describedby="loginError ? 'login-message' : undefined"
          @update:model-value="loginError = ''"
        />
        <FieldMessage id="login-message" :message="loginError" error />
      </div>
      <PasswordField
        id="password"
        v-model="password"
        label="密码"
        autocomplete="current-password"
        placeholder="请输入密码"
        :disabled="loading"
        :error="passwordError"
        @update:model-value="passwordError = ''"
      >
        <template #action>
          <RouterLink to="/auth/forgot-password" class="text-xs text-primary hover:underline">
            忘记密码
          </RouterLink>
        </template>
      </PasswordField>
      <FieldMessage :message="serverError" error />
      <Button class="w-full" type="submit" :disabled="loading">
        <Spinner v-if="loading" class="mr-2" />
        {{ loading ? '正在登录' : '登录' }}
      </Button>
      <p class="text-center text-xs text-muted-foreground">
        首次使用？
        <RouterLink to="/auth/bootstrap" class="font-medium text-foreground underline-offset-4 hover:underline">
          初始化系统
        </RouterLink>
      </p>
    </form>
  </AuthShell>
</template>
