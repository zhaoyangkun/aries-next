# 展示端浮动工具栏与体验增强设计

> 日期：2026-10-02 ｜ 状态：已确认（用户已批准方案）

## 背景与目标

博客展示端（`apps/web`，Nuxt 4 SSR）基础能力已齐（TOC 侧栏、阅读进度条、代码高亮、深色模式、双 Tab 搜索、相关阅读、归档时间线、RSS/Sitemap、图库灯箱），但缺少流行主题（Butterfly / NexT / Halo / 旧版 xue）常见的右下角工具栏与一批体验细节，整体观感偏简陋。

目标：新增右下角浮动工具栏（回到顶部为主），并落地一轮功能与 UI 增强，全部保持 SSR/SEO 安全。

## 用户已确认的决策

- 工具栏形态：**进度环按钮 + 悬停展开**（非 xue 月亮扇形、非仅回顶）
- 核心范围（全做）：移动端目录抽屉、文章图片灯箱、Ctrl/⌘+K 全局搜索弹层、阅读时长/字数统计
- 加分项（做）：标签云动画、全量中文 WebFont、站点运行时长
- 明确**不做**：打赏二维码、评论增强、管理端改动、后端检索能力改动

## 方案

### 1. 浮动工具栏 `AppToolbar.vue`（新组件）

- 位置 `fixed bottom-6 right-6 z-40`，主按钮 44px 圆形：
  - SVG 圆环（`stroke-dasharray`）显示滚动进度 `scrollTop / (scrollHeight - clientHeight)`
  - 滚动超过 300px 浮现（opacity/translate 过渡），点击 `window.scrollTo({ top: 0, behavior: 'smooth' })`
- 悬停（桌面，`@media (hover: hover)`）或点击（移动端 toggle）展开竖排子项：
  - 回到底部（有滚动空间时显示）
  - 目录：仅文章页且 `xl` 以下视口显示，唤出 TOC 抽屉
  - 搜索：打开 ⌘K 弹层
- 深色模式切换不放（顶栏已有）。

### 2. 移动端目录抽屉 `TocDrawer.vue` + `useArticleToc` composable

- 把 `ArticleToc.vue` 中「客户端从渲染正文提取 h2/h3、补 `toc-${i}` id、IntersectionObserver 高亮当前章节」抽为 `app/composables/useArticleToc.ts`
- `ArticleToc`（桌面 xl 侧栏）与 `TocDrawer`（移动端 bottom sheet：圆角上滑、`max-height: 70vh`、章节点击跳转后自动关闭）共用该 composable

### 3. 文章图片灯箱

- 扩展 `AppImageLightbox.vue` 支持通用图片列表模式（图库现有用法兼容）
- `HighlightedContent.vue` 渲染后给 `.article-content img` 挂点击放大（`cursor: zoom-in`），多图可左右切换

### 4. 全局搜索弹层 `SearchPalette.vue`

- 任意页面 `Ctrl/⌘ + K` 打开居中弹层（`/` 不绑定，避免与输入框冲突）；Esc 关闭；焦点自动落入输入框
- 输入复用现有搜索建议接口（与 `AppSearchBox` 同源）
- 回车 → `/search?q=`（现有页面含关键词高亮与问 AI Tab）；`⌘/Ctrl + Enter` → `/search?q=&tab=ai`
- 键盘/滚动事件全部 `onMounted` 注册，SSR 安全；打开时锁 body 滚动

### 5. 阅读时长/字数统计

- `app/utils/readingTime.ts`：`readingTime(markdown)` 中文按字符、英文按词估算，返回 `{ minutes, words }`
- 文章详情页 meta 行显示「约 X 分钟 · Y 字」；SSR 用 `markdown_source` 直接计算，无 hydration 差异

### 6. 标签云动画

- `tags/index.vue` 从 chips 升级为权重云：字号按文章数对数缩放，入场交错动画用 Vue `<TransitionGroup>` + CSS，不引第三方库

### 7. 全量中文 WebFont

- `apps/web/public/fonts/` 引入 Noto Sans SC 中文 woff2 分片（unicode-range、font-display: swap），`main.css` 的 `--font-sans` 声明补上中文源
- 浏览器按页面实际用字按需下载分片；preload  latin 关键片

### 8. 站点运行时长（唯一后端变更）

- `GET /api/public/site` 响应新增 `created_at`（取 `site_settings` 行创建时间，`OffsetDateTime`，ISO 8601）
- 前端 footer 显示「本站已运行 X 天」，从 `created_at` 计算
- 同步更新 `docs/openapi.yaml` 并重新生成 `@aries/api-client`（向后兼容的纯新增字段）

## 关键文件

| 动作 | 文件 |
|------|------|
| 新增 | `apps/web/app/components/AppToolbar.vue`、`TocDrawer.vue`、`SearchPalette.vue` |
| 新增 | `apps/web/app/composables/useArticleToc.ts`、`apps/web/app/utils/readingTime.ts` |
| 修改 | `apps/web/app/layouts/default.vue`（挂工具栏、⌘K、footer 运行时长） |
| 修改 | `apps/web/app/components/ArticleToc.vue`（改用 composable）、`HighlightedContent.vue`（图片灯箱）、`AppImageLightbox.vue`（通用模式）、`tags/index.vue`（标签云） |
| 修改 | `apps/web/app/pages/articles/[slug].vue`（meta 行阅读时长、目录抽屉、工具栏目录项数据源） |
| 修改 | `apps/web/app/assets/css/main.css`（字体声明） |
| 修改 | `backend/server/src/http/public/site.rs`（`created_at`）、`docs/openapi.yaml`、`packages/api-client`（重新生成） |

## 测试策略

- 新增 vitest：readingTime（中英文混合、边界）、useArticleToc（提取/嵌套/去重）、搜索弹层快捷键逻辑
- 后端：public site API contract test 断言 `created_at` 存在
- 门禁：`pnpm typecheck`、`pnpm test`、`pnpm build`、`cargo fmt/clippy/test`（backend 单字段变更）
- 人工验证清单：桌面/移动端回顶与进度环、抽屉目录跳转、正文图片放大切换、⌘K 弹层交互、暗色下各新组件观感、字体加载（Network 面板分片按需）

## 风险与回滚

- 中文字体：若分片加载影响首屏，可回退为仅 latin 子集（CSS 一处声明）
- 灯箱扩展需保证图库现有调用不回归（props 兼容）
- API 纯新增字段，旧前端忽略即可，回滚 = revert 前端组件 + 后端字段
