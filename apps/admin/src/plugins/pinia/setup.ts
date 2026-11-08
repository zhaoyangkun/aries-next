import type { App } from 'vue'

import { createPinia } from 'pinia'
import { createPersistedState } from 'pinia-plugin-persistedstate'

const pinia = createPinia()

const persistedState = createPersistedState({
  storage: sessionStorage,
})
pinia.use(persistedState)

/** 创建 Pinia 实例并启用 sessionStorage 持久化插件（主题/布局偏好刷新不丢），同时导出实例供模块外使用。 */
export function setupPinia(app: App) {
  app.use(pinia)
}

export default pinia
