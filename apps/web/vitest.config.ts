import { defineConfig } from 'vitest/config'

// 公开站单测只覆盖可脱离 Nuxt 运行时的纯逻辑（utils / composables / server utils），
// Nuxt 自动导入的全局函数（useRuntimeConfig、$fetch、useState 等）在测试中用 vi.stubGlobal 替身，
// 因此 Node 环境即可，不引入 @nuxt/test-utils
export default defineConfig({
  test: {
    environment: 'node',
    include: ['tests/**/*.test.ts'],
  },
})
