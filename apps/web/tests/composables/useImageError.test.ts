import { describe, expect, it } from 'vitest'

import { useImageError } from '../../app/composables/useImageError'

describe('useImageError', () => {
  it('starts unflagged and flags after a load error', () => {
    const { failed, onImageError } = useImageError()

    expect(failed.value).toBe(false)

    onImageError()

    expect(failed.value).toBe(true)
  })

  it('keeps independent state per call', () => {
    const first = useImageError()
    const second = useImageError()

    first.onImageError()

    expect(first.failed.value).toBe(true)
    expect(second.failed.value).toBe(false)
  })
})
