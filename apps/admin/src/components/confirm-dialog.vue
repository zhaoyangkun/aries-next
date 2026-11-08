<script setup lang="ts">
/** 确认对话框：基于 AlertDialog 的危险操作二次确认。Props 见 ConfirmDialogProps。 */
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'

interface ConfirmDialogProps {
  isLoading?: boolean
  disabled?: boolean
  cancelButtonText?: string
  confirmButtonText?: string
  destructive?: boolean
}

const {
  isLoading = false,
  disabled = false,
  destructive = false,
  cancelButtonText = 'Cancel',
  confirmButtonText = 'Continue',
} = defineProps<ConfirmDialogProps>()

const emits = defineEmits<{
  confirm: []
}>()

const openModel = defineModel<boolean>('open', {
  default: false,
})

function handleConfirm() {
  emits('confirm')
  openModel.value = false
}
</script>

<template>
  <AlertDialog :open="openModel">
    <AlertDialogContent>
      <AlertDialogHeader class="text-start">
        <AlertDialogTitle>
          <slot name="title" />
        </AlertDialogTitle>
        <AlertDialogDescription as-child>
          <slot name="description" />
        </AlertDialogDescription>
      </AlertDialogHeader>

      <slot />

      <AlertDialogFooter>
        <AlertDialogCancel :disabled="isLoading" @click="openModel = false">
          {{ cancelButtonText }}
        </AlertDialogCancel>

        <Button
          :variant="destructive ? 'destructive' : 'default'"
          :disabled="disabled || isLoading"
          @click="handleConfirm"
        >
          {{ confirmButtonText }}
        </Button>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
