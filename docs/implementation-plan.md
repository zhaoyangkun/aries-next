# Aries Next 前后端实施规划

> 最后验证日期：2026-09-07
>
> 本文档回答：重写的总体技术方向、架构骨架和里程碑是什么。写给需要理解「为什么这么搭」的开发者。各 Phase 的范围、验收标准与实时状态见 [Phase 路线图](phases/README.md)；旧 Aries 功能盘点见 [博客功能地图](blog-feature-map.md)；API 细节以 [openapi.yaml](openapi.yaml) 为准，本文不复制端点清单。

## 目录

- [1. 当前状态](#1-当前状态)
- [2. 总体技术决策](#2-总体技术决策)
- [3. Backend 结构](#3-backend-结构)
- [4. API Contract](#4-api-contract)
- [5. Admin 信息架构与目录](#5-admin-信息架构与目录)
- [6. Admin 通用组件与页面优先级](#6-admin-通用组件与页面优先级)
- [7. Public Web 与 Design System](#7-public-web-与-design-system)
- [8. 里程碑](#8-里程碑)
- [9. 下一步执行顺序](#9-下一步执行顺序)

## 1. 当前状态

Phase 00–03、05、06 已完成，Phase 04 与 Phase 08 部分完成，Phase 07 进行中。权威进度表在 `docs/phases/README.md`，这里只做概述：

- Backend：PostgreSQL + SQLx Migration、统一错误响应、Session/Permission、文章发布主链路（状态机、Revision、Optimistic Lock）、Media Library（Local/S3）、评论、Dashboard、Audit、扩展内容（Pages/Journals/Galleries/Links/Navigation）、分组设置，以及第一批 AI 能力（编辑器助手 SSE、评论 AI 审核、用量审计）。
- Admin：Login/Bootstrap、Permission-aware Shell、文章编辑器（Vditor + Autosave + AI 助手）、媒体库、评论管理、扩展内容与设置页面均已接入真实 API。
- Public Web：Nuxt 4 SSR 页面（首页/文章/分类/标签/归档/搜索/友链/日志/图库/自定义页）、SEO（Canonical/Open Graph/JSON-LD）、`/sitemap.xml`、`/rss.xml`、`/robots.txt`、旧版 `/p/:page` 的 `301` 跳转、评论提交与展示。
- 未开始：Phase 07 的三轮正式 Rehearsal 与真实 Cutover（Migrator ETL、Validate、Runbook 已就绪）、Embedding/RAG/Semantic Search（Phase 08 第二批）、SMTP 邮件实际发送（Phase 05 遗留）、CI 管线。

后续开发顺序：Phase 07 迁移切换为主线；Phase 04 收尾（真机检查）与 Phase 08 第二批可并行。

## 2. 总体技术决策

### Backend

- 保持 Rust Modular Monolith，不拆分 Microservice。
- Axum 负责 HTTP，Tower 负责 Middleware；SQLx + 显式 SQL，不引入重量级 ORM。
- OpenAPI Contract 手工维护在 `docs/openapi.yaml`（唯一权威来源）；**不使用 Utoipa 生成**（与早期计划不同，见 `phases/phase-00-foundation.md` §3）。
- Admin Session 为自研实现：PostgreSQL 存 Token Hash + HttpOnly Cookie；不使用 `tower-sessions`，不在 Local Storage 保存长期 Token。
- Request 校验在 Handler/DTO 层手写，不引入 Garde。

### Admin

- Vue 3、TypeScript、Vite、Pinia、Tailwind CSS、shadcn-vue、Reka UI，`@lucide/vue` 图标；基于 shadcn-vue-admin 模板重建，页面走文件路由（`src/pages/`）。
- API 层用 axios 手写（`src/modules/**/api`、`src/shared/api`）；**不使用 `openapi-typescript` 生成 Client**（早期计划已裁剪）。
- Markdown 编辑器用 Vditor 保持旧内容兼容；单测用 Vitest。

### Public Web

- Nuxt 4 SSR；Tailwind CSS；与 Admin 只共享 `@aries/design-tokens` 的 Brand Token，不共享业务 UI Component。
- 页面数据用 `useAsyncData` 做 SSR Fetch；保证 Canonical URL、Sitemap、RSS、Open Graph 和 Structured Data。

## 3. Backend 结构

业务按能力划分模块，放在 4 个 Crate 内，不为每个业务创建独立 Crate。实际结构（以代码为准）：

```text
backend/
├── core/src/      # Domain Model 与 Repository/Service trait（auth、content、comments、media、settings、ai 等）
├── infra/src/     # PostgreSQL Repository、storage/（local + s3）、markdown.rs、password.rs、ai.rs
├── server/src/
│   ├── config.rs  # 环境变量与校验
│   ├── http/      # 路由与 DTO：admin 模块平铺，匿名端点在 http/public/，统一错误在 error.rs
│   ├── security.rs / rate_limit.rs / logging.rs / state.rs / worker.rs
│   └── main.rs
└── migrator/src/  # MySQL → PostgreSQL ETL 工具（preflight / migrate / validate 子命令）
```

依赖方向严格遵守 `server → core ← infra`；`aries-core` 不依赖 Axum、SQLx 或具体 AI Vendor。Dependency 按功能逐步增加，不一次性安装。Redis 不作为第一阶段依赖：Session、Rate Limit 和 Cache 在单实例阶段用 PostgreSQL 或进程内实现。

## 4. API Contract

路径按访问边界划分：`/api/public/*`（匿名、显式 Cache-Control）、`/api/admin/*`（需 Authentication + Origin 校验）、`/api/health/*`。端点清单以 `docs/openapi.yaml` 为准。

与早期规划的一处重要偏差：AI 端点实际挂在 `/api/admin/ai/*` 而非 `/api/ai/*`——Session Cookie 的 Path 限定 `/api/admin`，挂在 Admin 边界下浏览器才会携带 Cookie，同时自动获得 Origin 校验。AI 编辑器端点使用 SSE 流式返回（`start` → `delta` → `usage` → `done` / `error`），所有 AI 请求写入 `ai_requests` 审计。

## 5. Admin 信息架构与目录

Main Navigation 按「概览 / 内容 / 互动 / 媒体 / 外观 / 系统」分组；已实现的页面与 `apps/admin/src/pages/` 一一对应（articles、categories、tags、pages、journals、galleries、links、navigation、media、comments、dashboard、audit-logs、settings、auth）。AI 入口目前在文章编辑器内（选中文本 Rewrite / Summary / Metadata）和设置页 AI 分组，独立的「AI 写作工作台 / 用量审计」导航项待 Phase 08 后续批次。

Admin 目录遵循 shadcn-vue-admin 模板约定：

```text
apps/admin/src/
├── pages/        # 文件路由，<route lang="yaml"> 配 meta
├── layouts/      # AdminLayout 等
├── modules/      # 业务模块（api/、components/、composables/），API 层显式 import
├── shared/       # api（axios）、components、composables、utils
├── stores/       # Pinia
└── components/   # shadcn-vue 生成组件与通用组件（自动导入）
```

Admin Shell 具备：可折叠 Sidebar 与 Mobile Drawer、Breadcrumb、User Menu/Logout、Route Loading、Permission-aware Navigation、Full Page Error/Empty/Forbidden/Offline 状态。Global Search / Command Palette 与 Notification Center 未实现。不默认加入多页签缓存。

## 6. Admin 通用组件与页面优先级

通用组件只表达 UI Pattern，不包含业务 API。已有：`AppPageHeader`、`AppEmptyState`、`AppConfirmDialog`、`AppMediaPicker`、`MarkdownEditor`（Vditor Wrapper + AI Action）与 shadcn-vue 基础组件。待补：`AppDataTable`/`AppFilterBar` 的统一抽象（目前各列表页自行组合）、`AppStatusBadge`、`AppErrorState` 的统一封装。

页面优先级（P0 → P2）与实际状态：

- **P0 可登录、可发布**（✅ 已完成）：Login、Session 恢复、Admin Shell、Dashboard、Article List/Editor、Category/Tag Selector、Media Upload 与 Cover Picker、Draft/Publish/Recycle。
- **P1 日常运营**（✅ 已完成）：Comment Moderation、Page/Journal/Gallery、Media Library、Navigation 与 Link、Site Setting、User Profile 与 Password。
- **P2 内容效率**（🚧 部分完成）：AI Rewrite/Summary/Metadata ✅；Audit Log ✅；Autosave 与 Draft Recovery ✅；Batch Action、Command Palette、Background Job Monitor 未做。

## 7. Public Web 与 Design System

第一阶段页面已全部落地（见 Phase 04 交付记录）：Home Feed、Article Detail、Category/Tag/Archive、Search、Custom Page、Link/Journal/Gallery、RSS/Sitemap/Robots、`404`/`500` 与 Article Password Challenge。SEO 要求不变：每页唯一 Title/Description、Article Canonical + Open Graph + JSON-LD `BlogPosting`、Pagination Canonical、Legacy URL `301` Redirect。

Design System 方向（历史决策，保留）：Admin 定位为安静、高密度的 CMS Tool——Cool Neutral Background、White Surface、Blue Primary Accent（延续旧 D2 Admin 色调）；状态色使用不同 Hue；Card Radius ≤ `8px`；Page Title `24px`–`28px`；Data Table/Toolbar/Pagination 高度稳定；Icon 统一 Lucide，不熟悉的 Icon 必须给 Tooltip；Mobile 重点支持审核、搜索和简单编辑。

## 8. 里程碑

| Milestone | 范围 | 状态 |
| --- | --- | --- |
| M0 Foundation | Error、Config、Contract、Test Harness、Design Token | ✅ |
| M1 Auth | Session、Login、Permission、Admin Shell | ✅ |
| M2 Content | Article/Category/Tag API、Editor、Taxonomy | ✅ |
| M3 Media | Upload、Storage、Metadata、Media Picker、Import | ✅ |
| M4 Operations | Comment、Dashboard、Audit、Setting | ✅（SMTP 发送未接入） |
| M5 Public Web | Public API、Nuxt SSR、SEO、Redirect | 🚧（余真机检查） |
| M5b Extended | Page/Journal/Gallery/Link/Navigation/Appearance | ✅ |
| M6 Migration | Full ETL、Validation、Report、Cutover Rehearsal | 🚧（ETL/Validation/Report ✅；Rehearsal 与 Cutover ⬜） |
| M7 AI | Editor AI、评论审核、用量审计 | 🚧（第一批 ✅；Embedding/RAG ⬜） |

## 9. 下一步执行顺序

1. **Phase 07 迁移切换（主线）**：Migrator ETL（Extract → Transform → Load → Validate → Report）与 Legacy bcrypt 密码兼容已完成并经一次真实数据演练，Runbook 见 `docs/migration-runbook.md`；剩余为 Sanitized Sample、Full Snapshot、Cutover 三轮正式 Rehearsal 与真实 Cutover（含 Rollback 演练）。
2. **Phase 04 收尾**：Desktop 与 Mobile 真机视觉及可访问性检查，完成后转 ✅。
3. **可并行**：Phase 05 遗留的 SMTP Email Adapter（`email_deliveries` 闭环）；Phase 08 第二批（Embedding、`content_embeddings`、RAG/Semantic Search、Evaluation Set）。

AI 深度能力（Embedding/RAG）不阻塞 Phase 07；首次替换旧站的 Release Gate 不包含 AI，见 `phases/README.md` §5。
