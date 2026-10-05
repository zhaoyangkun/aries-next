<script setup lang="ts">
// 导航节点渲染：href 已由服务端解析，命中 Nuxt 路由的站内链接用 NuxtLink
// （保留 router-link-active 激活态与 SPA 跳转）；外链与未命中路由的同源路径
// （如 /admin，由后端/Caddy 处理）用原生 <a>，避免 vue-router 对未知路径刷警告。
// open_in_new_tab 或外链一律补 rel="noopener" 防止反向标签劫持
import type { PublicNavigationNode } from '~/composables/usePublicApi'

const props = defineProps<{ node: PublicNavigationNode }>()

const router = useRouter()

const isExternal = computed(() => /^(https?:)?\/\//.test(props.node.href ?? ''))
const isNuxtRoute = computed(() => {
  const href = props.node.href
  if (!href || isExternal.value) return false
  return isRegisteredRoute(router.getRoutes(), href)
})
const target = computed(() => (props.node.open_in_new_tab ? '_blank' : undefined))
const rel = computed(() =>
  props.node.open_in_new_tab || isExternal.value ? 'noopener' : undefined,
)
</script>

<template>
  <span v-if="!node.href">{{ node.label }}</span>
  <a v-else-if="!isNuxtRoute" :href="node.href" :target="target" :rel="rel">{{ node.label }}</a>
  <NuxtLink v-else :to="node.href" :target="target" :rel="rel">{{ node.label }}</NuxtLink>
</template>
