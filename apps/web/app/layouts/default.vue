<script setup lang="ts">
// 站点设置驱动导航名称、页脚 ICP 备案与全局 Title 模板
import type { PublicNavigationNode } from '~/composables/usePublicApi'

const site = await useSite()
const navigation = await useNavigation()
const { toggleTheme } = useTheme()
const { show: showSearchPalette } = useSearchPalette()

// Ctrl/⌘+K：全局唤起搜索弹层（SearchPalette 自身管理关闭与焦点）
onMounted(() => {
  const onGlobalKey = (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault()
      showSearchPalette()
    }
  }
  window.addEventListener('keydown', onGlobalKey)
  onBeforeUnmount(() => window.removeEventListener('keydown', onGlobalKey))
})

// 站点运行天数：created_at 缺失/非法时为 0，模板据此隐藏
const runDays = computed(() => siteRunDays(site.value.created_at))

// header 滚动后加毛玻璃与细边框；SSR 初始不带该 class，挂载后再读滚动位置，保证 Hydration 一致
const scrolled = ref(false)
onMounted(() => {
  const onScroll = () => {
    scrolled.value = window.scrollY > 8
  }
  onScroll()
  window.addEventListener('scroll', onScroll, { passive: true })
  onBeforeUnmount(() => window.removeEventListener('scroll', onScroll))
})

// 内置默认菜单始终展示，覆盖站内标准页面；后台配置的导航项追加其后。
// href 相同的后台项覆盖对应默认项（可自定义文案/打开方式），避免重复。
const DEFAULT_NAV: PublicNavigationNode[] = [
  { label: '首页', target_type: 'url', target_id: null, url: '/', href: '/', open_in_new_tab: false, children: [] },
  { label: '分类', target_type: 'url', target_id: null, url: '/categories', href: '/categories', open_in_new_tab: false, children: [] },
  { label: '标签', target_type: 'url', target_id: null, url: '/tags', href: '/tags', open_in_new_tab: false, children: [] },
  { label: '归档', target_type: 'url', target_id: null, url: '/archives', href: '/archives', open_in_new_tab: false, children: [] },
  { label: '友链', target_type: 'url', target_id: null, url: '/links', href: '/links', open_in_new_tab: false, children: [] },
  { label: '日志', target_type: 'url', target_id: null, url: '/journals', href: '/journals', open_in_new_tab: false, children: [] },
  { label: '图库', target_type: 'url', target_id: null, url: '/galleries', href: '/galleries', open_in_new_tab: false, children: [] },
]

const navItems = computed(() => {
  const configured = navigation.value
  if (configured.length === 0) return DEFAULT_NAV
  const configuredHrefs = new Set(configured.map((item) => item.href).filter(Boolean))
  const defaults = DEFAULT_NAV.filter((item) => !configuredHrefs.has(item.href))
  return [...defaults, ...configured]
})

function scrollToTop() {
  window.scrollTo({ top: 0, behavior: 'smooth' })
}

useHead(
  computed(() => ({
    // 对齐旧版规则：{副标题} - {站点名}
    titleTemplate: (title?: string) =>
      title ? `${title} - ${site.value.site_name}` : site.value.site_name,
    meta: [{ name: 'description', content: site.value.site_description || 'Aries blog' }],
  })),
)
</script>

<template>
  <div class="site-shell">
    <header class="site-header" :class="{ 'is-scrolled': scrolled }">
      <div class="site-header-inner">
        <NuxtLink class="site-name" to="/">{{ site.site_name }}</NuxtLink>
        <div class="flex min-w-0 items-center gap-1">
          <nav aria-label="主导航">
            <template v-for="(item, index) in navItems" :key="index">
              <!-- 桌面端：含子菜单的节点用 group hover/focus-within 纯 CSS 下拉，不引依赖 -->
              <div v-if="item.children.length > 0" class="nav-group hidden sm:block">
                <AppNavLink :node="item" />
                <div class="nav-dropdown">
                  <AppNavLink
                    v-for="(child, childIndex) in item.children"
                    :key="childIndex"
                    :node="child"
                  />
                </div>
              </div>
              <!-- 移动端没有悬停态，平铺父项与子项（下拉容器在 sm 以下隐藏） -->
              <template v-if="item.children.length > 0">
                <AppNavLink :node="item" class="sm:hidden" />
                <AppNavLink
                  v-for="(child, childIndex) in item.children"
                  :key="`m-${childIndex}`"
                  :node="child"
                  class="sm:hidden"
                />
              </template>
              <AppNavLink v-else :node="item" />
            </template>
          </nav>
          <!-- 桌面端顶栏搜索框（移动端无悬停/空间，保留搜索页链接） -->
          <AppSearchBox />
          <NuxtLink to="/search" class="sm:hidden">搜索</NuxtLink>
          <!-- 昼夜切换：内联 SVG 太阳/月亮，显隐由 html.dark 的 CSS 控制 -->
          <button
            type="button"
            class="theme-toggle"
            aria-label="切换亮色 / 暗色主题"
            @click="toggleTheme"
          >
            <svg
              class="icon-sun"
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <circle cx="12" cy="12" r="4" />
              <path d="M12 2v2" />
              <path d="M12 20v2" />
              <path d="m4.93 4.93 1.41 1.41" />
              <path d="m17.66 17.66 1.41 1.41" />
              <path d="M2 12h2" />
              <path d="M20 12h2" />
              <path d="m6.34 17.66-1.41 1.41" />
              <path d="m19.07 4.93-1.41 1.41" />
            </svg>
            <svg
              class="icon-moon"
              xmlns="http://www.w3.org/2000/svg"
              width="18"
              height="18"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
            </svg>
          </button>
        </div>
      </div>
    </header>
    <main class="site-main"><slot /></main>
    <!-- 右下角浮动工具栏 + 全局搜索弹层（状态经 useSearchPalette/useTocDrawer 共享） -->
    <AppToolbar />
    <SearchPalette />
    <footer class="site-footer">
      <!-- 页脚复用同一导航数据，只展示一级节点，避免页脚过重 -->
      <nav v-if="navItems.length > 0" class="site-footer-nav" aria-label="页脚导航">
        <AppNavLink
          v-for="(item, index) in navItems"
          :key="index"
          :node="item"
        />
        <NuxtLink to="/search">搜索</NuxtLink>
      </nav>
      <div class="site-footer-bottom">
        <div class="flex flex-col gap-1">
          <span>© {{ new Date().getFullYear() }} {{ site.site_name }}</span>
          <span v-if="site.site_description">{{ site.site_description }}</span>
          <span v-if="runDays > 0">本站已运行 {{ runDays }} 天</span>
        </div>
        <div class="flex items-center gap-4">
          <a
            v-if="site.icp_text"
            href="https://beian.miit.gov.cn/"
            target="_blank"
            rel="noopener noreferrer nofollow"
            class="text-muted-foreground no-underline transition-colors hover:text-primary"
          >{{ site.icp_text }}</a>
          <a
            href="#"
            class="inline-flex items-center gap-1 no-underline transition-colors hover:text-primary"
            aria-label="返回顶部"
            @click.prevent="scrollToTop"
          >
            返回顶部
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="m18 15-6-6-6 6" />
            </svg>
          </a>
        </div>
      </div>
    </footer>
  </div>
</template>
