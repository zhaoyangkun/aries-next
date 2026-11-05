import { describe, expect, it } from 'vitest'

import { SOCIAL_ICON_KEYS, socialIconMarkup } from '../../app/utils/socialIcons'

describe('socialIconMarkup', () => {
  it('returns markup for every key in the whitelist', () => {
    for (const key of SOCIAL_ICON_KEYS) {
      const markup = socialIconMarkup(key)
      expect(markup).toMatch(/^<(path|circle|rect)/)
      expect(markup).toContain('<')
    }
  })

  it('falls back to the generic link icon for unknown or missing keys', () => {
    const fallback = socialIconMarkup('link')

    expect(socialIconMarkup('unknown-key')).toBe(fallback)
    expect(socialIconMarkup('')).toBe(fallback)
    expect(socialIconMarkup(null)).toBe(fallback)
    expect(socialIconMarkup(undefined)).toBe(fallback)
  })

  it('never injects the raw key into markup', () => {
    const malicious = '"><script>alert(1)</script>'

    expect(socialIconMarkup(malicious)).toBe(socialIconMarkup('link'))
    expect(socialIconMarkup(malicious)).not.toContain('<script')
  })
})
