<script setup lang="ts">
// 通用分页：通过 URL query (?page=N) 导航，保证分页地址可分享、可被爬虫索引。
// ghost 风格：无边框圆点，当前页红字淡红底（样式见 main.css 的 .page-circle）
const props = defineProps<{
  page: number
  pageSize: number
  total: number
}>()

const route = useRoute()

const totalPages = computed(() => Math.max(1, Math.ceil(props.total / props.pageSize)))

// 页码窗口：当前页前后各两页
const pages = computed(() => {
  const list: number[] = []
  const start = Math.max(1, props.page - 2)
  const end = Math.min(totalPages.value, props.page + 2)
  for (let i = start; i <= end; i += 1) list.push(i)
  return list
})

function pageLink(target: number) {
  // 第 1 页回到无参地址，保持 canonical 干净
  return {
    path: route.path,
    query: { ...route.query, page: target > 1 ? target : undefined },
  }
}
</script>

<template>
  <nav v-if="totalPages > 1" class="mt-12 flex flex-wrap items-center justify-center gap-2" aria-label="分页">
    <NuxtLink v-if="page > 1" :to="pageLink(page - 1)" class="page-circle" rel="prev" aria-label="上一页">
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4" aria-hidden="true">
        <path d="m15 18-6-6 6-6" />
      </svg>
    </NuxtLink>
    <span v-else class="page-circle page-circle-disabled" aria-disabled="true">
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4" aria-hidden="true">
        <path d="m15 18-6-6 6-6" />
      </svg>
    </span>

    <NuxtLink
      v-for="p in pages"
      :key="p"
      :to="pageLink(p)"
      class="page-circle"
      :class="{ 'page-circle-current': p === page }"
      :aria-current="p === page ? 'page' : undefined"
    >
      {{ p }}
    </NuxtLink>

    <NuxtLink v-if="page < totalPages" :to="pageLink(page + 1)" class="page-circle" rel="next" aria-label="下一页">
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4" aria-hidden="true">
        <path d="m9 18 6-6-6-6" />
      </svg>
    </NuxtLink>
    <span v-else class="page-circle page-circle-disabled" aria-disabled="true">
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-4" aria-hidden="true">
        <path d="m9 18 6-6-6-6" />
      </svg>
    </span>
  </nav>
</template>
