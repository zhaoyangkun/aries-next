<script setup lang="ts">
/** 命令面板条目（带图标包装）。Props: name/icon。 */
import type { Component } from 'vue'

import { MilestoneIcon } from '@lucide/vue'

const { icon } = defineProps<{
  name: string
  icon?: Component
}>()
</script>

<template>
  <div class="flex items-center gap-2">
    <component :is="icon" v-if="icon" class="size-4" />
    <MilestoneIcon v-else class="size-4" />
    {{ name }}
  </div>
</template>
