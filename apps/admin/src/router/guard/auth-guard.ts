import type { Router } from 'vue-router'

import pinia from '@/plugins/pinia/setup'
import { useSessionStore } from '@/modules/auth/stores/session'
import type { PermissionName } from '@/modules/auth/api/auth'

// 认证语义：页面默认需要登录；meta.public 的页面（认证页、错误页）例外。
// 权限语义：meta.permission 缺失时不校验；校验失败进 403。
export function setupAuthGuard(router: Router) {
  router.beforeEach(async (to) => {
    const session = useSessionStore(pinia)
    if (session.status === 'unknown') {
      await session.restore()
    }

    const isPublic = to.meta.public === true

    if (!isPublic && session.status === 'unavailable') {
      return { path: '/errors/503', query: { redirect: to.fullPath } }
    }

    if (!isPublic && !session.isAuthenticated) {
      return { path: '/auth/sign-in', query: { redirect: to.fullPath } }
    }

    if (isPublic && session.isAuthenticated && to.path.startsWith('/auth')) {
      return { path: '/' }
    }

    const permission = to.meta.permission as PermissionName | undefined
    if (permission && session.isAuthenticated && !session.hasPermission(permission)) {
      return { path: '/errors/403' }
    }

    return true
  })
}
