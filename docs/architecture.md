# Aries Next 系统架构

> 本文档回答「系统由哪些部分组成、为什么这样划分、权限和安全边界在哪里」，写给需要理解或修改整体结构的人。具体端点、字段和环境变量不在这里复制：API 细节查 `docs/openapi.yaml`，环境变量查 `.env.example`，数据表查 `migrations/`。
>
> 最后验证日期：2026-09-06（对照 `backend/server/src/lib.rs`、`backend/server/src/http/mod.rs`、`backend/server/src/http/logs.rs`、`backend/server/src/state.rs`、`backend/server/src/log_store.rs`、`backend/core/src/auth.rs` 核对）。

## 架构形态

Aries Next 是一个 Modular Monolith：一个 Rust API Process、一个 PostgreSQL Database、一个独立构建的 Vue 3 Admin SPA，以及一个 Nuxt 4 SSR Public Web。

选择单体而不是 Microservice 的原因很实际：博客系统的模块间耦合（文章引用分类、媒体、评论）远高于它们独立演进的需求，拆成分布式只会引入网络复杂度和分布式事务问题。模块边界通过代码结构和 Dependency Direction 维护；只有出现明确的独立扩容、故障隔离或部署周期需求时，才评估拆分。

## Dependency Direction

```text
server ------> core <------ infra
   |                         |
   +---- HTTP Adapter        +---- PostgreSQL / Storage / AI

migrator ----> MySQL（只读）+ PostgreSQL
admin/web ---> OpenAPI Contract ---> server
```

规则：

- `backend/core` 只定义 Domain Model 和 Repository / Service Contract（trait），不依赖 Axum、SQLx、PostgreSQL、Storage Provider 或 AI Vendor。这样业务规则可以在没有 HTTP 和数据库的情况下测试，外部依赖也可以整体替换实现。
- `backend/server` 负责 HTTP、Authentication、Validation、Middleware 和 DTO 转换；`AppState` 通过 `Arc<dyn ...>` 持有 core 的 trait 实现（见 `backend/server/src/state.rs`）。
- `backend/infra` 实现 core 定义的 Contract，例如 `PostgresContentRepository`、`Argon2PasswordHasher`、`ComrakMarkdownRenderer`、Local / S3 的 `MediaStorage`（`backend/infra/src/storage/`）以及 `AnthropicProvider` / `OpenAiCompatibleProvider` 等 AI Provider。
- HTTP Request DTO、Response DTO、Domain Object 和 Database Entity 必须分离；不直接把 Database Entity 返回给客户端。
- 前端以 `docs/openapi.yaml` 为唯一 API 契约来源，不手工复制接口类型。

## Backend Module

Backend 按业务能力划分模块：`auth`、`content`（含 `taxonomy`）、`engagement`、`media`、`appearance`、`settings`、`search`、`ai`。划分是逻辑上的，代码按实际聚合度组织——例如 Category / Tag 没有独立的 core 模块，而是作为 `content` 的一部分；搜索目前只有公开端点里的关键词查询，独立的 `search` 模块（含 Semantic Search）尚未实现。

当前 `aries-core` 的实际模块清单以 `backend/core/src/lib.rs` 为准：`auth`、`content`、`comments`、`media`（含 `SiteSettingsRepository`）、`pages`、`journals`、`galleries`、`links`、`navigation`、`settings`（分组设置）、`ai`、`ai_prompts`、`jobs`（Background Job）、`health`。HTTP 层路由按端点聚合，清单见 `backend/server/src/http/mod.rs`：管理端包含 `auth`、`articles`、`profile`、`taxonomy`、`media`、`site_settings`、`comments`、`dashboard`、`audit`、`pages`、`journals`、`galleries`、`links`、`navigation`、`settings_groups`、`ai`；匿名端点集中在 `http/public/`（`articles`、`taxonomy`、`site`、`extended`、`comments`）。

Module 之间通过 core 的显式 Contract 协作，不直接访问对方的数据表实现业务逻辑。每个 Module 的 Domain Rule（如密码策略、Role-Permission 矩阵、评论状态机）放在 core 并有 Unit Test。

## 运行时组件

一个 `aries-server` 进程内包含三部分：

- HTTP Server（`backend/server/src/lib.rs`）：Axum Router、Middleware、路由挂载。
- Background Worker（`backend/server/src/worker.rs`）：轮询 `background_jobs` 表执行异步任务，与 HTTP 共用 `AppState`。任务类型：`media_cleanup`（物理清除已软删除且零引用的媒体）、`metadata_probe`（媒体元数据探测）、`comment_notification`（评论通知，当前仅记录日志，Email Adapter 未接入）；历史遗留的 `import_markdown` 任务直接标记完成（导入改在 Commit 端点同步执行）。
- Logging（`backend/server/src/logging.rs` + `backend/server/src/log_store.rs`）：三路输出。控制台在开发期为 Pretty、生产为 JSON；文件始终为 JSON，写入 `LOG_DIR`（默认 `./logs`）下的 `aries-server.YYYY-MM-DD`，按天切分，超过 `LOG_RETENTION_DAYS`（默认 14 天）自动清理；同时经自定义 tracing Layer 批量落入 PostgreSQL `server_logs` 表（mpsc channel 缓冲，满时丢弃并计数；保留期与文件一致，到期物理 DELETE，无软删除；自身 INSERT 的 sqlx 事件被过滤以防递归）。内部例行操作（日志落库写入、Worker 每 5s 的空轮询领任务查询）包在 `quiet_internal` 标记 Span 内，三路输出层统一消音。SQL 语句日志开关：`LOG_SQL` 仅为启动初始值（未设置时非 production 默认 true、production 默认 false），运行期可经 `PUT /api/admin/logs/sql` 切换（tracing_subscriber reload），无需重启。慢查询由 `DATABASE_SLOW_QUERY_MS`（毫秒，默认 500，0 关闭）控制：超过阈值的 SQL 以 WARN 级、`sqlx::query` target 记录并自动落入 `server_logs`；生产建议保持全量 SQL 日志关闭、只开慢查询。

HTTP 请求管线由外到内：精确白名单 CORS（Origin 取自 `ADMIN_ORIGINS`，允许 Credentials）→ Request ID（`x-request-id` 生成与透传）→ tower_http TraceLayer 访问日志（Span 携带 method/path/query/`x-request-id`，响应记录 status 与 latency，5xx 记 error）→ `log_request_params` 中间件（非 GET/HEAD 的 JSON/Form 请求体脱敏后记录顶层参数，敏感键值替换为 `***`，超 8KB 或 multipart 只记跳过原因）。请求内的每条日志（含 SQL 日志）的 `fields` 会注入 `http.method`/`http.path`/`http.query`/`http.request_id`。同一请求的访问日志、业务日志和 SQL 日志共享同一 `x-request-id` 并写入 `server_logs.request_id`，可按它查询整条请求链路。`/api/admin` 子路由额外挂 `enforce_origin`（`backend/server/src/http/mod.rs`）：非 GET/HEAD/OPTIONS 请求的 `Origin` Header 必须精确匹配白名单，与精确 CORS 共同构成 CSRF 防护。

运行日志的可视化入口在 Admin「系统 → 运行日志」页（`/logs`）：级别（最低级别语义）、target、关键词、时间范围筛选（含 15 分钟/1 小时/24 小时/今天快捷选项），分页与 5 秒自动刷新，SQL 日志开关，24h 级别趋势迷你柱状图（数据来自 `GET /api/admin/logs/stats` 分桶统计），「隐藏 SQL 日志」开关（`exclude_target=sqlx::query`，与按 target 精确筛选互斥），以及两种互斥的上下文视图——点击 `request_id` 进入的请求链路视图和「查看前后上下文」（`around_id` 锚点模式）。查询 API（`GET /api/admin/logs`、`/api/admin/logs/stats`、`/api/admin/logs/targets`、`/api/admin/logs/sql`）仅 Owner（`ManageSettings`）可访问，契约见 `docs/openapi.yaml`。

## API Boundary

- `/api/public/*`：匿名只读为主（评论提交、访问密码解锁、浏览计数是例外），公开端点必须显式声明 `Cache-Control` 策略（见 `backend/server/src/http/public/mod.rs`）。
- `/api/admin/*`：需要 Authentication 和 Permission 的管理接口，包括 AI 能力。
- `/api/admin/ai/*`：AI 编辑器助手（SSE 流式）与用量查询。AI 端点故意挂在 `/api/admin` 下而不是独立的 `/api/ai`：Session Cookie 的 Path 限定 `/api/admin`，挂在外面浏览器不会带 Cookie；同时自动获得 `enforce_origin` 保护。AI 端点另有每用户限流，所有调用写入 `ai_requests` 审计表，且 AI 只产出建议，不自动发布、删除或审核通过内容。
- `/api/media/files/{*path}`：匿名媒体文件服务（Local Provider 时由本进程直接发文件）。
- `/api/health/*`：Liveness 与 Readiness Probe。

公开接口与管理接口使用不同的 Response DTO。公开接口不得返回 Password Hash、内部路径、未发布内容、访客 Email / IP / User-Agent 原文、Provider Secret 或 Audit Detail。端点细节以 `docs/openapi.yaml` 为唯一权威来源。

## Authentication 与 Authorization

Admin 使用 PostgreSQL Server Session，不向 Browser 暴露长期 JWT。登录成功后 Backend 生成高熵 Session Token，Browser 仅通过 `HttpOnly` Cookie 持有原始 Token，PostgreSQL 只保存其 SHA-256 Hash。

- Cookie 名称为 `aries_admin_session`，Path 限定 `/api/admin`，启用 `HttpOnly` 和 `SameSite=Lax`（`backend/server/src/http/auth.rs`）。
- Production 必须将 `SESSION_COOKIE_SECURE` 设为 `true` 并仅通过 HTTPS 访问 Admin。
- Login 会先撤销当前 Cookie 对应的旧 Session 再签发新 Session，避免 Session Fixation；修改密码、完成密码重置或禁用 User 时撤销该 User 的全部 Session。
- 登录、找回密码等敏感端点有内存滑动窗口限流（`backend/server/src/rate_limit.rs`）。

Role 固定为 `owner`、`editor`、`moderator`，Permission 是固定枚举（`backend/core/src/auth.rs` 的 `Permission`）。授权矩阵：

| Permission（前端标识） | Owner | Editor | Moderator |
| --- | --- | --- | --- |
| `dashboard:view`（`ViewDashboard`） | 是 | 是 | 是 |
| `content:manage`（`ManageContent`） | 是 | 是 | 否 |
| `comments:moderate`（`ModerateComments`） | 是 | 否 | 是 |
| `users:manage`（`ManageUsers`） | 是 | 否 | 否 |
| `settings:manage`（`ManageSettings`） | 是 | 否 | 否 |
| `profile:manage`（`ManageProfile`） | 是 | 是 | 是 |

矩阵写在 `Role::allows` 里并有 Unit Test 锁定。`users:manage` 权限已定义，但多用户管理的 API 和页面尚未实现，目前实际只有一个 Owner 加个人资料管理。Backend 的 Permission Guard 是授权依据；Admin 的路由 Guard 和 Sidebar 只做入口隐藏，不能替代 Backend 授权。

首个 Owner 通过一次性 Bootstrap 创建：`BOOTSTRAP_SECRET` 至少 24 个字符，Repository 使用 PostgreSQL Advisory Lock 保证并发请求最多成功一次（`backend/infra/src/auth.rs`）。

## Data Ownership

- PostgreSQL 是业务数据的 Source of Truth。
- Object Storage 保存媒体 Binary（Local 磁盘或 S3 Compatible），PostgreSQL 保存媒体 Metadata 和引用关系（`media_assets`、`media_usages`）。
- 站点级配置分两处：单例的 `site_settings`（站点名称、分页大小等强类型字段）和按组存储的 `setting_groups`（`appearance` / `email` / `integrations` / `ai`，JSON Payload）。Secret 字段（如 `smtp_password`、AI `api_key`）写库不读出，GET 只返回 `*_set: bool`。
- Search Index 和 Vector Index 属于 Derived Data（规划项，尚未实现），未来必须能从已发布内容重建。
- Redis 只在 Session、Rate Limit 或 Cache 出现实际需求后引入；当前限流与去重都是进程内内存实现。

## PostgreSQL Schema 与 Migration

运行配置通过 `DATABASE_SCHEMA` 指定业务 Schema。Backend 启动时先确保 Schema 存在，再设置 PostgreSQL `search_path`，最后执行 SQLx Migration（`backend/infra/src/lib.rs`）。Schema 名称只接受简单 Identifier（如 `public`、`aries`），服务端会校验。

所有结构变更只通过 `migrations/` 下的版本化 Migration，禁止启动时自动建表。表结构细节不在文档中复制，直接查对应 Migration 文件；测试通过随机 Schema 隔离运行（`backend/server/tests/common/mod.rs`）。

## Frontend Boundary

Admin 和 Public Web 不共享页面组件，只通过 `@aries/design-tokens` 共享 Brand Token：

- Admin（`apps/admin`）面向高频内容运营，强调信息密度、批量操作和稳定导航；基于 shadcn-vue，页面按文件路由组织。
- Public Web（`apps/web`）面向阅读、SEO 和内容发现，Nuxt SSR，每个页面维护 Title、Description 和 Canonical URL，并提供 `sitemap.xml`、`rss.xml`、`robots.txt`。

两端视觉不强制一致。

## Migration Policy

旧 MySQL 在开发阶段保持不变。数据迁移采用可重复执行的 ETL：

1. Read-only Preflight
2. Full Rehearsal
3. Count、Hash 和 Reference Validation
4. Write Freeze
5. Final Delta Migration
6. Production Cutover
7. Rollback Window

迁移前必须先写 `docs/database-mapping.md` 再写迁移代码。当前 `aries-migrator` 只实现了第 1 步（只读 Preflight 行数对比），后续步骤尚未实现。默认不使用长期 Dual Write；正式切换后旧 MySQL 保持只读至少一个观察周期。
