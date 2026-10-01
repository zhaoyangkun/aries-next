# Phase 00：工程与 Contract 基线

> 状态：✅ 已完成　最后验证日期：2026-09-06
>
> 本文档回答：后续所有业务共用哪些工程、API、Database、Security 和 Test 基线。写给出后端与前端开发者。

## 1. 目标

建立所有后续业务共用的工程、API、Database、Security、Observability 和 Test 基线，使前后端能够围绕稳定 Contract 开发。

## 2. 实际交付的基线

- Rust Cargo Workspace（`backend/` 四个 Crate）与 pnpm Workspace（`apps/`、`packages/`）。
- PostgreSQL 独立 Host、Username、Password、Database、Schema 配置；启动时创建 Schema 并执行 SQLx Migration（见 `backend/server/src/main.rs`）。
- 统一错误响应：`{ error: { code, message, details? } }`，不返回 SQLx 原始错误（见 `backend/server/src/http/error.rs`）。
- 路由按访问边界分层：`/api/public/*`、`/api/admin/*`、`/api/health/*`（见 `backend/server/src/http/`）。
- Security Header、精确 CORS（`ADMIN_ORIGINS` 白名单）、内存滑动窗口 Rate Limit（见 `backend/server/src/security.rs`、`rate_limit.rs`）。
- 结构化日志双输出：控制台 Pretty / 生产 JSON，文件按天切分并自动清理（见 `backend/server/src/logging.rs`）。
- Health API：`/api/health/live` 与 `/api/health/ready`，Readiness 检查 Database，不泄露连接信息。
- Integration Test Harness：HTTP 级 Contract Test 位于 `backend/server/tests/`，通过随机命名的隔离 PostgreSQL Schema 运行（需 `ARIES_RUN_DATABASE_TESTS=1`）。
- Admin、Public Web 和共享 Design Token（`@aries/design-tokens`）的可构建骨架。

## 3. 与原始计划的偏差

计划中的以下事项在实际实施中被替换或裁剪，以本节为准：

- **OpenAPI 不用 Utoipa 生成**：Contract 改为手工维护的 `docs/openapi.yaml`（唯一权威来源），Server 代码不再生成 OpenAPI JSON，也不提供 Swagger UI。
- **Validation 不引入 Garde**：Request 校验在 Handler 与 DTO 层手写，配合统一错误映射。
- **Session 不用 `tower-sessions`**：Admin Session 为自研实现（PostgreSQL 存 SHA-256 Hash + HttpOnly Cookie，见 Phase 01）。
- **错误响应未携带 `request_id`**：排障依赖结构化日志，不随响应回传 Trace 标识。
- **前端不生成 TypeScript Client**：Admin 使用 axios 手写 API 层（`apps/admin/src/modules/**/api` 与 `src/shared/api`）；`packages/api-types` 未创建，`packages/api-client` 保留为空包。
- **CI 管线未搭建**：合并前验证依赖本地统一命令（`cargo fmt --check`、`cargo clippy`、`cargo test`、`pnpm typecheck`、`pnpm build`）。

## 4. Database 约定（沿用至今）

- 所有结构变更只通过 `migrations/` 下的 SQLx Migration；已发布 Migration 不修改，只追加新版本（Roll-forward）。
- 不在 Application 启动时创建业务表。
- Session、Audit、Background Job 等表在对应 Phase 的 Migration 中按需新增，不提前建空表。

## 5. 本 Phase 不做

- 不实现 Login、Article CRUD 或任何 AI Provider。
- 不创建 Redis 依赖。
- 不提前设计 Microservice 或 Event Bus。

## 6. Acceptance Gate 核对

1. ✅ Contract 稳定可用：`docs/openapi.yaml` 手工维护并随端点同步，Admin axios 层可调用全部已实现 API。
2. ✅ 错误响应包含稳定 `code`，不返回 SQLx 原始错误（`request_id` 裁剪，见 §3）。
3. ✅ Test 可在独立 PostgreSQL Schema 重复运行（`backend/server/tests/common/mod.rs`）。
4. ✅ Production 配置下 CORS 白名单、无公开 Swagger、Error Detail 不含内部信息。
5. ⬜ CI 未搭建；本地统一验证命令全部通过并作为合并前门槛。
