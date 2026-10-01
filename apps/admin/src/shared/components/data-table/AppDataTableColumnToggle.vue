<script setup lang="ts" generic="TData">
import type { Table } from '@tanstack/vue-table'
import { Columns3Icon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import '../data-table/types'

defineProps<{
  table: Table<TData> | undefined
}>()
</script>

<template>
  <DropdownMenu v-if="table">
    <DropdownMenuTrigger as-child>
      <Button variant="outline" size="sm" class="h-8 text-xs">
        <Columns3Icon />
        列显示
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" class="w-40">
      <DropdownMenuLabel>显示列</DropdownMenuLabel>
      <DropdownMenuSeparator />
      <DropdownMenuCheckboxItem
        v-for="column in table.getAllLeafColumns().filter((item) => item.getCanHide())"
        :key="column.id"
        :model-value="column.getIsVisible()"
        @update:model-value="column.toggleVisibility($event === true)"
        @select.prevent
      >
        {{ column.columnDef.meta?.toggleLabel ?? column.id }}
      </DropdownMenuCheckboxItem>
    </DropdownMenuContent>
  </DropdownMenu>
</template>
