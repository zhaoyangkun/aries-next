<script setup lang="ts" generic="TData">
import { computed, h, ref } from 'vue'
import {
  FlexRender,
  getCoreRowModel,
  useVueTable,
  type ColumnDef,
  type RowSelectionState,
  type VisibilityState,
} from '@tanstack/vue-table'
import {
  ArrowDownIcon,
  ArrowUpDownIcon,
  ArrowUpIcon,
  FileQuestionIcon,
  RefreshCwIcon,
} from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import AppEmptyState from '../AppEmptyState.vue'
import './types'

const props = withDefaults(
  defineProps<{
    data: TData[]
    columns: ColumnDef<TData, unknown>[]
    loading?: boolean
    error?: string
    /** 当前排序字段与方向（后端排序）；表头点击通过 update:sort 通知父级。 */
    sortField?: string
    sortOrder?: 'asc' | 'desc'
    /** 行选择状态（Record<rowKey, true>），配合 update:selection 支持 v-model:selection。 */
    selection?: RowSelectionState
    selectable?: boolean
    /** 行可点击时打开，统一行 hover/cursor 语义；点击通过 row-click 抛出。 */
    clickable?: boolean
    /** 行唯一标识；行选择状态以它为 key。 */
    rowKey?: (row: TData) => string | number
    emptyTitle?: string
    emptyDescription?: string
    emptyIcon?: typeof FileQuestionIcon
  }>(),
  { loading: false, error: '', sortOrder: 'desc', selectable: false, clickable: false },
)

const emit = defineEmits<{
  'update:sort': [field: string]
  'update:selection': [selection: RowSelectionState]
  'row-click': [row: TData]
  retry: []
}>()

const columnVisibility = ref<VisibilityState>({})

const selectionColumn: ColumnDef<TData, unknown> = {
  id: '__selection',
  enableHiding: false,
  meta: { headerClass: 'w-10', cellClass: 'w-10' },
  header: ({ table }) =>
    h(Checkbox, {
      'modelValue': table.getIsAllPageRowsSelected()
        ? true
        : table.getIsSomePageRowsSelected()
          ? 'indeterminate'
          : false,
      'onUpdate:modelValue': (value: boolean | 'indeterminate') =>
        table.toggleAllPageRowsSelected(value === true),
      'aria-label': '全选当前页',
      'onClick': (event: Event) => event.stopPropagation(),
    }),
  cell: ({ row }) =>
    h(Checkbox, {
      'modelValue': row.getIsSelected(),
      'onUpdate:modelValue': (value: boolean | 'indeterminate') => row.toggleSelected(value === true),
      'aria-label': '选择该行',
      'onClick': (event: Event) => event.stopPropagation(),
    }),
}

const tableColumns = computed(() =>
  props.selectable ? [selectionColumn, ...props.columns] : props.columns,
)

const rowKey = props.rowKey
const table = useVueTable({
  get data() {
    return props.data
  },
  get columns() {
    return tableColumns.value
  },
  state: {
    get rowSelection() {
      return props.selection ?? {}
    },
    get columnVisibility() {
      return columnVisibility.value
    },
  },
  enableRowSelection: props.selectable,
  manualSorting: true,
  getCoreRowModel: getCoreRowModel(),
  getRowId: rowKey ? (row) => String(rowKey(row)) : undefined,
  onRowSelectionChange: (updater) => {
    const next =
      typeof updater === 'function' ? updater(props.selection ?? {}) : updater
    emit('update:selection', next)
  },
  onColumnVisibilityChange: (updater) => {
    columnVisibility.value =
      typeof updater === 'function' ? updater(columnVisibility.value) : updater
  },
})

// 同字段再次点击交给父级翻转方向，父级决定 asc/desc 规则。
function toggleSort(field: string) {
  emit('update:sort', field)
}

const headerGroups = computed(() => table.getHeaderGroups())
const rows = computed(() => table.getRowModel().rows)
const columnCount = computed(() => table.getVisibleLeafColumns().length || 1)

defineExpose({ table })
</script>

<template>
  <Table>
    <TableHeader>
      <TableRow v-for="headerGroup in headerGroups" :key="headerGroup.id">
        <TableHead
          v-for="header in headerGroup.headers"
          :key="header.id"
          :class="header.column.columnDef.meta?.headerClass"
        >
          <template v-if="!header.isPlaceholder">
            <button
              v-if="header.column.columnDef.meta?.sortable"
              type="button"
              class="hover:text-foreground inline-flex items-center gap-1 transition-colors"
              :aria-label="`按${header.column.columnDef.meta?.toggleLabel ?? header.column.id}排序`"
              @click="toggleSort(header.column.id)"
            >
              <FlexRender :render="header.column.columnDef.header" :props="header.getContext()" />
              <ArrowUpIcon v-if="sortField === header.column.id && sortOrder === 'asc'" class="size-3.5 text-primary" />
              <ArrowDownIcon v-else-if="sortField === header.column.id && sortOrder === 'desc'" class="size-3.5 text-primary" />
              <ArrowUpDownIcon v-else class="size-3.5 text-muted-foreground/60" />
            </button>
            <FlexRender v-else :render="header.column.columnDef.header" :props="header.getContext()" />
          </template>
        </TableHead>
      </TableRow>
    </TableHeader>
    <TableBody>
      <template v-if="loading && data.length === 0">
        <TableRow v-for="index in 5" :key="index" aria-hidden="true">
          <TableCell :colspan="columnCount" class="p-2">
            <Skeleton class="h-9 w-full" />
          </TableCell>
        </TableRow>
      </template>
      <TableRow v-else-if="error">
        <TableCell :colspan="columnCount" class="h-48 text-center">
          <p class="text-sm font-medium">{{ error }}</p>
          <Button variant="outline" size="sm" class="mt-3" @click="emit('retry')">
            <RefreshCwIcon />
            重试
          </Button>
        </TableCell>
      </TableRow>
      <template v-else-if="rows.length">
        <TableRow
          v-for="row in rows"
          :key="row.id"
          :data-state="row.getIsSelected() ? 'selected' : undefined"
          :class="[loading ? 'opacity-60' : '', clickable ? 'cursor-pointer' : '']"
          @click="clickable && emit('row-click', row.original)"
        >
          <TableCell
            v-for="cell in row.getVisibleCells()"
            :key="cell.id"
            :class="cell.column.columnDef.meta?.cellClass"
          >
            <slot :name="`cell-${cell.column.id}`" :row="row.original" :value="cell.getValue()">
              <FlexRender :render="cell.column.columnDef.cell" :props="cell.getContext()" />
            </slot>
          </TableCell>
        </TableRow>
      </template>
      <TableRow v-else>
        <TableCell :colspan="columnCount" class="p-0">
          <slot name="empty">
            <AppEmptyState
              :title="emptyTitle ?? '暂无数据'"
              :description="emptyDescription ?? '调整筛选条件，或稍后再试。'"
              :icon="emptyIcon ?? FileQuestionIcon"
              class="min-h-56"
            >
              <template v-if="$slots['empty-actions']" #actions>
                <slot name="empty-actions" />
              </template>
            </AppEmptyState>
          </slot>
        </TableCell>
      </TableRow>
    </TableBody>
  </Table>
</template>
