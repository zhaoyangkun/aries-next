import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'

const api = vi.hoisted(() => ({
  session: vi.fn(),
  login: vi.fn(),
  bootstrap: vi.fn(),
  logout: vi.fn(),
  unauthorized: vi.fn(),
}))

vi.mock('../api/auth', () => ({
  authApi: api,
  getApiError: () => 'request failed',
  isUnauthorized: api.unauthorized,
}))

import { useSessionStore } from './session'

const user = {
  id: 1,
  username: 'owner',
  email: 'owner@example.com',
  display_name: 'Aries Owner',
  avatar_url: null,
  role: 'owner' as const,
  permissions: ['dashboard:view' as const],
}

describe('session store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
    api.unauthorized.mockReturnValue(true)
  })

  it('restores an authenticated session after refresh', async () => {
    api.session.mockResolvedValue({ user, expires_at: '2026-08-03T00:00:00Z' })
    const store = useSessionStore()

    await expect(store.restore()).resolves.toBe(true)
    expect(store.user).toEqual(user)
    expect(store.status).toBe('authenticated')
  })

  it('becomes anonymous when session restore fails', async () => {
    api.session.mockRejectedValue(new Error('unauthorized'))
    const store = useSessionStore()

    await expect(store.restore()).resolves.toBe(false)
    expect(store.user).toBeNull()
    expect(store.status).toBe('anonymous')
  })

  it('exposes service unavailability separately from an expired session', async () => {
    api.session.mockRejectedValue(new Error('offline'))
    api.unauthorized.mockReturnValue(false)
    const store = useSessionStore()

    await expect(store.restore()).resolves.toBe(false)
    expect(store.user).toBeNull()
    expect(store.status).toBe('unavailable')
  })

  it('clears local state even when the logout request fails', async () => {
    api.login.mockResolvedValue({ user, expires_at: '2026-08-03T00:00:00Z' })
    api.logout.mockRejectedValue(new Error('offline'))
    const store = useSessionStore()
    await store.login('owner', 'password')

    await expect(store.logout()).rejects.toThrow('offline')
    expect(store.user).toBeNull()
    expect(store.status).toBe('anonymous')
  })

  it('checks permissions from the restored session', async () => {
    api.session.mockResolvedValue({ user, expires_at: '2026-08-03T00:00:00Z' })
    const store = useSessionStore()

    await store.restore()

    expect(store.hasPermission('dashboard:view')).toBe(true)
    expect(store.hasPermission('content:manage')).toBe(false)
  })
})
