import axios from 'axios'

export interface ApiErrorResponse {
  error?: {
    code?: string
    message?: string
    details?: Record<string, unknown>
  }
}

export const api = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL ?? '',
  withCredentials: true,
  // 不预设 Content-Type：Axios 会为普通对象自动设置 application/json，
  // FormData 则交给浏览器生成带 boundary 的 multipart 头；预设 JSON 头会让
  // Axios 把 FormData 序列化成 JSON 字符串，导致媒体上传/Markdown 导入被 Backend 拒绝。
})

export function getApiError(error: unknown, fallback = '请求失败，请稍后重试') {
  if (axios.isAxiosError<ApiErrorResponse>(error)) {
    return error.response?.data?.error?.message ?? fallback
  }
  return fallback
}

// 读取 Backend 统一错误结构中的业务错误码，用于按 code 分支处理（如版本冲突）。
export function getApiErrorCode(error: unknown) {
  if (axios.isAxiosError<ApiErrorResponse>(error)) {
    return error.response?.data?.error?.code
  }
  return undefined
}

export function isUnauthorized(error: unknown) {
  return axios.isAxiosError(error) && error.response?.status === 401
}

/**
 * 登录回跳等场景只接受站内路径，防开放重定向。
 */
export function safeInternalPath(path: string): string {
  if (path.startsWith('/') && !path.startsWith('//')) return path
  return '/'
}

type UnauthorizedHandler = () => void
let unauthorizedHandler: UnauthorizedHandler | null = null

/**
 * 注册全局 401 处理（App 启动时调用一次）。
 * client 是模块级单例，不直接依赖 Pinia/Router，避免与业务 API 层循环依赖；
 * 具体的会话清理与跳转逻辑由调用方注入。
 */
export function setUnauthorizedHandler(handler: UnauthorizedHandler | null) {
  unauthorizedHandler = handler
}

// 全局 401 拦截：Session 已被服务端判定失效时，交由注册的 Handler 清理本地会话并跳转登录页。
// 错误照常 reject，调用方仍按原逻辑处理（如表单内联报错）。
api.interceptors.response.use(
  (response) => response,
  (error: unknown) => {
    if (isUnauthorized(error)) unauthorizedHandler?.()
    return Promise.reject(error)
  },
)
