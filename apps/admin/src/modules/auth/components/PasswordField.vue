<script setup lang="ts">
import { ref } from 'vue'
import { EyeIcon, EyeOffIcon } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import FieldMessage from './FieldMessage.vue'

withDefaults(
  defineProps<{
    id: string
    modelValue: string
    label: string
    autocomplete: string
    placeholder?: string
    hint?: string
    error?: string
    disabled?: boolean
    minlength?: number
    maxlength?: number
  }>(),
  {
    placeholder: '',
    hint: '',
    error: '',
    disabled: false,
    minlength: undefined,
    maxlength: undefined,
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const visible = ref(false)
</script>

<template>
  <div class="grid gap-2">
    <div class="flex items-center justify-between gap-3">
      <label :for="id" class="text-sm font-medium leading-none">{{ label }}</label>
      <slot name="action" />
    </div>
    <div class="relative">
      <Input
        :id="id"
        :model-value="modelValue"
        :type="visible ? 'text' : 'password'"
        :autocomplete="autocomplete"
        :placeholder="placeholder"
        :disabled="disabled"
        :minlength="minlength"
        :maxlength="maxlength"
        :aria-invalid="Boolean(error)"
        :aria-describedby="error || hint ? `${id}-message` : undefined"
        class="pr-10"
        @update:model-value="emit('update:modelValue', String($event ?? ''))"
      />
      <Button
        type="button"
        variant="ghost"
        size="icon"
        class="absolute right-0 top-0 size-9 text-muted-foreground hover:bg-transparent hover:text-foreground"
        :disabled="disabled"
        :aria-label="visible ? '隐藏密码' : '显示密码'"
        @click="visible = !visible"
      >
        <EyeOffIcon v-if="visible" class="size-4" />
        <EyeIcon v-else class="size-4" />
      </Button>
    </div>
    <FieldMessage
      v-if="error || hint"
      :id="`${id}-message`"
      :message="error || hint"
      :error="Boolean(error)"
    />
  </div>
</template>
