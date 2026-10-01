import tailwindcss from '@tailwindcss/vite'

// 后端 Public API 地址：仅服务端（SSR 渲染与 sitemap/rss 等 Server Route）使用，
// 属于 Nuxt 私有 Runtime Config（不内联进客户端 Bundle），运行期由环境变量
// NUXT_INTERNAL_API_BASE 覆盖——镜像一次构建、任意域名部署；浏览器端请求固定走
// 同源 /api/public（生产由 Caddy 转发到后端，开发由下方 nitro devProxy 转发），
// 与域名无关。开发默认值指向本机后端。
const internalApiBase = process.env.NUXT_INTERNAL_API_BASE ?? 'http://localhost:8088/api/public'

export default defineNuxtConfig({
  compatibilityDate: '2026-08-01',
  devtools: { enabled: true },
  css: ['~/assets/css/main.css'],
  vite: {
    plugins: [tailwindcss()],
  },
  runtimeConfig: {
    // 私有段：绝不序列化到客户端，容器内用容器网络地址（compose 注入）
    internalApiBase,
    public: {
      // Canonical 绝对地址的兜底；优先使用站点设置里的 site_url
      siteUrl: process.env.NUXT_PUBLIC_SITE_URL ?? '',
    },
  },
  nitro: {
    // 开发环境把 /api 代理到后端：正文中的相对媒体 URL（/api/media/files/...）
    // 与密码文章的 HttpOnly Cookie 都需要同源访问（后端 CORS 只放行 Admin 来源）
    devProxy: {
      '/api': { target: `${new URL(internalApiBase).origin}/api`, changeOrigin: true },
    },
  },
  routeRules: {
    '/**': {
      headers: {
        'X-Content-Type-Options': 'nosniff',
        'X-Frame-Options': 'DENY',
        'Referrer-Policy': 'strict-origin-when-cross-origin',
        // CSP 先只给 frame-ancestors，避免误伤 Nuxt 内联脚本；与 Backend 安全头重复无妨（值一致）
        'Content-Security-Policy': "frame-ancestors 'none'",
      },
    },
  },
  app: {
    head: {
      htmlAttrs: { lang: 'zh-CN' },
      meta: [{ name: 'description', content: 'Aries blog' }],
    },
  },
})
