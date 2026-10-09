// 灯箱触摸手势的纯函数：pinch 中心缩放换算、滑动切换/下滑关闭判定、双击判定。
// 抽成纯函数便于单测，组件内只负责 Pointer Events 状态机与状态落地。

export interface GestureVector {
  x: number
  y: number
}

export function clampScale(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value))
}

/**
 * 双指捏合缩放：按间距比例换算新缩放系数，并保持「初始中点下的内容点」
 * 始终位于当前中点之下（transform 为 translate + scale、原点为容器中心时，
 * 屏幕上 contentPoint * scale + translate = 触点）。
 */
export function pinchTransform(params: {
  startDistance: number
  currentDistance: number
  /** 初始两指中点（相对容器中心） */
  startCenter: GestureVector
  /** 当前两指中点（相对容器中心） */
  currentCenter: GestureVector
  startScale: number
  startTranslate: GestureVector
  minScale: number
  maxScale: number
}): { scale: number; translateX: number; translateY: number } {
  if (params.startDistance <= 0) {
    return {
      scale: params.startScale,
      translateX: params.startTranslate.x,
      translateY: params.startTranslate.y,
    }
  }
  const scale = clampScale(
    params.startScale * (params.currentDistance / params.startDistance),
    params.minScale,
    params.maxScale,
  )
  const ratio = scale / params.startScale
  return {
    scale,
    translateX: params.currentCenter.x - ratio * (params.startCenter.x - params.startTranslate.x),
    translateY: params.currentCenter.y - ratio * (params.startCenter.y - params.startTranslate.y),
  }
}

/**
 * 横向滑动切换判定：横向主导且位移超过阈值，或位移不大但速度够快（轻扫）。
 * 返回切换步进：左滑 +1（下一张）、右滑 -1（上一张）、不切换 0。
 */
export function swipeStep(
  dx: number,
  dy: number,
  durationMs: number,
  minDistance = 60,
  minVelocity = 0.3,
  minFlickDistance = 24,
): number {
  if (Math.abs(dx) <= Math.abs(dy)) return 0
  const velocity = durationMs > 0 ? Math.abs(dx) / durationMs : 0
  if (Math.abs(dx) >= minDistance || (Math.abs(dx) >= minFlickDistance && velocity >= minVelocity)) {
    return dx < 0 ? 1 : -1
  }
  return 0
}

/**
 * 下滑关闭判定：阈值偏保守，要求明显竖直向下，避免与横向切换/跟手回弹混淆。
 */
export function shouldCloseSwipe(
  dy: number,
  dx: number,
  durationMs: number,
  minDistance = 120,
  minVelocity = 0.6,
): boolean {
  if (dy <= 0) return false
  if (Math.abs(dx) * 1.25 > dy) return false
  const velocity = durationMs > 0 ? dy / durationMs : 0
  return dy >= minDistance || velocity >= minVelocity
}

/** 双击/双指点按判定：两次点按的时间与位置都要落在窗口内 */
export function isDoubleTap(
  elapsedMs: number,
  distancePx: number,
  windowMs = 300,
  radiusPx = 32,
): boolean {
  return elapsedMs <= windowMs && distancePx <= radiusPx
}
