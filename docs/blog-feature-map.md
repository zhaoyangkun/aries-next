# Aries 博客功能地图

> 本文档回答「旧版 Aries 有哪些功能、Aries Next 如何继承、当前做到哪一步」，写给需要判断重写范围与进度的人。状态标记：✅ 已完成、🚧 部分完成、⬜ 未开始。实施顺序与验收标准见 `docs/phases/README.md`。
>
> 最后验证日期：2026-09-06（对照 `backend/server/src/http/`、`apps/admin/src/pages/`、`apps/web/app/pages/` 与 `migrations/` 核对）。

## 1. 旧版基线

旧 Aries 的特征（继承策略的出发点）：

- Backend 使用 Gin、GORM 和 MySQL。
- Admin 使用 Vue 2、D2 Admin 和 Element UI。
- Public Web 使用 Go Template，通过主题目录切换页面模板。
- Admin API 使用 JWT，公开评论接口与部分读取接口缺少完整访问边界。
- 数据关系主要依赖 Application 维护，Database 缺少 Foreign Key。

## 2. 当前进度概览

- Backend：Bootstrap、Session、密码重置 Token 流程、文章发布与修订、媒体库（Local / S3）、Markdown 导入、评论与 Dashboard、Audit Log、页面 / 日志 / 图库 / 友链 / 导航、分组设置（appearance / email / integrations / ai）、AI 编辑器助手与评论 AI 风险标记均已实现；公开端点覆盖文章、分类、标签、归档、关键词搜索、站点信息及扩展内容。
- Admin：文章、页面、日志、图库、友链、导航、分类、标签、媒体库、评论、审计日志、各组设置页面均已接入真实 API；文章编辑保留 Dialog 形态并内置 AI 助手。
- Public Web：已接入 `/api/public`，页面覆盖首页、文章详情（含评论与访问密码）、分类、标签、归档、搜索、日志、图库、友链、自定义页面，并提供 `sitemap.xml`、`rss.xml`、`robots.txt`。
- 尚未实现：邮件发送（SMTP 配置可存，投递未接入）、Captcha、多用户管理、Twikoo Adapter、Semantic Search / RAG、社交链接、旧图床 Provider 重写、完整 Legacy 301 映射表、正式迁移 ETL（`aries-migrator` 当前只有只读 Preflight）。

## 3. 旧博客功能清单

| 领域 | 旧版能力 | Aries Next 处理策略 | 状态 |
| --- | --- | --- | --- |
| Authentication | 初始化注册、Captcha 登录、JWT、忘记密码、邮件重置 | 改为一次性 Bootstrap + Server Session；Captcha 按风险启用（未实现）；Reset Token 已生成但邮件投递未接入 | 🚧 |
| 用户 | 个人资料、头像、签名、修改密码 | 保留；已增加 Role、Session 和 Audit；多用户管理（`users:manage`）权限已定义，API 与页面未实现 | 🚧 |
| 文章 | CRUD、分页、关键词与状态筛选 | 保留；稳定 Pagination、统一状态机，另有 Markdown Preview 与 Revision 历史 | ✅ |
| 发布状态 | 草稿、发布、回收站 | 统一为 `draft`、`published`、`recycled`（`backend/core/src/content.rs`） | ✅ |
| 文章属性 | 置顶、排序、自定义 URL、封面、摘要、SEO 关键词 | 保留；自定义 URL 统一为 `slug`，排序使用显式 `sort_order` | ✅ |
| 文章安全 | 访问密码、是否允许评论 | 保留；只保存 Password Hash，公开访问经 `/api/public/articles/{slug}/access` 换取短期授权 | ✅ |
| Markdown | Vditor、Markdown Source、Rendered HTML | 保留；Admin 仍用 Vditor，Backend 用 Comrak 统一 Render 与 Sanitize，前端只提交 Source | ✅ |
| 文件导入 | 从 Markdown 文件批量导入文章 | 保留为可预览的 Import Job（`/api/admin/articles/imports`），确认后才发布 | ✅ |
| 分类 | 文章、友链、图库三种分类，文章分类支持父子层级 | 同一物理表按 `kind` 隔离三套体系，`(kind, slug)` 唯一；文章分类保留父子层级 | ✅ |
| 标签 | CRUD、文章多标签、文章数统计 | 保留；Count 由查询得出，不手工维护 | ✅ |
| 评论 | 嵌套回复、审核、回收站、Markdown、邮件通知 | Built-in Comment 已实现：嵌套、`pending`/`approved`/`rejected`/`spam`/`recycled` 状态机、Backend 渲染、提交限流与重复提交防护、AI 风险标记（仅标记不自动处置）；通知已走 `comment_notification` Background Job，但仅记录日志，邮件投递未接入 | 🚧 |
| Twikoo | Built-in Comment 与 Twikoo 切换 | 作为可选 Adapter 延后，不阻塞核心博客上线 | ⬜ |
| 页面 | 自定义页面及 Markdown 内容 | 保留；统一 Content Contract，公开 API 为 `/api/public/pages/{slug}`，Web 路由沿用旧版 `/custom/{slug}` | ✅ |
| 日志 | 短内容、公开或私密 | 保留为独立 Content Type，不并入 Article | ✅ |
| 图库 | 分类、图片、名称、描述、地点 | 保留；Gallery Item 引用 Media Asset，另有公开 `/api/public/photos` 聚合 | ✅ |
| 友情链接 | 分类、站点名称、URL、描述、图标 | 保留；分类复用 `kind = link` 的分类体系 | ✅ |
| 导航 | 两级菜单、排序、打开方式、图标 | 保留；目标可引用内部 Content 或外部 URL，支持显式排序 | ✅ |
| 媒体 | 多图上传、查询、删除、存储类型 | 重构为 Media Library + Storage Adapter + 引用检查（`media_usages`），支持批量删除与远程抓取 | ✅ |
| 图床 | 本地、sm.ms、imgbb、Tencent COS 等 | 已支持 Local 与 S3 Compatible（`backend/infra/src/storage/`）；旧 URL 原样迁移；专用 Provider 按需增加 | 🚧 |
| 主题 | 两套 Go Template 主题、主题切换 | 不迁移可执行 Template；Public Web 为 Nuxt 固定组件，外观通过 `appearance` 设置组与 `@aries/design-tokens` 管理 | 🚧 |
| 网站设置 | 名称、描述、URL、Logo、备案、静态根路径、Head/Footer | 安全字段入 `site_settings`；任意 Head/Footer 注入默认不迁移，改为 `integrations` 设置组的受控配置 | ✅ |
| 邮件设置 | SMTP、测试邮件 | SMTP 配置入 `email` 设置组（`smtp_password` 写库不读出）；Email Adapter 与测试邮件未实现 | 🚧 |
| 参数设置 | 首页、归档、Sitemap 分页大小 | 保留为 `site_settings` 里的强类型字段（`page_size_index` / `page_size_archive` / `page_size_search`） | ✅ |
| 社交信息 | QQ、微信、GitHub、微博、知乎 | 规划为可排序的 Social Link 列表，尚未实现 | ⬜ |
| Dashboard | 文章数、评论数、最新内容 | 保留；数据来自聚合 API `/api/admin/dashboard`，不由前端拼接 | ✅ |
| Public Web | 首页、文章详情、分类、标签、归档、搜索 | 全部保留并改为 Nuxt SSR | ✅ |
| 扩展页面 | 友链、日志、图库、自定义页 | 全部保留 | ✅ |
| SEO | Go Template SSR、Sitemap | 已扩展为 SSR + Canonical + Open Graph + `sitemap.xml` + `rss.xml` + `robots.txt`；Structured Data 与完整 Redirect 表未完成 | 🚧 |
| 统计 | Visit Count、Comment Count | 保留；Visit 经 `/api/public/articles/{slug}/views` 上报，按客户端指纹去重，不接受客户端写入 Count | ✅ |
| API 文档 | Swagger | 改为手工维护的 OpenAPI 契约 `docs/openapi.yaml`（唯一权威来源，不引入 Utoipa 生成）；Production 不提供 Swagger UI | ✅ |
| 前端日志 | D2 Admin 本地日志页面 | 不原样迁移；使用统一 Error Boundary（`error.vue`）与后端 Tracing，可选 Error Provider 未接 | 🚧 |

## 4. 公开站页面地图

以下为 `apps/web/app/pages/` 的实际路由：

| 页面 | Route | 核心能力 |
| --- | --- | --- |
| 首页 | `/` | Published Article Feed、置顶、Pagination |
| 文章详情 | `/articles/{slug}` | Markdown、目录、标签、评论、访问密码解锁 |
| 分类索引 | `/categories` | 分类树与文章数量 |
| 分类文章 | `/categories/{slug}` | Category Filter、Pagination |
| 标签索引 | `/tags` | Tag List |
| 标签文章 | `/tags/{slug}` | Tag Filter、Pagination |
| 归档 | `/archives` | 按 Year/Month 聚合文章 |
| 搜索 | `/search?q=` | Keyword Search；Semantic Mode 待 AI 检索上线 |
| 自定义页面 | `/custom/{slug}` | 沿用旧版 xue 主题路由，数据源为 `/api/public/pages/{slug}` |
| 关于 | `/about` | 静态内容页 |
| 日志 | `/journals` | 公开短内容 Timeline |
| 图库 | `/galleries`、`/galleries/{slug}` | Gallery Category 与 Media Asset |
| 友情链接 | `/links` | Link Category 与站点信息 |
| Sitemap | `/sitemap.xml` | Published Content URL（`apps/web/server/routes/`） |
| RSS | `/rss.xml` | 最新 Published Article |
| Robots | `/robots.txt` | Crawler Policy 与 Sitemap 地址 |
| 旧版分页 | `/p/{page}` | 301 跳转到 `/?page=N`（唯一的 Legacy Redirect，完整映射表待 Phase 07） |

Legacy Route 必须在 Phase 07 形成完整 `301 Redirect` 表，不能仅依赖前端跳转；当前只有 `/p/{page}` 一条。

## 5. Admin 信息架构

以下为 `apps/admin/src/constants/sidebar-data.ts` 的实际结构（每项带 Permission 标识，无权限的入口直接隐藏）：

```text
工作台
└── 概览  /dashboard

内容
├── 文章  /articles
├── 页面  /pages
├── 日志  /journals
├── 图库  /galleries
├── 友情链接  /links
├── 导航菜单  /navigation
├── 分类  /categories
├── 标签  /tags
├── 媒体库  /media
└── 评论  /comments

系统
├── 审计日志  /audit-logs
└── 设置
    ├── 个人资料  /settings
    ├── 外观  /settings/appearance
    ├── 站点设置  /settings/site
    ├── 邮件设置  /settings/email
    ├── 第三方集成  /settings/integrations
    └── AI 设置  /settings/ai
```

与原规划的差异：AI 没有独立的 Sidebar 分组——写作助手在文章编辑 Dialog 内（`ArticleAiAssistDialog.vue`），评论 AI 审核结论展示在评论列表，配置在「AI 设置」，用量数据经 `/api/admin/ai/usage` 提供；「用户与权限」和「Background Job」页面尚未实现。文章创建和编辑继续使用 Dialog，Media Library、Setting 和 Audit 使用独立 Route。

## 6. 核心业务规则

1. AI、Import 和 Autosave 只能产生 Draft，不得自动发布。
2. Published Article 必须有唯一 `slug`、`published_at` 和可渲染正文。
3. Rendered HTML 只由 Backend 生成并执行 Sanitization。
4. Category、Tag、Media 等被引用资源删除前必须进行引用检查。
5. Comment 默认进入 `pending` 或按站点策略审核，客户端不能直接指定审核状态；AI 审核结论仅作标记，不自动通过或删除。
6. Visit Count、Comment Count 和 Tag Count 不接受客户端提交。
7. 所有管理写操作必须记录 Operator、Action、Target 和 Timestamp（`audit_logs`）。
8. Secret 不得通过普通 Setting API 明文返回（`smtp_password`、AI `api_key` 等只回 `*_set: bool`）。
9. Public API 只返回 Published、可见且未删除的数据。
10. Legacy 数据迁移失败时必须输出 Report，不得静默丢弃。

## 7. 不直接继承的旧实现

- 不继续使用 JWT 存储于 Browser Storage，改用 HttpOnly Session Cookie。
- 不允许 Public Registration 创建管理员，初始化使用一次性 Bootstrap 流程。
- 不运行从旧站迁移来的 Go Template、JavaScript 或任意 Head/Footer HTML。
- 不将旧 GORM Entity 直接映射为 API Response。
- 不继续依赖 Application 手工维护所有关联，PostgreSQL 使用 Foreign Key 与 Constraint。
- 不为每个旧图床 Provider 立即重写 SDK，先建立统一 Storage Adapter。
- 不把 AI 接在 Mock Editor 或不稳定 Content API 上。

## 8. 追踪原则

- 每个功能只能归属一个主要 Phase，后续 Phase 可以扩展但不得重复定义基础 Contract。
- Phase 完成以 Acceptance Gate 为准，不以页面「看起来完成」为准。
- 发生范围变化时，先更新对应 Phase 文档，再修改 Migration、API 或 UI。
- `docs/implementation-plan.md` 保留架构总览；`docs/phases` 负责可交付的执行计划。
