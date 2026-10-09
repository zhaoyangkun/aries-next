import { describe, expect, it } from 'vitest'

import {
  clampScale,
  isDoubleTap,
  pinchTransform,
  shouldCloseSwipe,
  swipeStep,
} from '../../app/utils/lightboxGestures'

describe('clampScale', () => {
  it('clamps to bounds', () => {
    expect(clampScale(0.05, 0.1, 20)).toBe(0.1)
    expect(clampScale(25, 0.1, 20)).toBe(20)
    expect(clampScale(2, 0.1, 20)).toBe(2)
  })
})

describe('pinchTransform', () => {
  const base = {
    startCenter: { x: 10, y: 20 },
    startScale: 1,
    startTranslate: { x: 0, y: 0 },
    minScale: 0.1,
    maxScale: 20,
  }

  it('scales by distance ratio around a static midpoint', () => {
    const next = pinchTransform({
      ...base,
      startDistance: 100,
      currentDistance: 200,
      currentCenter: { x: 10, y: 20 },
    })
    expect(next.scale).toBe(2)
    // 中点不动时退化为 setScale 的中心缩放公式：t = c - ratio * (c - t0)
    expect(next.translateX).toBeCloseTo(10 - 2 * (10 - 0))
    expect(next.translateY).toBeCloseTo(20 - 2 * (20 - 0))
  })

  it('keeps the content point under the initial midpoint under the current midpoint', () => {
    const next = pinchTransform({
      ...base,
      startDistance: 100,
      currentDistance: 150,
      currentCenter: { x: 40, y: -10 },
    })
    // 锚定不变式：(c1 - t1) / s1 === (c0 - t0) / s0
    expect((40 - next.translateX) / next.scale).toBeCloseTo(10)
    expect((-10 - next.translateY) / next.scale).toBeCloseTo(20)
  })

  it('clamps scale and keeps the anchor invariant at the clamp boundary', () => {
    const next = pinchTransform({
      ...base,
      startDistance: 100,
      currentDistance: 5000,
      currentCenter: { x: 0, y: 0 },
    })
    expect(next.scale).toBe(20)
    expect((0 - next.translateX) / next.scale).toBeCloseTo(10)
  })

  it('pans when midpoint moves at constant distance', () => {
    const next = pinchTransform({
      ...base,
      startDistance: 100,
      currentDistance: 100,
      currentCenter: { x: 15, y: 30 },
    })
    expect(next.scale).toBe(1)
    expect(next.translateX).toBeCloseTo(5)
    expect(next.translateY).toBeCloseTo(10)
  })

  it('returns the start state when the start distance is zero', () => {
    const next = pinchTransform({
      ...base,
      startDistance: 0,
      currentDistance: 100,
      currentCenter: { x: 99, y: 99 },
    })
    expect(next).toEqual({ scale: 1, translateX: 0, translateY: 0 })
  })
})

describe('swipeStep', () => {
  it('switches on horizontal displacement beyond the threshold', () => {
    expect(swipeStep(-100, 10, 300)).toBe(1)
    expect(swipeStep(100, -10, 300)).toBe(-1)
  })

  it('ignores vertically dominated movement', () => {
    expect(swipeStep(-80, 90, 300)).toBe(0)
    expect(swipeStep(50, 60, 300)).toBe(0)
  })

  it('ignores slow short movement', () => {
    expect(swipeStep(-40, 0, 500)).toBe(0)
  })

  it('accepts a fast flick below the distance threshold', () => {
    // 30px / 50ms = 0.6 px/ms
    expect(swipeStep(-30, 5, 50)).toBe(1)
    expect(swipeStep(30, 5, 50)).toBe(-1)
  })

  it('rejects tiny jitter even at high velocity', () => {
    // 10px < minFlickDistance，速度再高也不切换
    expect(swipeStep(-10, 0, 10)).toBe(0)
  })
})

describe('shouldCloseSwipe', () => {
  it('closes on a long downward swipe', () => {
    expect(shouldCloseSwipe(150, 20, 300)).toBe(true)
  })

  it('closes on a fast downward flick', () => {
    // 90px < 120 阈值，但 90/100 = 0.9 px/ms 超过速度阈值
    expect(shouldCloseSwipe(90, 10, 100)).toBe(true)
  })

  it('rejects upward or horizontal movement', () => {
    expect(shouldCloseSwipe(-150, 0, 300)).toBe(false)
    expect(shouldCloseSwipe(130, 120, 300)).toBe(false)
  })

  it('rejects slow short downward drags', () => {
    expect(shouldCloseSwipe(80, 10, 800)).toBe(false)
  })
})

describe('isDoubleTap', () => {
  it('matches taps inside the time window and radius', () => {
    expect(isDoubleTap(200, 10)).toBe(true)
    expect(isDoubleTap(300, 32)).toBe(true)
  })

  it('rejects taps outside the window or radius', () => {
    expect(isDoubleTap(301, 10)).toBe(false)
    expect(isDoubleTap(200, 40)).toBe(false)
  })
})
