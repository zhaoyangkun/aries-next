<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import dayjs from 'dayjs'
import { InboxIcon } from '@lucide/vue'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { AppDataTablePagination } from '@/shared/components/data-table'
import AppEmptyState from '@/shared/components/AppEmptyState.vue'
import { emailDeliveryApi, type EmailDelivery, type EmailDeliveryStatus } from '@/modules/settings/api/settings'
import { getApiError } from '@/shared/api/client'

import SettingsLayout from './components/settings-layout.vue'

const statusOptions: { value: '' | EmailDeliveryStatus; label: string }[] = [
  { value: '', label: '全部' },
  { value: 'pending', label: '待投递' },
  { value: 'sent', label: '已投递' },
  { value: 'failed', label: '失败' },
  { value: 'bounced', label: '退信' },
]

const statusBadgeVariant: Record<EmailDeliveryStatus, 'secondary' | 'default' | 'destructive' | 'outline'> = {
  pending: 'secondary',
  sent: 'default',
  failed: 'destructive',
  bounced: 'outline',
}

const statusLabel: Record<EmailDeliveryStatus, string> = {
  pending: '待投递',
  sent: '已投递',
  failed: '失败',
  bounced: '退信',
}

const page = ref(1)
const pageSize = ref(20)
const status = ref<'' | EmailDeliveryStatus>('')
const deliveries = ref<EmailDelivery[]>([])
const total = ref(0)
const loading = ref(true)
const error = ref('')

async function loadDeliveries() {
  loading.value = true
  error.value = ''
  try {
    const result = await emailDeliveryApi.list({
      page: page.value,
      page_size: pageSize.value,
      status: status.value || undefined,
    })
    deliveries.value = result.items
    total.value = result.total
  }
  catch (requestError) {
    error.value = getApiError(requestError, '加载邮件投递记录失败')
  }
  finally {
    loading.value = false
  }
}

function applyFilter(next: '' | EmailDeliveryStatus) {
  status.value = next
  // page 非 1 时由下方 watch 触发加载，避免重复请求。
  if (page.value !== 1) {
    page.value = 1
    return
  }
  loadDeliveries()
}

function formatTime(value: string) {
  const parsed = dayjs(value)
  return parsed.isValid() ? parsed.format('YYYY-MM-DD HH:mm:ss') : '-'
}

watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadDeliveries()
})

onMounted(loadDeliveries)
</script>

<template>
  <SettingsLayout title="邮件投递" description="查看评论通知等邮件的投递记录与失败原因。">
    <Card>
      <CardHeader class="gap-4">
        <div class="flex flex-wrap items-center gap-2">
          <Button
            v-for="option in statusOptions"
            :key="option.value"
            :variant="status === option.value ? 'default' : 'outline'"
            size="sm"
            class="h-8"
            @click="applyFilter(option.value)"
          >
            {{ option.label }}
          </Button>
        </div>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="error" class="border-b px-4 py-3 text-sm text-destructive">{{ error }}</p>

        <div v-if="loading" class="space-y-2 p-4">
          <div v-for="index in 6" :key="index" class="h-10 w-full rounded-md bg-muted/60" />
        </div>

        <AppEmptyState
          v-else-if="deliveries.length === 0"
          :icon="InboxIcon"
          title="没有邮件投递记录"
          description="发送测试邮件或产生评论回复后，这里会显示投递记录。"
        />

        <div v-else class="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>收件人</TableHead>
                <TableHead>主题</TableHead>
                <TableHead class="w-28">模板</TableHead>
                <TableHead class="w-20">状态</TableHead>
                <TableHead class="w-24">尝试次数</TableHead>
                <TableHead>最近错误</TableHead>
                <TableHead class="w-40">创建时间</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="delivery in deliveries" :key="delivery.id">
                <TableCell class="text-sm">{{ delivery.recipient_email }}</TableCell>
                <TableCell class="max-w-56 truncate text-sm" :title="delivery.subject">
                  {{ delivery.subject }}
                </TableCell>
                <TableCell>
                  <code class="rounded bg-muted px-1.5 py-0.5 text-xs font-mono">{{ delivery.template }}</code>
                </TableCell>
                <TableCell>
                  <Badge :variant="statusBadgeVariant[delivery.status]">{{ statusLabel[delivery.status] }}</Badge>
                </TableCell>
                <TableCell class="text-xs tabular-nums text-muted-foreground">
                  {{ delivery.attempts }}/{{ delivery.max_attempts }}
                </TableCell>
                <TableCell class="max-w-48">
                  <span
                    v-if="delivery.last_error"
                    class="block truncate text-xs text-destructive"
                    :title="delivery.last_error"
                  >
                    {{ delivery.last_error }}
                  </span>
                  <span v-else class="text-xs text-muted-foreground">-</span>
                </TableCell>
                <TableCell class="whitespace-nowrap text-xs tabular-nums text-muted-foreground">
                  {{ formatTime(delivery.created_at) }}
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </div>
      </CardContent>
      <div class="border-t p-4">
        <AppDataTablePagination
          v-model:page="page"
          v-model:page-size="pageSize"
          :total="total"
          :loading="loading"
          unit="条记录"
        />
      </div>
    </Card>
  </SettingsLayout>
</template>

<route lang="yaml">
meta:
  permission: settings:manage
</route>
