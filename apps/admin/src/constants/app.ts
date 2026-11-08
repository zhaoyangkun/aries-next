/**
 * Application-wide configuration defaults
 *
 * This file consolidates small, cross-cutting config values
 * that don't belong to a specific domain (theme, navigation, etc.).
 */

// ── Pagination ────────────────────────────────────────────
export const DEFAULT_PAGE_SIZE = 10 as const
/** 列表分页的可选每页条数。 */
export const PAGE_SIZES = [10, 20, 50, 100] as const

// ── Routing ───────────────────────────────────────────────
export const HOME_PATH = '/dashboard' as const
/** 登录页路由路径（路由守卫与 401 拦截跳转共用）。 */
export const LOGIN_PATH = '/auth/sign-in' as const
