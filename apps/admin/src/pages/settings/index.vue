<script setup lang="ts">
import { CheckCircle2Icon, LoaderCircleIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { authApi, getApiError } from '@/modules/auth/api/auth'
import FieldMessage from '@/modules/auth/components/FieldMessage.vue'
import PasswordField from '@/modules/auth/components/PasswordField.vue'
import { useSessionStore } from '@/modules/auth/stores/session'
import {
  validateAvatarUrl,
  validateDisplayName,
  validateEmail,
  validatePassword,
  validateRequired,
} from '@/modules/auth/validation'

import SettingsLayout from './components/settings-layout.vue'

const router = useRouter()
const session = useSessionStore()
const email = ref('')
const displayName = ref('')
const avatarUrl = ref('')
const currentPassword = ref('')
const newPassword = ref('')
const loading = ref(false)
const passwordLoading = ref(false)
const profileMessage = ref('')
const profileError = ref('')
const passwordError = ref('')
const profileFields = reactive({
  displayName: '',
  email: '',
  avatarUrl: '',
})
const passwordFields = reactive({
  current: '',
  next: '',
})

onMounted(async () => {
  try {
    const profile = await authApi.getProfile()
    email.value = profile.email
    displayName.value = profile.display_name
    avatarUrl.value = profile.avatar_url ?? ''
    session.setUser(profile)
  }
  catch (requestError) {
    profileError.value = getApiError(requestError)
  }
})

async function saveProfile() {
  profileFields.displayName = validateDisplayName(displayName.value)
  profileFields.email = validateEmail(email.value)
  profileFields.avatarUrl = validateAvatarUrl(avatarUrl.value)
  if (Object.values(profileFields).some(Boolean))
    return

  loading.value = true
  profileMessage.value = ''
  profileError.value = ''
  try {
    const profile = await authApi.updateProfile({
      email: email.value,
      display_name: displayName.value,
      avatar_url: avatarUrl.value || null,
    })
    session.setUser(profile)
    profileMessage.value = '个人资料已保存'
  }
  catch (requestError) {
    profileError.value = getApiError(requestError)
  }
  finally {
    loading.value = false
  }
}

async function savePassword() {
  passwordFields.current = validateRequired(currentPassword.value, '当前密码')
  passwordFields.next = validatePassword(newPassword.value)
  if (Object.values(passwordFields).some(Boolean))
    return

  passwordLoading.value = true
  passwordError.value = ''
  try {
    await authApi.updatePassword(currentPassword.value, newPassword.value)
    currentPassword.value = ''
    newPassword.value = ''
    // 修改密码后所有 Session 被撤销，回到登录页。
    window.setTimeout(() => router.replace('/auth/sign-in'), 700)
  }
  catch (requestError) {
    passwordError.value = getApiError(requestError)
  }
  finally {
    passwordLoading.value = false
  }
}
</script>

<template>
  <SettingsLayout title="个人资料" description="管理公开显示信息和管理员密码。">
    <div class="grid gap-6 xl:grid-cols-2">
      <Card class="h-fit">
        <form class="contents" novalidate @submit.prevent="saveProfile">
          <CardHeader>
            <CardTitle class="text-base">基本信息</CardTitle>
            <CardDescription>用于后台账户识别和公开作者信息。</CardDescription>
          </CardHeader>
          <CardContent class="grid gap-5">
            <div class="grid gap-2">
              <label for="profile-display-name" class="text-sm font-medium leading-none">显示名称</label>
              <Input id="profile-display-name" v-model="displayName" :disabled="loading" :aria-invalid="Boolean(profileFields.displayName)" @update:model-value="profileFields.displayName = ''" />
              <FieldMessage :message="profileFields.displayName" error />
            </div>
            <div class="grid gap-2">
              <label for="profile-email" class="text-sm font-medium leading-none">邮箱</label>
              <Input id="profile-email" v-model="email" type="email" autocomplete="email" :disabled="loading" :aria-invalid="Boolean(profileFields.email)" @update:model-value="profileFields.email = ''" />
              <FieldMessage :message="profileFields.email" error />
            </div>
            <div class="grid gap-2">
              <label for="profile-avatar" class="text-sm font-medium leading-none">头像 URL</label>
              <Input id="profile-avatar" v-model="avatarUrl" type="url" placeholder="https://example.com/avatar.png" :disabled="loading" :aria-invalid="Boolean(profileFields.avatarUrl)" @update:model-value="profileFields.avatarUrl = ''" />
              <FieldMessage :message="profileFields.avatarUrl || '可留空，仅支持 HTTP 或 HTTPS 地址。'" :error="Boolean(profileFields.avatarUrl)" />
            </div>
            <FieldMessage :message="profileError" error />
          </CardContent>
          <CardFooter class="justify-between gap-4 border-t pt-6">
            <p v-if="profileMessage" role="status" class="flex items-center gap-1.5 text-xs text-muted-foreground">
              <CheckCircle2Icon class="size-4 text-primary" />
              {{ profileMessage }}
            </p>
            <span v-else />
            <Button type="submit" :disabled="loading">
              <LoaderCircleIcon v-if="loading" class="animate-spin" />
              {{ loading ? '保存中' : '保存资料' }}
            </Button>
          </CardFooter>
        </form>
      </Card>

      <Card class="h-fit">
        <form class="contents" novalidate @submit.prevent="savePassword">
          <CardHeader>
            <CardTitle class="text-base">修改密码</CardTitle>
            <CardDescription>更新后，当前账户的全部 Session 都会失效。</CardDescription>
          </CardHeader>
          <CardContent class="grid gap-5">
            <PasswordField
              id="current-password"
              v-model="currentPassword"
              label="当前密码"
              autocomplete="current-password"
              :disabled="passwordLoading"
              :error="passwordFields.current"
              @update:model-value="passwordFields.current = ''"
            />
            <PasswordField
              id="new-password"
              v-model="newPassword"
              label="新密码"
              autocomplete="new-password"
              hint="使用 10 至 128 个字符，并同时包含字母和数字。"
              :disabled="passwordLoading"
              :error="passwordFields.next"
              :minlength="10"
              :maxlength="128"
              @update:model-value="passwordFields.next = ''"
            />
            <FieldMessage :message="passwordError" error />
          </CardContent>
          <CardFooter class="justify-end border-t pt-6">
            <Button type="submit" :disabled="passwordLoading">
              <LoaderCircleIcon v-if="passwordLoading" class="animate-spin" />
              {{ passwordLoading ? '更新中' : '更新密码' }}
            </Button>
          </CardFooter>
        </form>
      </Card>
    </div>
  </SettingsLayout>
</template>
