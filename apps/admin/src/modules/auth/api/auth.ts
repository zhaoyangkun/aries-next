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
  /** 一次性初始化：凭 BOOTSTRAP_SECRET 创建首个 Owner 并建立 Session。 */
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

  /** 查询系统是否已完成初始化（决定登录页展示登录还是初始化入口）。 */
  async bootstrapStatus() {
    const { data } = await api.get<{ initialized: boolean }>('/api/admin/bootstrap/status')
    return data
  },

  /** 登录：Backend 先撤销旧 Session 再签发新 Session（防 Session Fixation），返回新会话。 */
  async login(payload: { login: string; password: string }) {
    const { data } = await api.post<SessionResponse>('/api/admin/auth/login', payload)
    return data
  },

  /** 登出并撤销当前 Session。 */
  async logout() {
    await api.post('/api/admin/auth/logout')
  },

  /** 恢复会话：App 启动时校验 Cookie 中的 Session 是否仍然有效。 */
  async session() {
    const { data } = await api.get<SessionResponse>('/api/admin/auth/session')
    return data
  },

  /** 请求发送密码重置邮件。 */
  async forgotPassword(email: string) {
    const { data } = await api.post<{ message: string }>('/api/admin/auth/password/forgot', { email })
    return data
  },

  /** 凭重置 Token 设置新密码；成功后 Backend 撤销该用户全部 Session。 */
  async resetPassword(token: string, password: string) {
    await api.post('/api/admin/auth/password/reset', { token, password })
  },

  /** 读取当前用户资料。 */
  async getProfile() {
    const { data } = await api.get<AuthUser>('/api/admin/profile')
    return data
  },

  /** 更新当前用户资料（email/display_name/avatar_url）。 */
  async updateProfile(payload: { email: string; display_name: string; avatar_url: string | null }) {
    const { data } = await api.put<AuthUser>('/api/admin/profile', payload)
    return data
  },

  /** 修改密码：需提供当前密码；成功后 Backend 撤销该用户全部 Session。 */
  async updatePassword(current_password: string, new_password: string) {
    await api.put('/api/admin/profile/password', { current_password, new_password })
  },
}
