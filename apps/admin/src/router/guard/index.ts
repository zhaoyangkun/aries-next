import type { Router } from 'vue-router'

import { setupAuthGuard } from './auth-guard'
import { setupCommonGuard } from './common-guard'

/** 按顺序装配路由守卫：通用守卫（标题/进度条）在前，认证与权限守卫在后。 */
export function setupRouterGuard(router: Router) {
  setupCommonGuard(router)
  setupAuthGuard(router)
}
