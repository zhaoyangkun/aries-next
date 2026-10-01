<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import dayjs from 'dayjs'
import { ScrollTextIcon, SearchIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
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
import { BasicPage } from '@/components/global-layout'
import { auditApi, type AuditLog } from '@/modules/audit/api/audit'
import { getApiError } from '@/shared/api/client'
import { toRfc3339 } from '@/utils/to-rfc3339'

const page = ref(1)
const pageSize = ref(20)
const action = ref('')
const targetType = ref('')
const targetId = ref('')
const start = ref('')
const end = ref('')
const logs = ref<AuditLog[]>([])
const total = ref(0)
const loading = ref(true)
const error = ref('')

async function loadLogs() {
  loading.value = true
  error.value = ''
  try {
    const result = await auditApi.list({
      page: page.value,
      page_size: pageSize.value,
      action: action.value.trim() || undefined,
      target_type: targetType.value.trim() || undefined,
      target_id: targetId.value.trim() || undefined,
      start: toRfc3339(start.value),
      end: toRfc3339(end.value),
    })
    logs.value = result.items
    total.value = result.total
  }
  catch (e) {
    error.value = getApiError(e, '加载审计日志失败')
  }
  finally {
    loading.value = false
  }
}

function applyFilter() {
  // page 非 1 时由下方 watch 触发加载，避免重复请求。
  if (page.value !== 1) {
    page.value = 1
    return
  }
  loadLogs()
}

function resetFilter() {
  action.value = ''
  targetType.value = ''
  targetId.value = ''
  start.value = ''
  end.value = ''
  if (page.value !== 1) {
    page.value = 1
    return
  }
  loadLogs()
}

function formatTime(value: string) {
  const parsed = dayjs(value)
  return parsed.isValid() ? parsed.format('YYYY-MM-DD HH:mm:ss') : '-'
}

function targetLabel(log: AuditLog) {
  if (!log.target_id)
    return '-'
  return `${log.target_type} #${log.target_id}`
}

// 翻页与每页数量变化时重新请求；pageSize 变化且不在第一页时先回到第一页（由 page 变化触发加载）。
watch([page, pageSize], ([currentPage, currentSize], [_previousPage, previousSize]) => {
  if (currentSize !== previousSize && currentPage !== 1) {
    page.value = 1
    return
  }
  loadLogs()
})

onMounted(loadLogs)
</script>

<template>
  <BasicPage title="审计日志" description="查询管理端敏感操作记录，仅 Owner 可见。">
    <Card>
      <CardHeader class="gap-4">
        <div class="flex flex-wrap items-end gap-3">
          <div class="grid w-40 gap-2">
            <Label for="audit-action">动作</Label>
            <Input
              id="audit-action"
              v-model="action"
              placeholder="如 article.created"
              class="h-8 text-sm"
              @keyup.enter="applyFilter"
            />
          </div>
          <div class="grid w-36 gap-2">
            <Label for="audit-target-type">目标类型</Label>
            <Input
              id="audit-target-type"
              v-model="targetType"
              placeholder="如 article"
              class="h-8 text-sm"
              @keyup.enter="applyFilter"
            />
          </div>
          <div class="grid w-32 gap-2">
            <Label for="audit-target-id">目标 ID</Label>
            <Input
              id="audit-target-id"
              v-model="targetId"
              placeholder="123"
              class="h-8 text-sm"
              @keyup.enter="applyFilter"
            />
          </div>
          <div class="grid w-44 gap-2">
            <Label for="audit-start">开始时间</Label>
            <Input
              id="audit-start"
              v-model="start"
              type="datetime-local"
              class="h-8 text-sm"
            />
          </div>
          <div class="grid w-44 gap-2">
            <Label for="audit-end">结束时间</Label>
            <Input
              id="audit-end"
              v-model="end"
              type="datetime-local"
              class="h-8 text-sm"
            />
          </div>
          <Button variant="outline" size="sm" class="h-8" @click="applyFilter">
            <SearchIcon />
            查询
          </Button>
          <Button variant="ghost" size="sm" class="h-8" @click="resetFilter">
            重置
          </Button>
        </div>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="error" class="border-b px-4 py-3 text-sm text-destructive">{{ error }}</p>

        <div v-if="loading" class="space-y-2 p-4">
          <div v-for="index in 6" :key="index" class="h-10 w-full rounded-md bg-muted/60" />
        </div>

        <AppEmptyState
          v-else-if="logs.length === 0"
          :icon="ScrollTextIcon"
          title="没有匹配的审计记录"
          description="调整筛选条件后重新查询。"
        />

        <div v-else class="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead class="w-24">时间</TableHead>
                <TableHead class="w-28">操作人</TableHead>
                <TableHead>动作</TableHead>
                <TableHead class="w-32">目标</TableHead>
                <TableHead>详情</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="log in logs" :key="log.id">
                <TableCell class="whitespace-nowrap text-xs tabular-nums text-muted-foreground">
                  {{ formatTime(log.created_at) }}
                </TableCell>
                <TableCell class="text-sm">
                  {{ log.actor_username ?? (log.actor_user_id ? `用户 #${log.actor_user_id}` : '系统') }}
                </TableCell>
                <TableCell>
                  <code class="rounded bg-muted px-1.5 py-0.5 text-xs font-mono">{{ log.action }}</code>
                </TableCell>
                <TableCell class="text-xs text-muted-foreground">{{ targetLabel(log) }}</TableCell>
                <TableCell class="max-w-md">
                  <code class="block truncate text-xs text-muted-foreground" :title="JSON.stringify(log.metadata)">
                    {{ JSON.stringify(log.metadata) }}
                  </code>
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
  </BasicPage>
</template>
