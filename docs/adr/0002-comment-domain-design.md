# ADR 0002：评论域设计决策

> 最后验证日期：2026-09-06。正文中以「现状注记」标注了已被后续实现取代或尚未落地的内容，历史决策原文保留。

## 状态

已接受。

## 背景

Phase 05 需要为 Aries Next 实现内置评论系统。旧 Aries 使用 Twikoo 第三方评论，新系统需要支持 Built-in Comment 与可选 Twikoo Adapter。评论域需要支持：访客提交评论、管理员审核、回复通知、Dashboard 聚合。

## 决策

### 1. 评论层级策略

采用 **Root + Parent 扁平存储、显示层重组** 方案：

- `comments` 表保存 `root_id`（根评论）和 `parent_id`（直接回复目标），物理存储为扁平行。
- Public API 返回时按 Thread 重组（最多 2 层），Admin API 默认扁平列表便于批量操作。
- 限制最大回复层级为 2 层（Root → Reply），超出的回复平铺到 Root 下。理由：博客评论通常不需要深层嵌套，2 层已足够表达对话关系。

现状注记：与代码一致，见 `backend/core/src/comments.rs` 与 `backend/server/src/http/public/comments.rs`。

### 2. 评论状态机

```
pending → approved  (管理员批准)
pending → rejected  (管理员拒绝)
pending → spam      (管理员标记垃圾)
approved → recycled (管理员回收)
rejected → recycled
spam → recycled
recycled → approved (恢复)
```

- 访客提交默认 `pending`，由站点设置 `comment_auto_approve` 决定是否自动批准。
- 状态转换由 Backend Permission Guard 控制，前端只做 UI 隐藏。

现状注记：状态机与代码一致（`CommentStatus::can_transition_to`，`pending` 不能直接回收）。设置项名称已被 `comment_policy` 取代——`site_settings.comment_policy` 取值为 `closed` / `moderated` / `auto_approve`（见 migration `202609050001`），`closed` 直接拒绝访客提交。

### 3. Target 类型约束

评论 Target 使用 `target_type` + `target_id` 二元组，`target_type` 受 Check Constraint 限制：

- `article`：文章评论
- `page`：自定义页面评论（Phase 06 启用）
- `link`：友情链接页面评论（Phase 06 启用）

同一 `target_type` + `target_id` + `parent_id` 组合下的评论按时间正序排列。

现状注记：`article` 与 `page` 评论均已实现（`/api/public/articles/{slug}/comments`、`/api/public/pages/{slug}/comments`）；`link` 类型在 Schema 中保留，但对访客提交暂不开放（`resolve_target` 对 `link` 直接返回 NotFound）。

### 4. 隐私保护

- 访客 `email` 仅用于通知和 Gravatar Hash，**绝不通过 Public API 返回**。
- 保存 `ip_hash`（SHA-256）和 `user_agent_digest`（前 128 字符 SHA-256），不保存原始 IP 和完整 UA。
- Admin 查看评论详情时，Email 仅对 Owner 可见，Editor/Moderator 看到脱敏版本。

现状注记：

- Public API 不返回 Email 这一点与代码一致；头像已由 Gravatar 改为 Cravatar（`https://cravatar.cn/avatar/{email_sha256}`）。
- `user_agent_digest` 实际对整个 User-Agent 字符串取 SHA-256，未做前 128 字符截断。
- 「Email 仅对 Owner 可见、其余角色脱敏」的分级方案已被取代：Admin API 向所有具备 `ModerateComments` 权限的角色返回明文 Email（用于辅助审核判断，见 `backend/server/src/http/comments.rs`）。

### 5. 内容处理

- 评论内容使用 Markdown 格式，Backend 使用 `comrak` 渲染并通过 `ammonia` 消毒。
- 禁止 Raw HTML 输入，允许的 Markdown 元素：段落、粗体、斜体、链接、行内代码、代码块。
- 内容长度限制：1–2000 字符（原始 Markdown）。

现状注记：渲染管线与长度限制与代码一致（`validate_content_length` 与数据库 CHECK 均为 1–2000 字符）。

### 6. 反垃圾策略

第一版使用基础防护，不引入第三方服务：

- Honeypot 字段（`website`，隐藏的表单字段，有值则静默丢弃）。
- 内容长度校验（1–2000 字符）。
- 同一 Email + Target 的重复提交检查（5 分钟窗口）。
- 同一 IP Hash 的提交频率限制（10 次/小时）。

现状注记：本节方案大部分已被取代：

- Honeypot 方案未采用：`website` 字段实际用作访客公开的个人主页链接（对齐 Twikoo，昵称渲染成外链），入库前校验 http/https，Public API 正常回传。
- 限流已由「Email 重复提交检查 + IP Hash 10 次/小时」改为统一的内存限流：同一客户端指纹（IP + UA）60 秒最多提交 3 条（`SUBMIT_RATE_LIMIT`，见 `backend/server/src/http/public/comments.rs`）。重复提交检查未实现。
- 后续新增了 AI 审核挂钩：高置信垃圾评论自动标记为 `spam`，AI 调用失败时静默回退、不影响提交（`moderate_comment_with_ai`）。

### 7. 通知机制

Phase 05 第一版使用 Background Job 异步发送通知邮件：

- 新评论提交时，通知该 Target 的所有已批准评论者（可选退订）。
- 管理员回复时，通知被回复的评论者。
- 通知 Job 失败不回滚评论事务，记录到 `email_deliveries` 表供后续重试。

现状注记：仅部分落地。`comment_notification` Job 类型与 `email_deliveries` 表已建好（migration `202609050001`），但 Email Adapter 尚未接入，Worker 执行该任务时只记录日志（见 `backend/server/src/worker.rs` 的 `comment_notification`）。「可选退订」未实现。

### 8. Dashboard 聚合

Dashboard 端点返回以下统计：

- 文章总数（按状态分组）
- 评论总数、待审核数、今日新增
- 最近 5 条待审核评论（快速入口）
- 最近 5 条失败的后台任务
- 最近 7 天的文章发布趋势

现状注记：前四项已实现（见 `backend/server/src/http/dashboard.rs` 与 `PostgresCommentRepository::dashboard_stats`）；「最近 7 天的文章发布趋势」未实现，`DashboardResponse` 中无该字段。

## 后果

- 评论系统完全内置，不依赖外部服务。
- 2 层回复限制简化了查询和展示逻辑。
- 隐私保护策略符合 GDPR 基本要求（Public API 不出 Email、IP/UA 只存 Hash 已落实；Admin 侧 Email 脱敏分级未按原方案执行，见第 4 节注记）。
- Background Job 机制与现有的 `media_cleanup` / `metadata_probe` 复用同一框架。
