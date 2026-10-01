import type { RowData } from '@tanstack/vue-table'

// 列表排序与分页由后端完成（manual 模式），TanStack Table 在这里只负责呈现与交互状态。
declare module '@tanstack/vue-table' {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface ColumnMeta<TData extends RowData, TValue> {
    /** 表头渲染为排序按钮，点击通过 update:sort 事件交给父级走后端排序。 */
    sortable?: boolean
    headerClass?: string
    cellClass?: string
    /** 列显示控制里展示的名称；缺省用列 id。 */
    toggleLabel?: string
  }
}
