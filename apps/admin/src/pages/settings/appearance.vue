<script setup lang="ts">
import { CheckCircle2Icon, LoaderCircleIcon } from '@lucide/vue'
import ContentLayout from '@/components/custom-theme/content-layout.vue'
import CustomColor from '@/components/custom-theme/custom-color.vue'
import CustomRadius from '@/components/custom-theme/custom-radius.vue'
import ToggleColorMode from '@/components/custom-theme/toggle-color-mode.vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { useSessionStore } from '@/modules/auth/stores/session'
import {
  settingsGroupApi,
  type AppearanceSettings,
  type ColorSchemePreference,
  type ListDensity,
} from '@/modules/settings/api/settings'
import { getApiError } from '@/shared/api/client'

import SettingsLayout from './components/settings-layout.vue'

const session = useSessionStore()
// 站点外观影响公开站，仅 settings:manage（owner）可读写；管理界面偏好对所有用户开放。
const canManageSiteAppearance = computed(() => session.hasPermission('settings:manage'))

// 站点外观表单：null 表示沿用公开站默认（跟随系统/舒适密度）。
const siteForm = reactive({
  logo_url: '',
  favicon_url: '',
  color_scheme: '' as '' | ColorSchemePreference,
  list_density: '' as '' | ListDensity,
})
const siteVersion = ref(0)
const siteLoading = ref(true)
const siteSaving = ref(false)
const siteMessage = ref('')
const siteError = ref('')

onMounted(async () => {
  if (!canManageSiteAppearance.value) {
    siteLoading.value = false
    return
  }
  try {
    const record = await settingsGroupApi.getAppearance()
    siteVersion.value = record.version
    siteForm.logo_url = record.settings.logo_url ?? ''
    siteForm.favicon_url = record.settings.favicon_url ?? ''
    siteForm.color_scheme = record.settings.color_scheme ?? ''
    siteForm.list_density = record.settings.list_density ?? ''
  }
  catch (requestError) {
    siteError.value = getApiError(requestError, '站点外观加载失败')
  }
  finally {
    siteLoading.value = false
  }
})

async function saveSiteAppearance() {
  siteSaving.value = true
  siteMessage.value = ''
  siteError.value = ''
  try {
    const settings: AppearanceSettings = {
      logo_url: siteForm.logo_url.trim() || null,
      favicon_url: siteForm.favicon_url.trim() || null,
      color_scheme: siteForm.color_scheme || null,
      list_density: siteForm.list_density || null,
    }
    const record = await settingsGroupApi.updateAppearance(siteVersion.value, settings)
    siteVersion.value = record.version
    siteMessage.value = '站点外观已保存'
  }
  catch (requestError) {
    siteError.value = getApiError(requestError, '保存失败')
  }
  finally {
    siteSaving.value = false
  }
}
</script>

<template>
  <SettingsLayout title="外观" description="站点外观影响公开站；界面偏好仅影响你自己的浏览器。">
    <div class="space-y-6">
      <Card v-if="canManageSiteAppearance">
        <form class="contents" novalidate @submit.prevent="saveSiteAppearance">
          <CardHeader>
            <CardTitle class="text-base">站点外观</CardTitle>
            <CardDescription>公开站的 Logo、Favicon 与默认展示偏好，保存后立即生效。</CardDescription>
          </CardHeader>
          <CardContent class="grid max-w-2xl gap-5">
            <div class="grid gap-4 sm:grid-cols-2">
              <div class="grid gap-2">
                <label for="appearance-logo" class="text-sm font-medium leading-none">Logo URL</label>
                <Input id="appearance-logo" v-model="siteForm.logo_url" :disabled="siteLoading || siteSaving" />
              </div>
              <div class="grid gap-2">
                <label for="appearance-favicon" class="text-sm font-medium leading-none">Favicon URL</label>
                <Input id="appearance-favicon" v-model="siteForm.favicon_url" :disabled="siteLoading || siteSaving" />
              </div>
            </div>
            <div class="grid gap-4 sm:grid-cols-2">
              <div class="grid gap-2">
                <label for="appearance-color-scheme" class="text-sm font-medium leading-none">颜色模式偏好</label>
                <select
                  id="appearance-color-scheme"
                  v-model="siteForm.color_scheme"
                  class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                  :disabled="siteLoading || siteSaving"
                >
                  <option value="">站点默认</option>
                  <option value="system">跟随系统</option>
                  <option value="light">固定亮色</option>
                  <option value="dark">固定暗色</option>
                </select>
              </div>
              <div class="grid gap-2">
                <label for="appearance-list-density" class="text-sm font-medium leading-none">列表密度</label>
                <select
                  id="appearance-list-density"
                  v-model="siteForm.list_density"
                  class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                  :disabled="siteLoading || siteSaving"
                >
                  <option value="">站点默认</option>
                  <option value="comfortable">舒适</option>
                  <option value="compact">紧凑</option>
                </select>
              </div>
            </div>
            <p v-if="siteError" role="alert" class="text-xs font-medium text-destructive">{{ siteError }}</p>
          </CardContent>
          <CardFooter class="justify-between gap-4 border-t pt-6">
            <p v-if="siteMessage" role="status" class="flex items-center gap-1.5 text-xs text-muted-foreground">
              <CheckCircle2Icon class="size-4 text-primary" />
              {{ siteMessage }}
            </p>
            <span v-else />
            <Button type="submit" :disabled="siteLoading || siteSaving">
              <LoaderCircleIcon v-if="siteSaving" class="animate-spin" />
              {{ siteSaving ? '保存中' : '保存设置' }}
            </Button>
          </CardFooter>
        </form>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle class="text-base">界面偏好</CardTitle>
          <CardDescription>所有选项即时生效，并保存在本机。</CardDescription>
        </CardHeader>
        <CardContent class="max-w-xl">
          <ToggleColorMode />
          <CustomColor />
          <CustomRadius />
          <ContentLayout />
        </CardContent>
      </Card>
    </div>
  </SettingsLayout>
</template>
