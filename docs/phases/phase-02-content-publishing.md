# Phase 02：文章发布主链路

> 状态：✅ 已完成（Acceptance Gate 于 2026-09-04 全部通过）　最后验证日期：2026-09-06
>
> 本文档回答：第一条真实业务链路（登录 → 创建 Draft → 编辑 → 发布 → 回收/恢复）的范围与验收标准。

## 1. 目标

打通“登录 -> 创建 Draft -> 编辑 Markdown -> 选择分类标签 -> 发布 -> 回收或恢复”的第一条真实业务链路，替换 Admin 中的 Mock Data 与 Local Storage 主存储。

## 2. Domain 范围

- Article：Draft、Published、Recycled 状态。
- Category：仅 Article Kind，支持有限父子层级。
- Tag：文章多标签。
- Article Revision：保存关键版本，支持冲突检查和恢复。
- Markdown Render：Backend Render 与 Sanitization。

## 3. Database 工作

- 审查并补全现有 `articles`、`categories`、`tags`、`article_tags` Constraint 和 Index。
- 新增 `article_revisions`：Article、Revision No、Source、Metadata Snapshot、Operator、Created At。
- 为 Slug 冲突、Published Listing、Category/Tag Filter 建立 Index。
- 明确 Optimistic Lock 字段，例如 `version`，避免多个编辑窗口静默覆盖。
- Count 字段由可靠事务或 Projection 更新，客户端不能提交。

## 4. API Contract

```text
GET    /api/admin/articles
POST   /api/admin/articles
GET    /api/admin/articles/{id}
PUT    /api/admin/articles/{id}
PATCH  /api/admin/articles/{id}/status
DELETE /api/admin/articles/{id}
GET    /api/admin/articles/{id}/revisions
POST   /api/admin/articles/{id}/revisions/{revision}/restore
GET    /api/admin/categories?kind=article
POST   /api/admin/categories
PUT    /api/admin/categories/{id}
DELETE /api/admin/categories/{id}
GET    /api/admin/tags
POST   /api/admin/tags
PUT    /api/admin/tags/{id}
DELETE /api/admin/tags/{id}
```

Article List 支持 `page`、`page_size`、`keyword`、`status`、`category_id`、`tag_id`、`sort`，并返回明确 `total`。

## 5. 业务规则

- Published Article 必须有 Title、唯一 Slug、Markdown Source、Rendered HTML 和 `published_at`。
- Draft 可以缺少 Category、Cover 和 Summary，但不能绕过长度与格式限制。
- Publish、Recycle、Recover 和 Delete 使用明确 Command，不允许任意写入 `status`。
- Article Password 只保存 Argon2id Hash；Admin Response 只返回 `password_protected`。
- Backend 根据 Markdown Source 生成 Rendered HTML，并使用 Allowlist Sanitization。
- Category 或 Tag 被引用时禁止直接物理删除，返回引用数量。
- Article Editor 使用 Optimistic Lock；版本冲突时必须提供 Reload 或 Compare，不覆盖服务器版本。

## 6. Admin 工作

- Article List 接入 Server Pagination、Filter、Sort、Selection 和 Error Retry。
- 现有 Article Editor Dialog 接入真实 API，URL 保持在 `/articles`。
- Dialog 支持 Create 与 Edit、Unsaved Guard、Autosave Draft、Publish Confirm 和 Conflict State。
- Category 与 Tag 使用 Combobox/Multi-select，可在权限允许时快速创建。
- 设置区包含 Slug、Summary、Cover URL、Allow Comment、Pinned、Access Password 和 SEO Keywords。
- Recycle 与永久删除使用不同 Danger Confirmation；默认只开放 Recycle。
- Editor 关闭后刷新 Article List，并保持当前 Filter 和 Pagination。

## 7. Test 与验证

- Unit Test：Article State Machine、Slug Normalization、Publish Validation。
- Integration Test：Article CRUD、Tag Transaction、Category Delete Constraint、Revision Restore。
- Contract Test：Pagination Total、Filter 组合、Optimistic Lock Conflict。
- Security Test：Sanitization、Password 不回显、越权写操作。
- E2E：创建 Draft -> 自动保存 -> 发布 -> Public API 可见 -> 回收 -> Public API 不可见 -> 恢复。

## 8. Acceptance Gate

1. Admin 不再依赖 Mock Article 或 Local Storage 作为唯一 Draft 存储。
2. 管理员可以发布一篇包含分类、标签、摘要和自定义 Slug 的 Markdown Article。
3. XSS Payload 无法进入 Public Rendered HTML。
4. 并发编辑不会静默覆盖，Revision 可以恢复。
5. Article 主流程 E2E 全部通过。

## 9. 本 Phase 不做

- 不实现 Media Upload，只允许暂时填写已存在的 Cover URL。
- 不实现公开站完整视觉页面，Public API 仅用于 Contract 验证。
- 不实现 AI Rewrite、AI Summary 或 Semantic Search。

## 10. 完成后更新

- OpenAPI 与生成的 TypeScript Client
- `docs/database-mapping.md` Article、Category、Tag 字段映射
- `docs/implementation-plan.md` 当前状态

## 11. 当前进度

截至 `2026-08-08` 已完成 Article Draft 编辑与状态流转主链路：

- `backend/core`：Article 状态机、Slug Normalization、Draft/Publish Validation、Repository 与 Markdown Renderer Contract。
- `backend/infra`：Article Create、Read、Pagination/Filter、Update、状态迁移、Tag Transaction 和 Revision Snapshot。
- PostgreSQL：`version` Optimistic Lock、Published Content Constraint、`article_revisions` 与列表、Tag Index。
- Markdown：`comrak` GFM Render 与 `ammonia` Allowlist Sanitization。
- Test：Domain Unit Test、Sanitization Security Test 和隔离 PostgreSQL Schema Integration Test。
- Admin API：Article List、Create Draft 和 Markdown Preview 已完成 Authentication、Permission、Origin CSRF、Audit 与统一错误映射。
- Admin API：Article Detail 和 Update 已完成 `version` Optimistic Lock、Audit 与统一 Conflict 错误映射。
- Admin API：Article Publish、Recycle 和 Recover 使用明确 Command，已接入状态机、Revision Snapshot、Audit 和 Optimistic Lock。
- Admin UI：Article List 已使用 Server Filter，点击 Article 行可打开编辑 Dialog；Dialog 会加载 PostgreSQL Markdown Draft 并保存修改，展示 Backend Sanitized Markdown Preview。
- Admin UI：Draft 可在 Editor Dialog 中直接发布；列表操作菜单支持 Publish、Recycle 和 Recover，并通过 `AppConfirmDialog` 二次确认。
- Taxonomy：Article Category 与 Tag 已支持列表读取和快速创建；Editor Dialog 可选择 Category 和多个 Tag，已有 Article 会回显关联关系。
- PostgreSQL：Article Create/Update 在同一事务同步 `article_tags` 后重新读取，Response 的 `tag_ids` 与 Database 保持一致。
- Contract Test：已覆盖 Draft Create、Detail、Update、Preview、List 与 Publish -> Recycle -> Recover 状态流转。

截至 `2026-08-11` 第二轮已完成 Article Settings、Revision Restore、Conflict Compare 与列表补齐：

- Admin API：Create/Update 支持 `access_password` 三态（缺省=不变、`null`=清除、字符串=设置），Argon2id Hash 后入库，Response 只暴露 `password_protected`。
- Admin API：`GET /articles/{id}/revisions` 与 `POST /articles/{id}/revisions/{rev}/restore`，Restore 走 Optimistic Lock，恢复前自动为当前版本留档新 Revision。
- Admin API：`DELETE /articles/{id}` 仅允许 Recycled 状态物理删除；Category/Tag 补齐 PUT/DELETE，被引用时返回 409 与 `reference_count`。
- Admin API：Article List 支持 `sort`（`updated_at|created_at|published_at|title` 白名单）与 `order`（`asc|desc`），非法值回退默认排序。
- Admin UI：Editor Dialog 新增 Settings 面板（Slug、Cover URL、SEO Keywords、Pinned、Allow Comments、Access Password 设置/清除）与 Revision 面板（快照查看、二次确认恢复）。
- Admin UI：保存遇 `ARTICLE_CONFLICT` 进入冲突状态，提供重新加载或保留本地副本，不静默覆盖；关闭有未保存修改时提示放弃确认。
- Admin UI：Article List 接入真正的 Server Pagination（页码与 Page Size）、Category/Tag 过滤和排序；Recycled 文章支持永久删除（独立 Danger 确认）。
- Contract Test：新增 Revision List/Restore、Delete 保护、Taxonomy 引用保护、Pagination Total、Sort 白名单与 Password 不回显用例。

截至 `2026-09-04` 第三轮已完成 Public Article Contract 与主流程 E2E，Acceptance Gate 全部通过：

- Core/Infra：`ContentRepository` 新增 `find_published_article_by_slug`，仅返回 Published 且未软删的 Article；查询显式 `$1::citext` 保持大小写不敏感与索引命中。
- Public API：`GET /api/public/articles` 与 `GET /api/public/articles/{slug}` 匿名只读，不挂 Origin CSRF 层；List 强制 Published、支持 `page/page_size/keyword/category_id/tag_id/sort/order`（默认 `published_at desc`，非法值回退）；Response DTO 剥离 `markdown_source`、`author_id`、`version`。
- 密码文章：Public Detail 返回 `password_protected: true` 且 `rendered_html: null`，解锁接口留待 Phase 04。
- Contract Test（`backend/server/tests/public_articles.rs`）：Published 可见性、Pagination Total、Category/Tag/Keyword 过滤、Sort 回退、Draft/Recycled/未知 Slug 404、密码不回显正文、XSS Sanitization、无 Origin/Cookie 可匿名访问。
- 主流程 E2E（`backend/server/tests/article_lifecycle.rs`）：Bootstrap 登录 → 创建 Draft（Public 不可见）→ 发布（可见）→ 回收（不可见）→ 恢复为 Draft → 重新发布（再次可见），全部通过真实 HTTP 与隔离 PostgreSQL Schema 验证。
- 同时修复 `update_article` 在未修改密码时 SQL 占位符不连续导致所有无密码更新失败的存量 Bug。

Acceptance Gate 核对：Admin 不再依赖 Mock/Local Storage；可发布含分类、标签、摘要和自定义 Slug 的 Markdown Article；XSS 无法进入 Public Rendered HTML；并发冲突不静默覆盖且 Revision 可恢复；主流程 E2E 通过。本 Phase 已完成。
