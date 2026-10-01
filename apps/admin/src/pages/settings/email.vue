<script setup lang="ts">
import { CheckCircle2Icon, LoaderCircleIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { settingsGroupApi } from '@/modules/settings/api/settings'
import { getApiError } from '@/shared/api/client'

import SettingsLayout from './components/settings-layout.vue'

// smtp_password 为 write-only secret：GET 只回 smtp_password_set，
// 提交时留空表示保持不变，点击「清除」则显式提交空串删除已保存密码。
const form = reactive({
  enabled: false,
  smtp_host: '',
  smtp_port: undefined as number | undefined,
  smtp_username: '',
  from_address: '',
  from_name: '',
})
const passwordSet = ref(false)
const newPassword = ref('')
const clearPassword = ref(false)
const version = ref(0)
const loading = ref(true)
const saving = ref(false)
const message = ref('')
const error = ref('')

onMounted(async () => {
  try {
    const record = await settingsGroupApi.getEmail()
    version.value = record.version
    passwordSet.value = record.settings.smtp_password_set
    form.enabled = record.settings.enabled
    form.smtp_host = record.settings.smtp_host ?? ''
    form.smtp_port = record.settings.smtp_port ?? undefined
    form.smtp_username = record.settings.smtp_username ?? ''
    form.from_address = record.settings.from_address ?? ''
    form.from_name = record.settings.from_name ?? ''
  }
  catch (requestError) {
    error.value = getApiError(requestError, '邮件设置加载失败')
  }
  finally {
    loading.value = false
  }
})

async function save() {
  saving.value = true
  message.value = ''
  error.value = ''
  try {
    const record = await settingsGroupApi.updateEmail(version.value, {
      enabled: form.enabled,
      smtp_host: form.smtp_host.trim() || null,
      smtp_port: form.smtp_port || null,
      smtp_username: form.smtp_username.trim() || null,
      // 三态语义：输入了新密码优先更新，其次显式清除，否则缺省保持不变。
      ...(newPassword.value
        ? { smtp_password: newPassword.value }
        : clearPassword.value
          ? { smtp_password: '' }
          : {}),
      from_address: form.from_address.trim() || null,
      from_name: form.from_name.trim() || null,
    })
    version.value = record.version
    passwordSet.value = record.settings.smtp_password_set
    newPassword.value = ''
    clearPassword.value = false
    message.value = '邮件设置已保存'
  }
  catch (requestError) {
    error.value = getApiError(requestError, '保存失败')
  }
  finally {
    saving.value = false
  }
}
</script>

<template>
  <SettingsLayout title="邮件设置" description="配置 SMTP 发信服务，用于评论回复通知与密码重置邮件。">
    <Card>
      <form class="contents" novalidate @submit.prevent="save">
        <CardHeader>
          <CardTitle class="text-base">SMTP 服务</CardTitle>
          <CardDescription>密码为 write-only：服务端只返回是否已设置，永不回读明文。</CardDescription>
        </CardHeader>
        <CardContent class="grid max-w-2xl gap-5">
          <label class="flex cursor-pointer items-center justify-between gap-3 text-sm font-medium">
            启用邮件发送
            <input v-model="form.enabled" type="checkbox" class="peer sr-only" :disabled="loading || saving" />
            <span class="relative h-5 w-9 shrink-0 rounded-full bg-input transition-colors after:absolute after:left-0.5 after:top-0.5 after:size-4 after:rounded-full after:bg-background after:shadow after:transition-transform peer-checked:bg-primary peer-checked:after:translate-x-4 peer-disabled:opacity-50" />
          </label>
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <label for="smtp-host" class="text-sm font-medium leading-none">SMTP 主机</label>
              <Input id="smtp-host" v-model="form.smtp_host" placeholder="smtp.example.com" :disabled="loading || saving" />
            </div>
            <div class="grid gap-2">
              <label for="smtp-port" class="text-sm font-medium leading-none">SMTP 端口</label>
              <Input id="smtp-port" v-model.number="form.smtp_port" type="number" min="1" max="65535" placeholder="465" :disabled="loading || saving" />
            </div>
          </div>
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <label for="smtp-username" class="text-sm font-medium leading-none">SMTP 用户名</label>
              <Input id="smtp-username" v-model="form.smtp_username" autocomplete="off" :disabled="loading || saving" />
            </div>
            <div class="grid gap-2">
              <label for="smtp-password" class="text-sm font-medium leading-none">
                SMTP 密码
                <span class="text-xs text-muted-foreground">（{{ passwordSet ? '已设置' : '未设置' }}）</span>
              </label>
              <Input
                id="smtp-password"
                v-model="newPassword"
                type="password"
                autocomplete="new-password"
                :placeholder="passwordSet ? '留空保持当前密码' : '输入密码'"
                :disabled="loading || saving"
                @update:model-value="clearPassword = false"
              />
              <div v-if="passwordSet && !newPassword" class="space-y-1.5">
                <Button
                  v-if="!clearPassword"
                  type="button"
                  variant="outline"
                  size="sm"
                  :disabled="loading || saving"
                  @click="clearPassword = true"
                >
                  清除已保存的密码
                </Button>
                <p v-else class="flex items-center justify-between gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-2.5 py-1.5 text-xs text-destructive">
                  保存后将清除 SMTP 密码
                  <button type="button" class="font-medium underline" :disabled="saving" @click="clearPassword = false">撤销</button>
                </p>
              </div>
            </div>
          </div>
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <label for="from-address" class="text-sm font-medium leading-none">发件地址</label>
              <Input id="from-address" v-model="form.from_address" type="email" placeholder="noreply@example.com" :disabled="loading || saving" />
            </div>
            <div class="grid gap-2">
              <label for="from-name" class="text-sm font-medium leading-none">发件人名称</label>
              <Input id="from-name" v-model="form.from_name" placeholder="Aries" :disabled="loading || saving" />
            </div>
          </div>
          <p v-if="error" role="alert" class="text-xs font-medium text-destructive">{{ error }}</p>
        </CardContent>
        <CardFooter class="justify-between gap-4 border-t pt-6">
          <p v-if="message" role="status" class="flex items-center gap-1.5 text-xs text-muted-foreground">
            <CheckCircle2Icon class="size-4 text-primary" />
            {{ message }}
          </p>
          <span v-else />
          <Button type="submit" :disabled="loading || saving">
            <LoaderCircleIcon v-if="saving" class="animate-spin" />
            {{ saving ? '保存中' : '保存设置' }}
          </Button>
        </CardFooter>
      </form>
    </Card>
  </SettingsLayout>
</template>

<route lang="yaml">
meta:
  permission: settings:manage
</route>
