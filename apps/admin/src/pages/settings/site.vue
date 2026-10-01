<script setup lang="ts">
import { CheckCircle2Icon, LoaderCircleIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { siteSettingsApi, type UpdateSiteSettingsPayload } from '@/modules/settings/api/settings'
import { getApiError } from '@/shared/api/client'

import SettingsLayout from './components/settings-layout.vue'

const form = reactive<UpdateSiteSettingsPayload>({
  site_name: '',
  site_description: '',
  site_url: '',
  logo_url: '',
  icp_text: '',
  default_cover_url: '',
  page_size_index: 10,
  page_size_archive: 20,
  page_size_search: 20,
  comment_policy: 'moderated',
  comments_per_page: 20,
})
const loading = ref(true)
const saving = ref(false)
const message = ref('')
const error = ref('')

onMounted(async () => {
  try {
    const settings = await siteSettingsApi.get()
    const { updated_at: _, ...payload } = settings
    Object.assign(form, payload)
  }
  catch (requestError) {
    error.value = getApiError(requestError, '站点设置加载失败')
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
    const settings = await siteSettingsApi.update({ ...form })
    const { updated_at: _, ...payload } = settings
    Object.assign(form, payload)
    message.value = '站点设置已保存'
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
  <SettingsLayout title="站点设置" description="公开站的基础信息与分页配置，保存后立即对公开端生效。">
    <Card>
      <form class="contents" novalidate @submit.prevent="save">
        <CardHeader>
          <CardTitle class="text-base">基础信息</CardTitle>
          <CardDescription>站点名称、描述与公开 URL，用于 SEO 与页脚展示。</CardDescription>
        </CardHeader>
        <CardContent class="grid max-w-2xl gap-5">
          <div class="grid gap-2">
            <label for="site-name" class="text-sm font-medium leading-none">站点名称</label>
            <Input id="site-name" v-model="form.site_name" :disabled="loading || saving" />
          </div>
          <div class="grid gap-2">
            <label for="site-description" class="text-sm font-medium leading-none">站点描述</label>
            <Textarea id="site-description" v-model="form.site_description" rows="3" :disabled="loading || saving" />
          </div>
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <label for="site-url" class="text-sm font-medium leading-none">站点 URL</label>
              <Input id="site-url" v-model="form.site_url" type="url" placeholder="https://example.com" :disabled="loading || saving" />
            </div>
            <div class="grid gap-2">
              <label for="icp-text" class="text-sm font-medium leading-none">备案号</label>
              <Input id="icp-text" v-model="form.icp_text" placeholder="京ICP备xxxxxxxx号" :disabled="loading || saving" />
            </div>
          </div>
          <div class="grid gap-4 sm:grid-cols-2">
            <div class="grid gap-2">
              <label for="logo-url" class="text-sm font-medium leading-none">Logo URL</label>
              <Input id="logo-url" v-model="form.logo_url" :disabled="loading || saving" />
            </div>
            <div class="grid gap-2">
              <label for="default-cover" class="text-sm font-medium leading-none">默认封面 URL</label>
              <Input id="default-cover" v-model="form.default_cover_url" :disabled="loading || saving" />
            </div>
          </div>

          <fieldset class="grid gap-4 border-t pt-5 sm:grid-cols-3">
            <legend class="text-sm font-medium">分页大小</legend>
            <div class="grid gap-2">
              <label for="page-size-index" class="text-xs text-muted-foreground">首页</label>
              <Input id="page-size-index" v-model.number="form.page_size_index" type="number" min="1" max="100" :disabled="loading || saving" />
            </div>
            <div class="grid gap-2">
              <label for="page-size-archive" class="text-xs text-muted-foreground">归档页</label>
              <Input id="page-size-archive" v-model.number="form.page_size_archive" type="number" min="1" max="100" :disabled="loading || saving" />
            </div>
            <div class="grid gap-2">
              <label for="page-size-search" class="text-xs text-muted-foreground">搜索页</label>
              <Input id="page-size-search" v-model.number="form.page_size_search" type="number" min="1" max="100" :disabled="loading || saving" />
            </div>
          </fieldset>

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
