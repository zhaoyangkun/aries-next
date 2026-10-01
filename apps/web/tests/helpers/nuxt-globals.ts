import { vi } from 'vitest'

// Nuxt 自动导入全局函数的最小替身，供脱离 Nuxt 运行时的单测使用。
// 每个测试文件应在 afterEach 中调用 vi.unstubAllGlobals() 清理。

/** useState 替身：按 key 缓存的 value 容器（返回 Map 便于断言缓存行为） */
export function stubUseState(): Map<string, unknown> {
  const store = new Map<string, unknown>()
  vi.stubGlobal('useState', (key: string, init?: () => unknown) => {
    if (!store.has(key)) store.set(key, init ? init() : undefined)
    return {
      get value() {
        return store.get(key)
      },
      set value(v: unknown) {
        store.set(key, v)
      },
    }
  })
  return store
}

/** useRuntimeConfig 替身：publicConfig 进 public 段，privateConfig 平铺为顶层私有键（如 internalApiBase） */
export function stubRuntimeConfig(
  publicConfig: Record<string, unknown>,
  privateConfig: Record<string, unknown> = {},
) {
  vi.stubGlobal('useRuntimeConfig', () => ({ ...privateConfig, public: publicConfig }))
}
