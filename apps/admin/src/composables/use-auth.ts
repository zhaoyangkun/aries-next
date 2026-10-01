import { safeInternalPath } from '@/shared/api/client'
import { useSessionStore } from '@/modules/auth/stores/session'

// 真实登录/登出：代理到 Session Store；登录成功后按 redirect 参数回跳。
export function useAuth() {
  const router = useRouter()
  const session = useSessionStore()
  const loading = shallowRef(false)

  async function logout() {
    await session.logout()
    router.push({ path: '/auth/sign-in' })
  }

  async function login(loginName: string, password: string) {
    loading.value = true
    try {
      await session.login(loginName, password)
      const redirect = router.currentRoute.value.query.redirect as string | undefined
      // 防开放重定向：只接受站内路径，非法值回退首页
      router.push(safeInternalPath(redirect || '/'))
    }
    finally {
      loading.value = false
    }
  }

  return {
    loading,
    logout,
    login,
  }
}
