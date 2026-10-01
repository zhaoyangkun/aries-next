# Phase 01：Authentication 与 Admin Shell

> 状态：✅ 已完成（2026-08-02）　最后验证日期：2026-09-06
>
> 本文档回答：管理员如何安全初始化账号、登录、恢复 Session、退出，以及 Admin Shell 需要具备哪些能力。

## 1. 目标

让管理员能够安全初始化账号、登录、恢复 Session、退出，并在权限感知的 Admin Shell 中操作。该 Phase 是所有管理 API 的前置条件。

## 2. 产品决策

- 不保留旧版公开 Registration；使用一次性 Bootstrap 创建首个 Owner。
- 使用 PostgreSQL Session Store 和 HttpOnly Cookie，不在 Local Storage 保存长期 Token。
- 首次只需要 `owner`、`editor`、`moderator` 三种 Role；若当前只有单管理员，仍保留可扩展 Role Contract。
- Captcha 不作为每次登录的固定步骤；登录失败达到阈值后再触发 Challenge。
- Password 使用 Argon2id。Legacy Password Hash 仅在迁移后的首次成功登录时升级。

## 3. Database 工作

- 扩展 `users`：状态、Role、最后登录时间、Password Updated At。
- 新增 `admin_sessions`：Session ID Hash、User、Expiry、Last Seen、IP/User Agent 摘要。
- 新增 `password_reset_tokens`：Token Hash、Expiry、Consumed At。
- 新增 `audit_logs` 基础表，记录登录和管理写操作。
- 所有 Session 与 Reset Token 支持主动撤销和过期清理。

## 4. API Contract

```text
POST /api/admin/bootstrap
POST /api/admin/auth/login
POST /api/admin/auth/logout
GET  /api/admin/auth/session
POST /api/admin/auth/password/forgot
POST /api/admin/auth/password/reset
GET  /api/admin/profile
PUT  /api/admin/profile
PUT  /api/admin/profile/password
```

`/bootstrap` 仅在无用户且持有一次性 Bootstrap Secret 时可用，成功后永久关闭。

## 5. Backend 工作

- 实现 Session Middleware、Current User Extractor 和 Permission Guard。
- 登录、Bootstrap、Forgot Password 和 Reset Password 设置 Rate Limit。
- 写入 Login Success、Login Failure、Logout、Password Change Audit。
- Session Cookie 明确 `HttpOnly`、`Secure`、`SameSite`、Path 和 Max Age。
- 对状态禁用用户立即撤销全部 Session。
- Email Adapter 尚未配置时，Forgot Password 返回一致响应且不泄露账号是否存在。

## 6. Admin 工作

- 实现 Bootstrap、Login、Forgot Password 和 Reset Password 页面。
- Pinia 仅保存当前 Session View，不持久化 Cookie Secret。
- Router Guard 支持 Anonymous、Authenticated 和 Permission Required。
- 完善 Sidebar、Mobile Drawer、Breadcrumb、User Menu、Logout 和 Route Loading。
- 实现 `401` Session Recovery、`403` Forbidden 和 Network Offline State。
- 导航项根据 Permission 隐藏，但 Backend 仍必须独立授权。

## 7. Test 与验证

- Unit Test：Password Policy、Permission Matrix、Session Expiry。
- Integration Test：Bootstrap 只能成功一次；Session Create、Rotate、Revoke。
- Contract Test：Login 错误不区分用户不存在与密码错误。
- E2E：Bootstrap -> Login -> Refresh 恢复 Session -> Logout -> Protected Route 拒绝访问。
- Security Test：CSRF、Cookie Attribute、Login Rate Limit 和 Session Fixation。

## 8. Acceptance Gate

1. 新环境可以安全创建首个 Owner，之后 Bootstrap API 不再可用。
2. Browser Refresh 后 Session 可恢复，Logout 后旧 Cookie 无法继续访问。
3. 所有 `/api/admin/*` 写接口默认要求 Authentication。
4. Role 与 Permission 在 API 和 Admin Navigation 中行为一致。
5. Authentication E2E 和 Security Test 全部通过。

## 9. 本 Phase 不做

- 不实现 Public User Registration 或读者账号。
- 不接入 OAuth、OIDC 或 SSO。
- 不实现复杂组织、多租户或自定义 Role Builder。

## 10. 完成后更新

- OpenAPI Authentication Scheme
- `docs/architecture.md` Session 与 Permission 章节
- Deployment Cookie、Proxy 和 HTTPS 配置

## 11. Database Integration Test

默认 `cargo test --workspace` 使用 Mock Repository 验证 Domain 与 HTTP Contract，不要求开发机必须运行 PostgreSQL。需要验证真实 Query 与 Migration 时，在项目根目录执行：

```powershell
$env:ARIES_RUN_DATABASE_TESTS='1'
cargo test -p aries-infra postgresql_repository_covers_session_and_password_reset_lifecycle -- --nocapture
Remove-Item Env:ARIES_RUN_DATABASE_TESTS
```

测试会创建随机命名的 `aries_test_*` Schema，覆盖 Bootstrap 唯一性、Session Create/Revoke、Password Reset 原子消费与 Session Revoke，完成后删除该测试 Schema。

## 12. 交付记录

完成日期：2026-08-02。

- Backend 已实现一次性 Bootstrap、Argon2id Password、PostgreSQL Session、Permission Guard、Profile、Password Change 与 Password Reset Contract。
- Admin 已实现 Authentication 页面、Session Restore、Permission-aware Router/Navigation、`403`、Offline State、Route Loading、Profile 和 Logout。
- API Contract 位于 `docs/openapi.yaml`，Deployment 安全要求位于 `docs/docker-deployment.md`。
- Rust Unit、Database Integration、API 功能测试与 Admin Store Unit Test 均通过。
- Mobile `390x844` 与 Desktop `1440x900` 已检查，无横向溢出，Browser Console 无 Error。
- Email Adapter 属于 Phase 05；当前 Forgot Password 只生成 Hash 后的 Reset Token 记录，不发送或返回原始 Token。
