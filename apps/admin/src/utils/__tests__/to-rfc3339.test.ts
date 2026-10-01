import { describe, expect, it } from 'vitest'

import { toRfc3339 } from '../to-rfc3339'

describe('toRfc3339', () => {
  it('returns undefined for empty value', () => {
    expect(toRfc3339('')).toBeUndefined()
  })

  it('appends :00Z to datetime-local value without seconds', () => {
    expect(toRfc3339('2026-09-12T08:30')).toBe('2026-09-12T08:30:00Z')
  })

  it('keeps seconds when already present', () => {
    expect(toRfc3339('2026-09-12T08:30:45')).toBe('2026-09-12T08:30:45Z')
  })
})
