# Phase 04：Public Web 与 SEO

> 状态：🚧 进行中（主体功能已完成，剩余 Desktop 与 Mobile 真机视觉及可访问性检查）　最后验证日期：2026-09-06
>
> 本文档回答：Nuxt 4 Public Web 的页面范围、SEO 要求、Cache 策略与验收标准。实际交付记录见文末 §11。

## 1. 目标

将 Nuxt 4 Public Web 从 Static Prototype 建设为可公开访问、可被 Search Engine 正确收录的博客阅读端，并兼容 Article Password 与 Legacy URL。

## 2. 页面范围

- Home Article Feed。
- Article Detail。
- Category Index 与 Category Article List。
- Tag Index 与 Tag Article List。
- Archive。
- Keyword Search。
- Article Password Challenge。
- `404`、`500` 和 Maintenance State。
- Sitemap、RSS 和 Robots。

Page、Journal、Gallery 和 Link 原计划在 Phase 06 接入，现已随 Phase 06 完成（见 `phase-06-extended-content-and-appearance.md`）。

## 3. Public API Contract

```text
GET  /api/public/site
GET  /api/public/articles
GET  /api/public/articles/{slug}
POST /api/public/articles/{slug}/access
POST /api/public/articles/{slug}/views
GET  /api/public/categories
GET  /api/public/categories/{slug}/articles
GET  /api/public/tags
GET  /api/public/tags/{slug}/articles
GET  /api/public/archives
GET  /api/public/search
```

Public API 只返回 Published、未删除和满足可见性条件的数据。Password Protected Article 在授权前不得返回正文、摘要之外的敏感 Metadata 或 Asset Reference。

## 4. Backend 工作

- 为公开列表提供稳定 Sort：Pinned、Sort Order、Published At、ID。
- Article Detail 返回 Previous/Next、Category、Tags 和 Comment Policy。
- Password Challenge 成功后写入短期、限定 Article 的 HttpOnly Access Cookie。
- Search 第一版使用 PostgreSQL Full Text Search；明确中文分词限制和 Fallback Strategy。
- View Count 使用幂等或时间窗口去重，写入异步聚合，不能由客户端直接指定 Count。
- 提供 Site Setting Public Projection，不泄露 SMTP、Storage 或 AI Secret。
- 输出 Feed 所需的绝对 URL、Updated At 和 Canonical Slug。

## 5. Nuxt 工作

- 使用 `useAsyncData` 完成 SSR Data Fetch，并处理 Loading、Empty、Error。
- Article 页面渲染 Sanitized HTML，支持 Heading Anchor、Code Highlight、TOC 和 Responsive Media。
- 页面 Metadata 包含唯一 Title、Description、Canonical 和 Open Graph。
- Article 输出 JSON-LD `BlogPosting` Structured Data。
- Pagination 页面输出稳定 Canonical，并避免重复内容。
- 实现 `/sitemap.xml`、`/rss.xml`、`/robots.txt`。
- 为 Legacy URL 建立 Server Route Redirect，不使用 Client-only Redirect。
- 所有页面通过 Keyboard、Contrast、Mobile Layout 和 Long Content 检查。

## 6. Cache 策略

- Site、Category 和 Tag Metadata 可使用短期 SWR。
- Article Detail 按 Slug 缓存，并在 Publish、Update、Recycle 后执行 Cache Invalidation。
- Password Protected Article 不进入共享 Public Cache。
- Preview、Admin Cookie 和 Error Response 不被 CDN Cache。

## 7. Test 与验证

- Contract Test：Draft/Recycled Article 对 Public API 永远不可见。
- SSR Test：首个 HTML Response 已包含 Title、正文和 Canonical，不依赖 Hydration 后补充。
- SEO Test：Sitemap、RSS、Robots、Open Graph 和 JSON-LD 格式有效。
- Security Test：Password Article Cache Isolation、Brute Force Rate Limit、HTML Sanitization。
- Playwright E2E：首页 -> 分类 -> Article -> Previous/Next -> Search -> Archive。
- Performance：检查 Core Web Vitals 基线、Image Size 和 Route Payload。

## 8. Acceptance Gate

1. Published Article 可通过 Nuxt SSR 访问，Draft 和 Recycled Article 不可见。
2. Article、Category、Tag、Archive 和 Search 主流程完整。
3. Password Protected Article 不会通过 API、Cache、RSS 或 Sitemap 泄露正文。
4. Canonical、Sitemap、RSS、Robots、Open Graph 和 Structured Data 验证通过。
5. Desktop 与 Mobile 阅读体验通过视觉和可访问性检查。

## 9. 本 Phase 不做

- 不实现评论提交（评论链路属 Phase 05，现已完成）。
- 不实现 Semantic Search。
- 不实现运行时安装第三方 Nuxt Theme。

## 10. 完成后更新

- Legacy Route Redirect 草案
- SEO Checklist 与 Production Domain 配置
- Nginx/CDN Cache 和 Proxy 文档

## 11. 交付记录

已完成（对照 §8 Acceptance Gate）：

- Nuxt SSR 页面：首页 Article Feed、Article Detail（含 Previous/Next、TOC、代码高亮）、Category/Tag 索引与列表、Archive、Search、自定义页（`/custom/{slug}`）、日志、图库、友链、`404` 与 `500` 错误页。
- Public API：文章列表/详情、密码解锁（`POST /api/public/articles/{slug}/access`，短期 HttpOnly Access Cookie）、浏览计数（`POST .../views`，内存滑动窗口去重）、分类、标签、归档、搜索、站点信息，仅暴露 Published 内容；公开端点显式声明 Cache-Control（见 `backend/server/src/http/public/`）。
- SEO：每页唯一 Title/Description/Canonical/Open Graph，Article 输出 JSON-LD `BlogPosting`；`/sitemap.xml`、`/rss.xml`、`/robots.txt` 由 Nuxt Server Route 生成（`apps/web/server/routes/`）。
- Legacy URL：旧版分页路由 `/p/:page` 经 Server Route `301` 跳转到 `/?page=N`（`apps/web/server/routes/p/[page].ts`）；完整 Redirect 表在 Phase 07 形成。
- 评论公开端（提交、Threaded 列表、分页）已随 Phase 05 完成并接入 Article Detail（`CommentSection.vue`）。
- Contract Test：`backend/server/tests/public_api.rs`、`public_articles.rs`、`comments_public_api.rs`。

剩余项：

- Desktop 与 Mobile 真机视觉及可访问性检查（Acceptance Gate 第 5 条），完成后本 Phase 转 ✅。
- 已知限制：View Count 去重为单机内存窗口，多实例部署需改用共享存储；生产环境需保证 Web 域下 `/api` 反代到 Backend。
