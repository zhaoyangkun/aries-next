<script setup lang="ts">
import { LoaderCircleIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

const props = withDefaults(
  defineProps<{
    open: boolean
    title: string
    description: string
    confirmLabel?: string
    destructive?: boolean
    busy?: boolean
  }>(),
  {
    confirmLabel: '确认',
    destructive: false,
    busy: false,
  },
)

const emit = defineEmits<{
  'update:open': [open: boolean]
  confirm: []
}>()

function handleOpenChange(open: boolean) {
  if (!props.busy) emit('update:open', open)
}
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="max-w-md">
      <DialogHeader>
        <DialogTitle>{{ title }}</DialogTitle>
        <DialogDescription>{{ description }}</DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <Button variant="outline" :disabled="busy" @click="emit('update:open', false)">取消</Button>
        <Button :variant="destructive ? 'destructive' : 'default'" :disabled="busy" @click="emit('confirm')">
          <LoaderCircleIcon v-if="busy" class="animate-spin" />
          {{ busy ? '正在处理' : confirmLabel }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
