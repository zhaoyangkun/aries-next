# Phase 05：评论与运营能力

> 状态：✅ 已完成（例外：邮件通知为 log-only，SMTP 发送未接入，见 §13）　最后验证日期：2026-09-06
>
> 本文档回答：评论、审核、通知、Dashboard 与 Audit 的范围与验收标准。分批交付记录见 §12、§13。

## 1. 目标

补齐博客日常运营所需的 Built-in Comment、审核、回复通知、Dashboard、用户资料和 Audit 查询，让站点在公开发布后可以持续维护。

## 2. Comment Domain

- Comment Target 支持 Article、Page 和 Link Page。
- Comment Thread 支持 Root 与 Parent Reference，但限制最大层级或在展示层扁平化。
- 状态统一为 `pending`、`approved`、`rejected`、`spam`、`recycled`。
- 访客 Email 仅用于通知和 Avatar Policy，不通过 Public API 返回。
- Markdown Comment 由 Backend Render 与 Sanitize；默认禁用 Raw HTML。

## 3. Database 工作

- 新增 `comments`、`comment_notifications` 和必要 Index。
- Comment Target 使用明确约束，防止同时引用多个无关资源。
- 保存 IP Hash、User Agent 摘要和 Moderation Reason，不保存不必要的原始隐私数据。
- 新增 `email_deliveries` 或统一 Notification Job，记录状态、重试与错误分类。
- 完善 `audit_logs` 查询 Index 和 Retention Policy。

## 4. API Contract

```text
GET    /api/public/articles/{slug}/comments
POST   /api/public/comments
POST   /api/public/comments/{id}/replies
GET    /api/admin/comments
GET    /api/admin/comments/{id}
PATCH  /api/admin/comments/{id}/status
POST   /api/admin/comments/{id}/reply
DELETE /api/admin/comments/{id}
GET    /api/admin/dashboard
GET    /api/admin/audit-logs
GET    /api/admin/profile
PUT    /api/admin/profile
```

Public Create DTO 不接受 `status`、`is_admin`、Rendered HTML、IP、Device 或 Avatar URL 等可信字段。

## 5. Backend 工作

- Comment Submit 设置 Rate Limit、Honeypot、内容长度和重复提交检查。
- 根据站点策略决定 Pending 或 Approved，决策只在 Backend。
- Admin Reply 使用当前 Session User，不能由请求伪造管理员身份。
- Email Notification 使用 Queue/Background Job，不阻塞 Comment Request。
- SMTP Secret 不通过 Setting API 回显，Test Email 需要 Owner Permission。
- Dashboard 聚合 Article、Comment、Pending、Recent Activity 和 Job Failure。
- Audit API 支持 Operator、Action、Target、Date Range Filter。

## 6. Admin 工作

- Comment Table 支持状态、内容、Target、作者和时间筛选。
- 支持 Preview、Approve、Reject、Spam、Recycle、Reply 和 Batch Action。
- 显示隐私字段时实施 Permission 与 Masking，避免列表默认暴露完整 Email。
- Dashboard 接入真实聚合数据，提供 Pending Comment 与 Failed Job 快捷入口。
- 增加 Profile、Password Change 和 Active Session 管理。
- Setting 中提供 Comment Policy、Page Size、Moderation 和 Notification Toggle。

## 7. Public Web 工作

- Article Detail 接入 Threaded Comment List、Pagination 和 Submit Form。
- 明确 Pending Feedback，不将未审核内容直接插入 Approved List。
- 支持 Reply Context、Form Validation、Submitting、Success、Rate Limited 和 Error State。
- 隐私声明说明 Email 用途；不显示 Device Fingerprint 等内部字段。

## 8. Twikoo 兼容策略

Built-in Comment 是该 Phase 的 Acceptance Gate。Twikoo 只作为可选 Comment Adapter：

- Site Setting 可选择 `builtin` 或 `twikoo`。
- Public Web Adapter 隔离第三方 Script。
- 第三方 Script 必须满足 CSP、Consent 与 Privacy 要求。
- Twikoo 不进入本 Phase 的阻塞范围，除非现网明确依赖且无法切换。

## 9. Test 与验证

- Unit Test：State Transition、Thread Rule、Notification Policy。
- Integration Test：Submit、Moderate、Reply、Recycle、Count Projection。
- Security Test：XSS、Spam Burst、Admin Impersonation、Email Enumeration。
- E2E：访客评论 -> Pending -> Admin Approve -> Public Visible -> Admin Reply -> Notification Job。
- Dashboard 与 Audit Filter Contract Test。

## 10. Acceptance Gate

1. 访客可以安全提交评论，审核策略和用户反馈明确。
2. 管理员可以在 Admin 完成筛选、审核、回复和回收。
3. Email 发送失败不影响 Comment Transaction，并可观察和重试。
4. Public API 不泄露 Email、IP、Moderation Detail 或内部 User 信息。
5. 评论、Dashboard 和 Audit 核心 E2E 通过。

## 11. 本 Phase 不做

- 不实现完整社区账号、点赞、关注或私信。
- 不使用 AI 自动删除评论。
- 不承诺 Twikoo 与 Built-in Comment 数据双向同步。

## 12. 分批交付记录

Phase 05 拆为两批交付，目前两批的功能主体均已完成：

### 第一批：Admin 运营能力 ✅

- Admin 端点：`GET /api/admin/comments`（状态/关键词/目标筛选 + 分页）、`GET /api/admin/comments/{id}`、`PATCH /api/admin/comments/{id}/status`、`POST /api/admin/comments/{id}/reply`、`DELETE /api/admin/comments/{id}`。
- 运营端点：`GET /api/admin/dashboard`（聚合文章/评论/失败 Job 与最近待审核列表）、`GET /api/admin/audit-logs`（操作人/动作/目标/日期范围筛选 + 分页）。
- Admin 页面：评论管理页（状态 Tab、筛选、审核、回复、删除）、Dashboard 接入真实聚合端点、Audit Log 查询页、Profile 页。
- 权限：评论管理使用 `moderate_comments`；Audit Log 查询使用 `manage_settings`（Owner）。
- 第一批取舍：回复编辑器使用轻量 `textarea`（Render 与 Sanitize 仍由 Backend 完成）；评论列表不 Join 目标标题，仅返回 `target_type` + `target_id`。

### 第二批：Public 评论链路 ✅

- 公开端点：`GET /api/public/articles/{slug}/comments`、`GET /api/public/pages/{slug}/comments`、`POST /api/public/comments`、`POST /api/public/comments/{id}/replies`（见 `backend/server/src/http/public/comments.rs`）。
- Submit 防护：Rate Limit、内容长度与重复提交检查；Public Create DTO 不接受 `status`、`is_admin` 等可信字段。
- Public Web Article Detail 评论组件（Threaded List、Submit Form、Pending 反馈），见 `apps/web/app/components/CommentSection.vue`。
- 访客 Email 非空校验在 Public 提交端点完成；管理员回复允许空 Email（见 Migration `202609050002`）。
- Contract Test：`backend/server/tests/comments_api.rs`、`comments_public_api.rs`。

## 13. 遗留项：邮件通知未真正发送

评论通知目前是 log-only 的 Background Job：`backend/server/src/worker.rs` 中明确标注「email adapter not yet implemented」，只写日志不发邮件；`email_deliveries` 表已在 Migration `202609050001` 中建立，但代码尚未写入或消费该表。SMTP 设置页（`/api/admin/settings/email`）已可配置并保存（`smtp_password` 为 write-only），但缺少实际发送的 Email Adapter。这不影响评论主链路与其余 Acceptance Gate 结论，但作为正式切换前的遗留项记录于此。
