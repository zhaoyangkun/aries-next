import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import {
  authApi,
  getApiError,
  isUnauthorized,
  type AuthUser,
  type PermissionName,
} from '../api/auth'

type SessionStatus = 'unknown' | 'loading' | 'authenticated' | 'anonymous' | 'unavailable'

export const useSessionStore = defineStore('session', () => {
  const user = ref<AuthUser | null>(null)
  const status = ref<SessionStatus>('unknown')
  const error = ref('')

  const isAuthenticated = computed(() => status.value === 'authenticated' && user.value !== null)

  function hasPermission(permission: PermissionName) {
    return user.value?.permissions.includes(permission) ?? false
  }

  async function restore() {
    if (status.value === 'loading') return isAuthenticated.value
    status.value = 'loading'
    error.value = ''
    try {
      const response = await authApi.session()
      user.value = response.user
      status.value = 'authenticated'
    } catch (requestError) {
      user.value = null
      status.value = isUnauthorized(requestError) ? 'anonymous' : 'unavailable'
      error.value = getApiError(requestError)
    }
    return isAuthenticated.value
  }

  async function login(loginName: string, password: string) {
    error.value = ''
    const response = await authApi.login({ login: loginName, password })
    user.value = response.user
    status.value = 'authenticated'
  }

  async function bootstrap(payload: Parameters<typeof authApi.bootstrap>[0]) {
    error.value = ''
    const response = await authApi.bootstrap(payload)
    user.value = response.user
    status.value = 'authenticated'
  }

  async function logout() {
    try {
      await authApi.logout()
    } finally {
      user.value = null
      status.value = 'anonymous'
    }
  }

  /** 全局 401 拦截时的本地清理：Session 已被服务端判定失效，不再请求登出接口 */
  function reset() {
    user.value = null
    status.value = 'anonymous'
  }

  function setUser(nextUser: AuthUser) {
    user.value = nextUser
    status.value = 'authenticated'
  }

  return {
    user,
    status,
    error,
    isAuthenticated,
    hasPermission,
    restore,
    login,
    bootstrap,
    logout,
    reset,
    setUser,
  }
})
