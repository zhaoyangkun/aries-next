<script setup lang="ts">
// 把 text 中的 keywords 渲染为 <mark> 高亮；无关键词时原样输出
import { splitByKeywords } from '~/utils/highlight'

const props = defineProps<{
  text: string
  keywords?: string[]
}>()

const segments = computed(() => splitByKeywords(props.text, props.keywords ?? []))
</script>

<template>
  <template v-for="(segment, index) in segments" :key="index">
    <mark v-if="segment.match" class="search-mark">{{ segment.text }}</mark>
    <template v-else>{{ segment.text }}</template>
  </template>
</template>
