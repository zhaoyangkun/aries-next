import pinia from '@/plugins/pinia/setup'
import router from '@/router'
import { useSessionStore } from '@/modules/auth/stores/session'

import { safeInternalPath, setUnauthorizedHandler } from './client'

/**
 * 注册全局 401 拦截：任意 API 返回 401（Session 失效/被撤销）时，
 * 清理本地会话并跳转登录页，携带 redirect 供登录成功后回跳原页面
 * （回跳校验在 auth-guard 与 use-auth 两处生效）。
 */
export function setupUnauthorizedHandler() {
  setUnauthorizedHandler(() => {
    const session = useSessionStore(pinia)
    session.reset()

    const current = router.currentRoute.value
    // 已在登录页时不再重复跳转（连续多个并发 401 也只导航一次）
    if (current.path === '/auth/sign-in') return
    void router.push({
      path: '/auth/sign-in',
      query: { redirect: safeInternalPath(current.fullPath) },
    })
  })
}
