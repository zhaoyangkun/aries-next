import { api } from '@/shared/api/client'

export { getApiError, isUnauthorized } from '@/shared/api/client'

export interface AuthUser {
  id: number
  username: string
  email: string
  display_name: string
  avatar_url: string | null
  role: 'owner' | 'editor' | 'moderator'
  permissions: PermissionName[]
}

export type PermissionName =
  | 'dashboard:view'
  | 'content:manage'
  | 'comments:moderate'
  | 'users:manage'
  | 'settings:manage'
  | 'profile:manage'

export interface SessionResponse {
  user: AuthUser
  expires_at: string
}

export const authApi = {
  async bootstrap(payload: {
    bootstrap_secret: string
    username: string
    email: string
    display_name: string
    password: string
  }) {
    const { data } = await api.post<SessionResponse>('/api/admin/bootstrap', payload)
    return data
  },

  async bootstrapStatus() {
    const { data } = await api.get<{ initialized: boolean }>('/api/admin/bootstrap/status')
    return data
  },

  async login(payload: { login: string; password: string }) {
    const { data } = await api.post<SessionResponse>('/api/admin/auth/login', payload)
    return data
  },

  async logout() {
    await api.post('/api/admin/auth/logout')
  },

  async session() {
    const { data } = await api.get<SessionResponse>('/api/admin/auth/session')
    return data
  },

  async forgotPassword(email: string) {
    const { data } = await api.post<{ message: string }>('/api/admin/auth/password/forgot', { email })
    return data
  },

  async resetPassword(token: string, password: string) {
    await api.post('/api/admin/auth/password/reset', { token, password })
  },

  async getProfile() {
    const { data } = await api.get<AuthUser>('/api/admin/profile')
    return data
  },

  async updateProfile(payload: { email: string; display_name: string; avatar_url: string | null }) {
    const { data } = await api.put<AuthUser>('/api/admin/profile', payload)
    return data
  },

  async updatePassword(current_password: string, new_password: string) {
    await api.put('/api/admin/profile/password', { current_password, new_password })
  },
}
