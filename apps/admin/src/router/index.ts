import { setupLayouts } from 'virtual:generated-layouts'
import { createRouter, createWebHistory } from 'vue-router'
import { handleHotUpdate, routes } from 'vue-router/auto-routes'

import { setupRouterGuard } from './guard'

const router = createRouter({
  // BASE_URL 随构建 base 变化：vite dev 默认为 '/'（5173 直连），
  // 生产构建传 --base=/admin/ 时由 aries-server 挂载在 /admin/ 下。
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: setupLayouts(routes),

  scrollBehavior() {
    return { left: 0, top: 0, behavior: 'smooth' }
  },
})

setupRouterGuard(router)

export default router

if (import.meta.hot) {
  handleHotUpdate(router)
}
