<script setup lang="ts">
import { CheckCircle2Icon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import { authApi, getApiError } from '@/modules/auth/api/auth'
import FieldMessage from '@/modules/auth/components/FieldMessage.vue'
import PasswordField from '@/modules/auth/components/PasswordField.vue'
import { useSessionStore } from '@/modules/auth/stores/session'
import {
  validateDisplayName,
  validateEmail,
  validatePassword,
  validateRequired,
  validateUsername,
} from '@/modules/auth/validation'

import AuthShell from './components/auth-shell.vue'

const router = useRouter()
const session = useSessionStore()
const secret = ref('')
const username = ref('')
const email = ref('')
const displayName = ref('')
const password = ref('')
const initialized = ref(false)
const loading = ref(false)
const serverError = ref('')
const errors = reactive({
  secret: '',
  username: '',
  email: '',
  displayName: '',
  password: '',
})

onMounted(async () => {
  try {
    initialized.value = (await authApi.bootstrapStatus()).initialized
  }
  catch (requestError) {
    serverError.value = getApiError(requestError, '无法读取初始化状态')
  }
})

async function submit() {
  errors.secret = validateRequired(secret.value, 'Bootstrap Secret')
  if (!errors.secret && secret.value.length < 24)
    errors.secret = 'Bootstrap Secret 至少需要 24 个字符'
  errors.username = validateUsername(username.value)
  errors.email = validateEmail(email.value)
  errors.displayName = validateDisplayName(displayName.value)
  errors.password = validatePassword(password.value)
  if (Object.values(errors).some(Boolean))
    return

  loading.value = true
  serverError.value = ''
  try {
    await session.bootstrap({
      bootstrap_secret: secret.value,
      username: username.value,
      email: email.value,
      display_name: displayName.value,
      password: password.value,
    })
    await router.replace('/')
  }
  catch (requestError) {
    serverError.value = getApiError(requestError, '初始化失败，请检查输入或服务配置')
  }
  finally {
    loading.value = false
  }
}
</script>

<template>
  <AuthShell title="初始化系统" description="创建首个 Owner。完成后该入口将自动关闭。" wide>
    <div v-if="initialized" class="grid justify-items-center gap-5 py-4 text-center">
      <CheckCircle2Icon class="size-10 text-primary" />
      <div>
        <p class="font-medium">系统已经初始化</p>
        <p class="mt-1 text-sm text-muted-foreground">请使用现有管理员账号登录。</p>
      </div>
      <Button class="w-full" as-child>
        <RouterLink to="/auth/sign-in">返回登录</RouterLink>
      </Button>
    </div>
    <form v-else class="grid gap-5" novalidate @submit.prevent="submit">
      <PasswordField
        id="bootstrap-secret"
        v-model="secret"
        label="Bootstrap Secret"
        autocomplete="off"
        hint="使用 Backend 环境变量中配置的初始化密钥。"
        :disabled="loading"
        :error="errors.secret"
        :minlength="24"
        @update:model-value="errors.secret = ''"
      />
      <div class="grid gap-4 sm:grid-cols-2">
        <div class="grid gap-2">
          <label for="username" class="text-sm font-medium leading-none">用户名</label>
          <Input id="username" v-model="username" autocomplete="username" :disabled="loading" :aria-invalid="Boolean(errors.username)" @update:model-value="errors.username = ''" />
          <FieldMessage :message="errors.username" error />
        </div>
        <div class="grid gap-2">
          <label for="email" class="text-sm font-medium leading-none">邮箱</label>
          <Input id="email" v-model="email" type="email" autocomplete="email" :disabled="loading" :aria-invalid="Boolean(errors.email)" @update:model-value="errors.email = ''" />
          <FieldMessage :message="errors.email" error />
        </div>
      </div>
      <div class="grid gap-2">
        <label for="display-name" class="text-sm font-medium leading-none">显示名称</label>
        <Input id="display-name" v-model="displayName" autocomplete="name" :disabled="loading" :aria-invalid="Boolean(errors.displayName)" @update:model-value="errors.displayName = ''" />
        <FieldMessage :message="errors.displayName" error />
      </div>
      <PasswordField
        id="password"
        v-model="password"
        label="管理员密码"
        autocomplete="new-password"
        hint="使用 10 至 128 个字符，并同时包含字母和数字。"
        :disabled="loading"
        :error="errors.password"
        :minlength="10"
        :maxlength="128"
        @update:model-value="errors.password = ''"
      />
      <FieldMessage :message="serverError" error />
      <Button class="w-full" type="submit" :disabled="loading">
        <Spinner v-if="loading" class="mr-2" />
        {{ loading ? '正在创建' : '创建 Owner' }}
      </Button>
      <p class="text-center text-xs text-muted-foreground">
        已有账号？
        <RouterLink to="/auth/sign-in" class="font-medium text-foreground underline-offset-4 hover:underline">返回登录</RouterLink>
      </p>
    </form>
  </AuthShell>
</template>
