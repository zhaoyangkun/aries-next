<script setup lang="ts">
// 社交链接：公开端 footer 展示的外部站点链接，全量覆盖语义（保存体即新状态）。
// 展示顺序即列表顺序；留空名称与 URL 的行在保存时丢弃，行内即时增删、统一保存。
import { LoaderCircleIcon, PlusIcon, Trash2Icon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { settingsGroupApi, type SocialIconKey, type SocialLink } from '@/modules/settings/api/settings'
import { getApiError } from '@/shared/api/client'
import { toast } from 'vue-sonner'

import SettingsLayout from './components/settings-layout.vue'

const ICON_OPTIONS: { value: SocialIconKey, label: string }[] = [
  { value: 'github', label: 'GitHub' },
  { value: 'gitee', label: 'Gitee' },
  { value: 'gitlab', label: 'GitLab' },
  { value: 'weibo', label: '微博' },
  { value: 'zhihu', label: '知乎' },
  { value: 'x', label: 'X（Twitter）' },
  { value: 'bilibili', label: '哔哩哔哩' },
  { value: 'wechat', label: '微信' },
  { value: 'qq', label: 'QQ' },
  { value: 'telegram', label: 'Telegram' },
  { value: 'link', label: '通用链接' },
]

const links = ref<SocialLink[]>([])
const version = ref(0)
const loading = ref(true)
const saving = ref(false)

onMounted(async () => {
  try {
    const record = await settingsGroupApi.getSocial()
    version.value = record.version
    links.value = record.settings.links.map(link => ({ ...link }))
  }
  catch (requestError) {
    toast.error(getApiError(requestError, '社交链接加载失败'))
  }
  finally {
    loading.value = false
  }
})

function addLink() {
  links.value.push({ name: '', url: '', icon: 'link' })
}

function removeLink(index: number) {
  links.value.splice(index, 1)
}

async function save() {
  saving.value = true
  try {
    // 丢弃名称与 URL 均为空的行；其余行 trim 后全量提交。
    const payload = {
      links: links.value
        .filter(link => link.name.trim() || link.url.trim())
        .map(link => ({ ...link, name: link.name.trim(), url: link.url.trim() })),
    }
    const record = await settingsGroupApi.updateSocial(version.value, payload)
    version.value = record.version
    links.value = record.settings.links.map(link => ({ ...link }))
    toast.success('社交链接已保存')
  }
  catch (requestError) {
    toast.error(getApiError(requestError, '保存失败'))
  }
  finally {
    saving.value = false
  }
}
</script>

<template>
  <SettingsLayout title="社交链接" description="公开站页脚展示的外部站点链接，保存后立即生效；展示顺序即列表顺序。">
    <Card>
      <form class="contents" novalidate @submit.prevent="save">
        <CardHeader>
          <CardTitle class="text-base">链接列表</CardTitle>
          <CardDescription>最多 10 条；URL 仅允许 http/https 外链，未知图标将退化为通用链接样式。</CardDescription>
        </CardHeader>
        <CardContent class="grid max-w-3xl gap-3">
          <div v-if="links.length === 0 && !loading" class="text-sm text-muted-foreground">
            还没有配置任何社交链接。
          </div>
          <div
            v-for="(link, index) in links"
            :key="index"
            class="grid items-end gap-3 sm:grid-cols-[10rem_1fr_1.5fr_2.5rem]"
          >
            <div class="grid gap-2">
              <label :for="`social-icon-${index}`" class="text-xs text-muted-foreground">图标</label>
              <select
                :id="`social-icon-${index}`"
                v-model="link.icon"
                class="flex h-9 w-full rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
                :disabled="loading || saving"
              >
                <option v-for="option in ICON_OPTIONS" :key="option.value" :value="option.value">
                  {{ option.label }}
                </option>
              </select>
            </div>
            <div class="grid gap-2">
              <label :for="`social-name-${index}`" class="text-xs text-muted-foreground">名称</label>
              <Input
                :id="`social-name-${index}`"
                v-model="link.name"
                maxlength="20"
                placeholder="GitHub"
                :disabled="loading || saving"
              />
            </div>
            <div class="grid gap-2">
              <label :for="`social-url-${index}`" class="text-xs text-muted-foreground">URL</label>
              <Input
                :id="`social-url-${index}`"
                v-model="link.url"
                type="url"
                placeholder="https://github.com/example"
                :disabled="loading || saving"
              />
            </div>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              aria-label="删除该链接"
              :disabled="loading || saving"
              @click="removeLink(index)"
            >
              <Trash2Icon />
            </Button>
          </div>
          <div>
            <Button type="button" variant="outline" :disabled="loading || saving || links.length >= 10" @click="addLink">
              <PlusIcon />
              添加链接
            </Button>
          </div>
        </CardContent>
        <CardFooter class="justify-end gap-4 border-t pt-6">
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
