# AGENTS.md

本文件面向 AI Coding Agent，假设读者对本项目一无所知。所有信息均来自仓库实际内容，不包含推测。工程规范细则见 `PROJECT_RULES.md`（面向人），本文只保留导航信息与红线规则，避免两份文件重复。

## 项目概述

Aries Next 是博客系统 Aries 的新一代重写实现，采用 Modular Monolith 架构：一个 Rust API Process、一个 PostgreSQL Database、一个独立构建的 Vue 3 Admin SPA、一个 Nuxt 4 SSR Public Web。旧版 Aries（MySQL）在正式切换前保持可运行，作为功能行为、数据迁移和 URL 兼容性的基准。

技术栈：

- Backend：Rust（edition 2024，最低 1.85）、Axum 0.8、Tokio、SQLx 0.8、Tower
- Database：PostgreSQL 17（或兼容版本），SQLx Migration 管理 Schema
- Admin：Vue 3、TypeScript、Vite 8、vue-router 5（文件路由）+ vite-plugin-vue-layouts、Pinia 4、Tailwind CSS 4、shadcn-vue（基于 shadcn-vue-admin 模板重建）、Reka UI、`@lucide/vue` 图标
- Public Web：Nuxt 4（4.5.1）、Vue 3、SSR、Tailwind CSS 4
- Migration：Rust 工具，MySQL 只读源 + PostgreSQL 目标
- AI：Provider Adapter、SSE 流式输出（编辑器助手改写/摘要/元数据/标签推荐/导读、评论 AI 审核，见 `backend/server/src/http/ai.rs`）；Embedding / RAG（相关文章 + 对话式搜索）已实现，默认关闭，后台开启 `smart_search` 并配置 Embedding 端点后启用（见 `docs/blog-feature-map.md` §9）

项目优先级：数据安全、可回滚迁移、SEO、可测试性、长期可维护性。不提前拆分 Microservice。

## 目录结构

```text
backend/server     # aries-server：Axum HTTP Server、Router、Middleware、Config
backend/core       # aries-core：Domain Model、Repository/Service Contract（trait）
backend/infra      # aries-infra：PostgreSQL Repository、Storage（local/S3）、Password Hash、Markdown 渲染、AI Adapter
backend/migrator   # aries-migrator：MySQL → PostgreSQL ETL 工具（preflight / migrate / validate 子命令）
apps/admin         # @aries/admin：Vue 3 管理端 SPA（页面在 src/pages/ 下按文件路由生成；本包特有约定见 apps/admin/AGENTS.md）
apps/web           # @aries/web：Nuxt 4 SSR 公开站
packages/design-tokens  # @aries/design-tokens：两端共享的 Brand Token（仅导出 theme.css）
packages/api-client     # @aries/api-client：由 docs/openapi.yaml（utoipa 注解生成的机器生成物）派生的共享类型（openapi-typescript，`generate` 脚本可重新生成；admin/web 从这里派生 DTO 类型）
migrations         # PostgreSQL 版本化 SQLx Migration（如 202608010001_initial_content.sql）
deploy             # Docker Compose 生产部署（Caddy 反代 + 多阶段构建；deploy/scripts/ 提供 build-admin/deploy/backup 自动化脚本，可选）
docs               # 架构、ADR、数据映射、Phase 规划、OpenAPI Contract（openapi.yaml）
tests              # 跨进程 Contract Test 与 E2E Test 目录（当前为空，为规划预留）
```

### Dependency Direction（严格遵守）

```text
server ------> core <------ infra
migrator ----> MySQL + PostgreSQL
migrator ----> infra
admin/web ---> OpenAPI Contract ---> server
```

- `aries-core` 不得依赖 Axum、SQLx、PostgreSQL 或具体 AI Vendor，只定义 Domain Model 和 `AuthRepository`、`ContentRepository`、`PasswordHasher`、`MarkdownRenderer` 等 trait。
- `aries-migrator` 复用 `aries-infra` 的 `ComrakMarkdownRenderer` 与 local storage 的 object_key 约定（Phase 07 起批准的依赖方向），保证迁移渲染/Sanitization 与线上一致。
- `aries-server` 负责 HTTP、Authentication、Validation、Middleware 和 DTO 转换；`AppState` 通过 `Arc<dyn ...>` 持有 core 的 trait 实现（见 `backend/server/src/state.rs`）。
- `aries-infra` 实现 core 定义的 Contract（`PostgresAuthRepository`、`PostgresContentRepository`、`Argon2PasswordHasher`、`ComrakMarkdownRenderer`）。
- HTTP Request DTO、Response DTO、Domain Object 和 Database Entity 必须分离；不直接返回 Database Entity。

`backend/server/src/http/` 下已实现的管理端路由模块：`auth`、`articles`、`taxonomy`、`profile`、`media`（媒体库与 Markdown 导入）、`site_settings`、`comments`、`dashboard`、`audit`、`pages`、`journals`、`galleries`（含条目与图库分类）、`links`（含友链分类）、`navigation`、`settings_groups`（appearance / email / integrations / ai 分组设置）、`ai`、`logs`（运行日志查询、SQL 日志开关、运行时级别覆盖与 SSE 实时 tail）；`public/` 下为匿名端点（文章列表/详情/解锁/浏览计数与评论、分类、标签、归档、搜索、站点信息，以及 `public/extended.rs` 的页面、日志、图库、友链、导航）。`/api/media/files/*path` 提供匿名媒体文件服务；`backend/server/src/worker.rs` 承载后台任务轮询（`media_cleanup` / `metadata_probe` / `comment_notification`（当前仅记录日志，Email Adapter 未接入）；历史遗留的 `import_markdown` 直接标 Done），单任务 panic 被 catch_unwind 隔离为失败、不杀死轮询循环；另每天物理清理一次 `admin_sessions` 中已过期或已撤销的 Session（`AuthRepository::delete_expired_sessions`）。

## 环境要求与本地启动

- Rust stable ≥ 1.85（CI 固定 latest stable；本地开发保持相近版本即可）、Node.js 24+、pnpm 11+（`packageManager: pnpm@11.9.0`）、PostgreSQL 17（自备实例；可选使用 `deploy/` 下的 Docker Compose 编排）。

```powershell
# 1. 配置环境变量（系统环境变量优先于 .env）
Copy-Item .env.example .env

# 2. 准备 PostgreSQL：使用 .env 中 DATABASE_* 指向的既有实例（默认本机 5433）；也可选使用 deploy/ 下的 Docker Compose（见 docs/docker-deployment.md）

# 3. 启动 Backend（启动时自动创建 DATABASE_SCHEMA 指定的 Schema 并执行 SQLx Migration）
cargo run -p aries-server

# 4. 启动前端（首次先 pnpm install）
pnpm dev:admin   # http://127.0.0.1:5173，Vite 将 /api 代理到 http://localhost:8088
pnpm dev:web     # http://127.0.0.1:3000
```

Backend 监听 `SERVER_ADDR`（默认 `0.0.0.0:8088`），未注册根路径 `/`，访问返回 404。健康检查：`/api/health/live`、`/api/health/ready`。

关键环境变量（完整默认值见 `.env.example`）：`APP_ENV`、`SERVER_ADDR`、`DATABASE_HOST/PORT/USERNAME/PASSWORD/NAME/SCHEMA`（全部未设置时回退读取 `DATABASE_URL`）、`DATABASE_SLOW_QUERY_MS`（SQL 慢查询阈值，毫秒，默认 500，0 关闭；超时以 WARN 记入运行日志）、`BOOTSTRAP_SECRET`（必填，≥ 24 字符，用于一次性创建首个 Owner）、`ADMIN_ORIGINS`（逗号分隔的精确 Origin 白名单）、`SESSION_TTL_HOURS`（1–720）、`SESSION_COOKIE_SECURE`（production 默认 true）、`MEDIA_PROVIDER`（local 或 s3）、`MEDIA_LOCAL_DIR`、`MEDIA_PUBLIC_BASE_URL`、`S3_ENDPOINT/BUCKET/ACCESS_KEY/SECRET_KEY/REGION/PUBLIC_BASE_URL`、`MIGRATION_MYSQL_URL`、`MIGRATION_POSTGRES_URL`、`RUST_LOG`、`LOG_DIR`（日志文件目录，默认 `./logs`）、`LOG_RETENTION_DAYS`（日志保留天数，默认 14）、`LOG_SQL`（SQL 语句日志开关，仅启动初始值：未设置时非 production 默认 true、production 默认 false，运行期可在 Admin「系统 → 运行日志」页切换，无需重启）、`LOG_SLOW_REQUEST_MS`（慢请求阈值，毫秒，默认 1000，0 关闭）、`LOG_ERROR_SPIKE_THRESHOLD`（ERROR 尖峰阈值：最近 5 分钟 ERROR 数，默认 10，0 关闭）。

## 构建与验证命令

合并前必须全部通过：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm install
pnpm typecheck    # 递归执行各包的 vue-tsc / nuxt typecheck
pnpm build        # 递归构建 apps 与 packages
```

- 前端单测：`pnpm --filter @aries/admin test` / `pnpm --filter @aries/web test`（均为 vitest run）；根目录 `pnpm test` 串联全部前端包。
- 根 `package.json` 脚本：`dev:admin`、`dev:web`、`build`、`lint`、`typecheck`、`test`。

## 红线规则

细则见 `PROJECT_RULES.md`，以下规则改动前必须先确认：

- Database Query 用参数化 SQL，禁止拼接用户输入。
- 所有 PostgreSQL 结构变更只通过 `migrations/` 下的 SQLx Migration，禁止启动时自动建表。
- API Contract 由 utoipa 代码注解生成（`docs/openapi.yaml` 是机器生成物，禁止手改）：端点变化必须同步更新 `backend/server/src/http/` 下的注解、重新导出并更新前端 API Client；公开接口不得返回密码、Token、内部路径、未发布内容或供应商密钥。
- 密钥只从环境变量读取；生产配置、密码、Token 不得提交 Git。
- 文档遵循 `docs/README.md` 的写作与维护规范：中文为主、专有名词英文、中英文间空格、标识符用反引号、事实与代码一致。
- Git：English Conventional Commits，一个 Commit 一个逻辑主题；不使用 `git reset --hard` 或覆盖他人修改的命令。

## 安全要点

- Password 用 Argon2id（`Argon2PasswordHasher`）；迁移来的 Legacy bcrypt Hash 由 `backend/infra/src/password.rs` 按前缀兼容验证，首次登录成功后升级为 Argon2id；Session Token 只存 SHA-256 Hash 于 PostgreSQL，Browser 通过 `HttpOnly` Cookie（`aries_admin_session`，Path 限定 `/api/admin`，`SameSite=Lax`）持有。
- Login 先撤销旧 Session 再签发新 Session（防 Session Fixation）；修改密码、重置密码或禁用 User 时撤销该 User 全部 Session。
- Admin 写请求（非 GET/HEAD/OPTIONS）的 `Origin` Header 必须精确匹配 `ADMIN_ORIGINS`，见 `backend/server/src/http/mod.rs` 的 `enforce_origin`。
- Role 固定为 `owner`、`editor`、`moderator`；授权以 Backend Permission Guard 为准，前端 Guard 只做入口隐藏。

## 测试策略

风险较高的代码必须有对应测试（分层细则见 `PROJECT_RULES.md` 第 10 节）。HTTP 级 Contract Test 与主流程 E2E 位于 `backend/server/tests/`（共享 Harness 在 `backend/server/tests/common/mod.rs`），通过随机 PostgreSQL Schema 隔离运行，需设置 `ARIES_RUN_DATABASE_TESTS=1` 并提供可用数据库（测试经 `dotenvy` 读取 `.env`）；`tests/contract` 与 `tests/e2e` 目录已预留给跨进程场景，当前为空。修复 Bug 时优先先写一个能复现该 Bug 的测试。

## 参考文档

- 项目规范细则：`PROJECT_RULES.md`
- 文档写作与维护规范、文档地图：`docs/README.md`
- 系统架构与权限矩阵：`docs/architecture.md`
- 实施规划与 Phase 划分（phase-00 至 phase-08）：`docs/implementation-plan.md`、`docs/phases/`
- 数据映射：`docs/database-mapping.md`
- 部署与安全配置：`docs/docker-deployment.md`（唯一权威文档）
- API Contract：`docs/openapi.yaml`
- 快速上手：`README.md`
