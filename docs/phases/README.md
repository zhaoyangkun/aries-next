# Aries Next Phase 路线图

> 最后验证日期：2026-09-07
>
> 本文档回答：重写工作分成哪些 Phase、当前各 Phase 进行到哪一步、按什么顺序推进。写给所有参与开发的人。

## 1. 使用方式

本目录将博客重写拆成可独立验收的 Phase。开发按依赖顺序推进；一个 Phase 未通过 Acceptance Gate 时，不开始依赖它的高层业务。

功能边界与旧版继承策略见 `docs/blog-feature-map.md`。

## 2. 当前进度

状态标记：✅ 已完成、🚧 进行中、⬜ 未开始。状态以各 Phase 文档中的 Acceptance Gate 核对结果为依据。

| Phase | 目标 | 状态 | 依赖 |
| --- | --- | --- | --- |
| [Phase 00](phase-00-foundation.md) | 工程、Contract、Security 和 Test 基线 | ✅ | 无 |
| [Phase 01](phase-01-auth-and-admin-shell.md) | Authentication 与可用 Admin Shell | ✅ | Phase 00 |
| [Phase 02](phase-02-content-publishing.md) | 打通文章发布主链路 | ✅ | Phase 01 |
| [Phase 03](phase-03-media-and-editor.md) | Media Library 与生产级 Editor | ✅ | Phase 02 |
| [Phase 04](phase-04-public-web-and-seo.md) | 可上线的 Nuxt Public Web | 🚧（页面、SEO、Legacy Redirect 已完成；余真机视觉与可访问性检查） | Phase 02、Phase 03 |
| [Phase 05](phase-05-comments-and-operations.md) | 评论、通知、Dashboard 和运营能力 | ✅（邮件通知为 log-only，SMTP 发送未接入） | Phase 01、Phase 04 |
| [Phase 06](phase-06-extended-content-and-appearance.md) | 页面、日志、图库、友链、导航与外观 | ✅ | Phase 03、Phase 04 |
| [Phase 07](phase-07-migration-and-cutover.md) | 全量迁移、演练、切换与回滚 | 🚧（Migrator ETL 与密码兼容已实现并经一次真实数据演练，Runbook 已就绪；余三轮正式 Rehearsal 与 Cutover） | Phase 02 至 Phase 06 |
| [Phase 08](phase-08-ai-capabilities.md) | AI 写作、审核与 Semantic Search | 🚧（第一批：编辑器助手 SSE、评论 AI 审核、用量审计；Embedding/RAG 未开始） | 稳定的 Content Contract |

补充说明：

- Phase 04 剩余项：Desktop 与 Mobile 真机视觉及可访问性检查，完成后转为 ✅。已知限制：Admin 的 Vditor 运行时资源默认走 unpkg CDN（自托管需把 `vditor/dist` 拷入 `public/` 并配置 `cdn` 选项）；View Count 去重为单机内存窗口；生产部署需保证 Web 域下 `/api` 反代到 Backend。
- Phase 05 剩余项：评论邮件通知目前是 log-only 的 Background Job（见 `backend/server/src/worker.rs`），SMTP 发送与 `email_deliveries` 投递闭环未接入，不影响其余验收结论。
- Phase 08 提前于 Phase 07 启动：AI 不属于首次替换旧站的 Release Gate，且编辑器助手只依赖已稳定的 Content Contract 与 Permission/Audit 基线。
- Phase 07 剩余项：三轮正式 Rehearsal（Sanitized Sample / Full Snapshot / Cutover Rehearsal）与真实 Cutover 属运维动作，执行口径见 `docs/migration-runbook.md`；演练基线与预期归档行为（未识别设置 Key、悬挂评论、死链图床等）已在 Runbook §8 列明。
- API Contract 的权威来源是 `docs/openapi.yaml`，端点细节不在本文重复。

## 3. 依赖关系

```text
Phase 00 Foundation
  -> Phase 01 Auth and Admin Shell
      -> Phase 02 Content Publishing
          -> Phase 03 Media and Editor
          -> Phase 04 Public Web and SEO
              -> Phase 05 Comments and Operations
          -> Phase 06 Extended Content and Appearance
              -> Phase 07 Migration and Cutover
                  -> Phase 08 AI Capabilities（实际已提前并行启动）
```

Phase 03 和 Phase 04 在 Phase 02 Contract 稳定后并行推进；上线前需共同验证 Article Cover、Markdown Asset 和 Article Detail。

## 4. 全局交付规则

每个 Phase 都必须完成以下横向工作：

- Database 变化使用 SQLx Migration，并更新 `docs/database-mapping.md`。
- API 变化同步 `docs/openapi.yaml` 与前端 API Client。
- 管理写操作包含 Authentication、Authorization、Validation 和 Audit。
- Public HTML 经过 Sanitization，Upload 经过 MIME、Size 和 Path 校验。
- UI 具备 Loading、Empty、Error、Disabled 和 Responsive 状态。
- Rust 通过 `cargo fmt`、`cargo clippy` 和 `cargo test`。
- Frontend 通过 `pnpm typecheck` 和 `pnpm build`。
- 关键流程通过 Contract Test 或 E2E 验证（HTTP 级测试位于 `backend/server/tests/`，经隔离 PostgreSQL Schema 运行）。

## 5. Release Gate

Phase 07 完成前，Aries Next 只作为 Preview 环境运行，不替换旧站。正式切换至少要求：

1. Phase 00 至 Phase 07 全部通过 Acceptance Gate。
2. Full Migration Rehearsal 的 Count、Hash 和 Reference Validation 全部通过。
3. Legacy URL 的 `301` Redirect、Canonical、Sitemap 和 RSS 已验证。
4. Admin 登录、文章发布、评论审核、媒体上传和回滚流程完成 E2E。
5. PostgreSQL Backup 与恢复演练成功。

AI 不属于首次替换旧站的 Release Gate。Phase 08 在主站稳定后独立发布。
