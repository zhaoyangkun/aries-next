import { defineConfig } from '@playwright/test'

// 服务地址允许通过环境变量覆盖（复用已在运行的实例），默认与本地开发端口一致：
// admin 为 Vite Dev Server（/api 代理到 Backend 8088），web 为 Nuxt SSR。
// 本配置不自动启动任何服务，前置条件见 README.md。
const adminBaseURL = process.env.E2E_ADMIN_BASE_URL ?? 'http://127.0.0.1:5173'
const webBaseURL = process.env.E2E_WEB_BASE_URL ?? 'http://127.0.0.1:3000'

export default defineConfig({
  testDir: './tests',
  timeout: 60_000,
  expect: { timeout: 10_000 },
  // 排序场景会真实改写数据库（sort_order），并行 Worker 会互相干扰，固定串行。
  workers: 1,
  retries: 0,
  reporter: [['list']],
  use: {
    locale: 'zh-CN',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'admin',
      testDir: './tests/admin',
      use: { baseURL: adminBaseURL, browserName: 'chromium' },
    },
    {
      name: 'web',
      testDir: './tests/web',
      use: { baseURL: webBaseURL, browserName: 'chromium' },
    },
  ],
})
