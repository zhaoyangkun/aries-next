<script setup lang="ts">
import { CheckCircle2Icon, LoaderCircleIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { settingsGroupApi } from '@/modules/settings/api/settings'
import { getApiError } from '@/shared/api/client'

import SettingsLayout from './components/settings-layout.vue'

const form = reactive({
  analytics_id: '',
  site_verification_token: '',
})
const version = ref(0)
const loading = ref(true)
const saving = ref(false)
const message = ref('')
const error = ref('')

onMounted(async () => {
  try {
    const record = await settingsGroupApi.getIntegrations()
    version.value = record.version
    form.analytics_id = record.settings.analytics_id ?? ''
    form.site_verification_token = record.settings.site_verification_token ?? ''
  }
  catch (requestError) {
    error.value = getApiError(requestError, '集成设置加载失败')
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
    const record = await settingsGroupApi.updateIntegrations(version.value, {
      analytics_id: form.analytics_id.trim() || null,
      site_verification_token: form.site_verification_token.trim() || null,
    })
    version.value = record.version
    message.value = '集成设置已保存'
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
  <SettingsLayout title="第三方集成" description="受控的集成配置位，仅结构化字段，不支持任意脚本注入。">
    <Card>
      <form class="contents" novalidate @submit.prevent="save">
        <CardHeader>
          <CardTitle class="text-base">集成配置</CardTitle>
          <CardDescription>统计与站点验证等第三方服务的标识信息，由公开站在渲染时读取。</CardDescription>
        </CardHeader>
        <CardContent class="grid max-w-2xl gap-5">
          <div class="grid gap-2">
            <label for="analytics-id" class="text-sm font-medium leading-none">统计 ID</label>
            <Input id="analytics-id" v-model="form.analytics_id" placeholder="G-XXXXXXXXXX" :disabled="loading || saving" />
            <p class="text-xs text-muted-foreground">站点统计服务（如 Google Analytics）的衡量 ID，留空表示不启用。</p>
          </div>
          <div class="grid gap-2">
            <label for="site-verification" class="text-sm font-medium leading-none">站点验证 Token</label>
            <Input id="site-verification" v-model="form.site_verification_token" :disabled="loading || saving" />
            <p class="text-xs text-muted-foreground">搜索引擎站长平台的站点验证值，用于输出 verification meta 标签。</p>
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
