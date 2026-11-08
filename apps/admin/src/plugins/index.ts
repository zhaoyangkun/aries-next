import type { App } from 'vue'

import { setupUnauthorizedHandler } from '@/shared/api/unauthorized'

import { setupAutoAnimate } from './auto-animate/setup'
import { setupDayjs } from './dayjs/setup'
import { setupI18n } from './i18n/setup'
import { setupNProgress } from './nprogress/setup'
import { setupPinia } from './pinia/setup'
import { setupRouter } from './router/setup'

/** 统一装配应用插件；顺序敏感：Pinia 必须先于 Router（守卫依赖 Store），全局 401 处理最后注册。 */
export function setupPlugins(app: App) {
  setupDayjs()
  setupNProgress()
  setupAutoAnimate(app)
  setupI18n(app)
  setupPinia(app)
  setupRouter(app)
  setupUnauthorizedHandler()
}
