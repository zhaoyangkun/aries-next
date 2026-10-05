import { describe, expect, it } from 'vitest'

import { isRegisteredRoute } from '../../app/utils/routeMatch'

// 与 pages/ 目录对应的扁平化路由表（: 开头为动态段）
const routes = [
  { path: '/' },
  { path: '/search' },
  { path: '/about' },
  { path: '/archives' },
  { path: '/categories' },
  { path: '/categories/:slug' },
  { path: '/tags' },
  { path: '/tags/:slug' },
  { path: '/articles/:slug' },
  { path: '/custom/:slug' },
]

describe('isRegisteredRoute', () => {
  it('matches static routes exactly', () => {
    expect(isRegisteredRoute(routes, '/')).toBe(true)
    expect(isRegisteredRoute(routes, '/about')).toBe(true)
    expect(isRegisteredRoute(routes, '/archives')).toBe(true)
  })

  it('matches dynamic segments', () => {
    expect(isRegisteredRoute(routes, '/categories/rust')).toBe(true)
    expect(isRegisteredRoute(routes, '/articles/hello-world')).toBe(true)
    expect(isRegisteredRoute(routes, '/custom/links')).toBe(true)
  })

  it('ignores query string and hash', () => {
    expect(isRegisteredRoute(routes, '/search?q=rust')).toBe(true)
    expect(isRegisteredRoute(routes, '/about#team')).toBe(true)
  })

  it('tolerates trailing slashes', () => {
    expect(isRegisteredRoute(routes, '/about/')).toBe(true)
    expect(isRegisteredRoute(routes, '/')).toBe(true)
  })

  it('rejects same-origin paths owned by the backend', () => {
    expect(isRegisteredRoute(routes, '/admin')).toBe(false)
    expect(isRegisteredRoute(routes, '/admin/')).toBe(false)
    expect(isRegisteredRoute(routes, '/api/health/live')).toBe(false)
  })

  it('rejects unknown paths and prefix-only matches', () => {
    expect(isRegisteredRoute(routes, '/nothing-here')).toBe(false)
    expect(isRegisteredRoute(routes, '/categories/rust/extra')).toBe(false)
    expect(isRegisteredRoute(routes, '/categories/')).toBe(true)
  })
})
