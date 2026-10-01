import { defineStore } from 'pinia'

import { useSessionStore } from '@/modules/auth/stores/session'

// 模板壳与业务 Session 之间的桥：isLogin 供路由 Guard 与布局组件使用。
export const useAuthStore = defineStore('user', () => {
  const session = useSessionStore()
  const isLogin = computed(() => session.isAuthenticated)

  return {
    isLogin,
  }
})
