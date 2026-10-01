<script setup lang="ts">
// 导航节点渲染：href 已由服务端解析，站内路由用 NuxtLink（保留 router-link-active 激活态），
// 外链用原生 <a>；open_in_new_tab 或外链一律补 rel="noopener" 防止反向标签劫持
import type { PublicNavigationNode } from '~/composables/usePublicApi'

const props = defineProps<{ node: PublicNavigationNode }>()

const isExternal = computed(() => /^(https?:)?\/\//.test(props.node.href ?? ''))
const target = computed(() => (props.node.open_in_new_tab ? '_blank' : undefined))
const rel = computed(() =>
  props.node.open_in_new_tab || isExternal.value ? 'noopener' : undefined,
)
</script>

<template>
  <span v-if="!node.href">{{ node.label }}</span>
  <a v-else-if="isExternal" :href="node.href" :target="target" :rel="rel">{{ node.label }}</a>
  <NuxtLink v-else :to="node.href" :target="target" :rel="rel">{{ node.label }}</NuxtLink>
</template>
