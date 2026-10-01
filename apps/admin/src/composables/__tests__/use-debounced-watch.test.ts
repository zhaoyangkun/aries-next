import { effectScope, nextTick, ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useDebouncedWatch } from '../use-debounced-watch'

describe('useDebouncedWatch', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('delays callback execution by the given delay', async () => {
    const source = ref('')
    const callback = vi.fn()
    const scope = effectScope()

    scope.run(() => useDebouncedWatch(source, callback, 300))

    source.value = 'a'
    await nextTick()
    expect(callback).not.toHaveBeenCalled()

    vi.advanceTimersByTime(300)
    expect(callback).toHaveBeenCalledTimes(1)
  })

  it('only keeps the last trigger while debouncing', async () => {
    const source = ref('')
    const callback = vi.fn()
    const scope = effectScope()

    scope.run(() => useDebouncedWatch(source, callback, 300))

    source.value = 'a'
    await nextTick()
    vi.advanceTimersByTime(100)
    source.value = 'ab'
    await nextTick()
    vi.advanceTimersByTime(100)
    source.value = 'abc'
    await nextTick()
    vi.advanceTimersByTime(300)

    expect(callback).toHaveBeenCalledTimes(1)
  })

  it('clears the pending timer when the scope is disposed', async () => {
    const source = ref('')
    const callback = vi.fn()
    const scope = effectScope()

    scope.run(() => useDebouncedWatch(source, callback, 300))

    source.value = 'a'
    await nextTick()
    scope.stop()
    vi.advanceTimersByTime(1000)

    expect(callback).not.toHaveBeenCalled()
  })

  it('supports multiple sources', async () => {
    const keyword = ref('')
    const status = ref('all')
    const callback = vi.fn()
    const scope = effectScope()

    scope.run(() => useDebouncedWatch([keyword, status], callback, 300))

    keyword.value = 'a'
    await nextTick()
    vi.advanceTimersByTime(300)
    status.value = 'pending'
    await nextTick()
    vi.advanceTimersByTime(300)

    expect(callback).toHaveBeenCalledTimes(2)
  })
})
