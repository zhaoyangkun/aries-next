# Aries Next

> 最后验证日期：2026-09-07

本文面向首次接触本项目的开发者，目标是让你照着步骤把整套系统在本地跑起来。项目文档的写作与维护规范见 `docs/README.md`。

Aries Next 是博客系统 Aries 的新一代重写实现，采用 Modular Monolith 架构：一个 Rust API Process、一个 PostgreSQL Database、一个 Vue 3 Admin SPA 和一个 Nuxt 4 SSR Public Web。旧版 Aries（Go + MySQL）在正式切换前保持可运行，作为功能行为、数据迁移和 URL 兼容性的基准。

## 技术栈

- Backend：Rust（edition 2024，最低 1.85）、Axum 0.8、Tokio、SQLx 0.8、Tower
- Database：PostgreSQL 17（或兼容版本），SQLx Migration 管理 Schema
- Admin：Vue 3、TypeScript、Vite 8、vue-router 5（文件路由）、Pinia 4、Tailwind CSS 4、shadcn-vue、Reka UI
- Public Web：Nuxt 4（4.5.1）、Vue 3、SSR、Tailwind CSS 4
- Migration：Rust 工具，MySQL 只读源 + PostgreSQL 目标（完整 ETL：`preflight` / `migrate` / `validate` 子命令，详见 `docs/migration-runbook.md`）
- AI：Provider Adapter、SSE 流式输出（已实现编辑器助手与评论 AI 审核）；Embedding、RAG 仍在规划阶段

## 目录结构

```text
backend/server          # aries-server：Axum HTTP Server、Router、Middleware、Config
backend/core            # aries-core：Domain Model、Repository/Service Contract（trait）
backend/infra           # aries-infra：PostgreSQL Repository、Storage（local/S3）、Password Hash、Markdown 渲染、AI Adapter
backend/migrator        # aries-migrator：MySQL → PostgreSQL 迁移工具
apps/admin              # @aries/admin：Vue 3 管理端 SPA
apps/web                # @aries/web：Nuxt 4 SSR 公开站
packages/design-tokens  # @aries/design-tokens：两端共享的 Brand Token（仅导出 theme.css）
packages/api-client     # 预留的 API Client 包（当前为空）
migrations              # PostgreSQL 版本化 SQLx Migration
docs                    # 架构、ADR、数据映射、Phase 规划、OpenAPI Contract
tests                   # 跨进程 Contract Test 与 E2E Test 目录（当前为空，为规划预留）
```

## 环境要求

- Rust stable ≥ 1.85（`rust-toolchain.toml` 锁定）
- Node.js 24+
- pnpm 11+（`packageManager: pnpm@11.9.0`）
- PostgreSQL 17（或兼容版本，自备实例；库中需预装 `citext` 扩展）

## 启动步骤

### 1. 配置环境变量

```powershell
Copy-Item .env.example .env
```

通常只需把 `BOOTSTRAP_SECRET` 改成至少 24 字符的随机串（必填，用于一次性创建首个 Owner）。数据库配置默认值：

```dotenv
DATABASE_HOST=localhost
DATABASE_PORT=5433
DATABASE_USERNAME=aries
DATABASE_PASSWORD=aries
DATABASE_NAME=aries
DATABASE_SCHEMA=public
```

系统环境变量优先于 `.env`。服务启动时会自动创建 `DATABASE_SCHEMA` 指定的 Schema，并执行 SQLx Migration。若 `DATABASE_HOST/USERNAME/PASSWORD/NAME` 一个都未设置，则回退读取 `DATABASE_URL` 连接串。

### 2. 准备 PostgreSQL

使用 `.env` 中 `DATABASE_*` 指向的既有实例（默认本机 `5433`，账号/密码/库名均为 `aries`）。项目不附带 Docker 容器，数据库需自备；库中需预装 `citext` 扩展（`CREATE EXTENSION IF NOT EXISTS citext WITH SCHEMA public`）。

### 3. 启动 Backend

```powershell
cargo run -p aries-server
```

`-p` 是 `--package` 的缩写，用于从 Cargo Workspace 中选择 `aries-server` Package。

Backend 监听 `SERVER_ADDR`（默认 `0.0.0.0:8088`），未注册根路径 `/`，访问返回 404。健康检查：

- Liveness：`http://127.0.0.1:8088/api/health/live`
- Readiness：`http://127.0.0.1:8088/api/health/ready`

### 4. 启动前端

首次启动先安装依赖：

```powershell
pnpm install
```

```powershell
pnpm dev:admin   # Admin：http://127.0.0.1:5173，Vite 将 /api 代理到 http://localhost:8088
pnpm dev:web     # Public Web：http://127.0.0.1:3000
```

## 常用开发命令

合并前以下命令必须全部通过：

```powershell
# Rust
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 前端
pnpm lint         # 递归执行各包 lint
pnpm typecheck    # 递归执行各包 vue-tsc / nuxt typecheck
pnpm build        # 递归构建 apps 与 packages

# Admin 单元测试（vitest run）
pnpm --filter @aries/admin test
```

涉及数据库的测试（Integration / Contract / E2E）默认跳过，需设置 `ARIES_RUN_DATABASE_TESTS=1` 并提供可用数据库（测试经 `dotenvy` 读取 `.env`）。

## 文档地图

文档写作与维护规范、完整文档清单见 [docs/README.md](docs/README.md)。

- [项目规范细则](PROJECT_RULES.md)
- [系统架构与权限矩阵](docs/architecture.md)
- [实施规划与 Phase 划分](docs/implementation-plan.md)
- [数据映射](docs/database-mapping.md)
- [迁移与切换 Runbook](docs/migration-runbook.md)
- [部署与生产配置](docs/docker-deployment.md)
- [OpenAPI Contract](docs/openapi.yaml)
- [ADR 0001：在独立 Repository 中开发](docs/adr/0001-separate-repository.md)
- [ADR 0002：评论域设计决策](docs/adr/0002-comment-domain-design.md)
- [AI Agent 入口上下文](AGENTS.md)
