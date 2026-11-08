import type { App } from 'vue'

import router from '@/router'

/** 创建 vue-router 实例并挂载到应用。 */
export function setupRouter(app: App) {
  app.use(router)
}
