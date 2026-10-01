# Phase 08：AI 能力

> 状态：🚧 进行中（第一批已完成：编辑器助手 SSE、评论 AI 审核、用量审计；Embedding / RAG / Semantic Search 未开始）　最后验证日期：2026-09-06
>
> 本文档回答：AI 写作、审核与 Semantic Search 的范围、安全原则与验收标准。与原计划的差异以文末 §12 为准。

## 1. 目标

在 Content Contract、Permission、Audit 和 Migration 稳定后，加入可控、可审计、以人工确认为核心的 AI 写作、审核与 Semantic Search 能力。

AI 不属于首次替换旧站的 Release Gate，可以独立启用、灰度和关闭。实际执行中本 Phase 提前于 Phase 07 启动：编辑器助手只依赖已稳定的 Content Contract 与 Permission/Audit 基线，与数据迁移无耦合。

## 2. 首批 Use Case

- Rewrite：改写选中的 Markdown 片段。
- Summary：根据正文生成摘要 Draft。
- Metadata：建议 Title、Slug、SEO Keywords、Category 和 Tags。
- Comment Assist：给出 Spam/Abuse 风险与理由，不自动删除。
- Semantic Search：基于 Published Article 回答并返回文章与段落引用。
- Usage Audit：按 Operator、Model、Feature、Date 查看 Token、Latency 和 Cost。

## 3. 产品与安全原则

- AI 输出默认是 Suggestion 或 Draft，必须 Preview、Diff、Accept 或 Reject。
- AI 不得自动 Publish、Recycle、Delete、Approve Comment 或修改 Setting。
- Provider 通过 Adapter 接入，Domain 不依赖具体 Vendor SDK。
- Prompt、Model、Temperature、Input Hash 和 Policy 使用 Version 管理。
- 外部内容视为不可信输入，防止 Prompt Injection 和数据外泄。
- 不发送 Password、Token、Private Comment、未授权 Draft 或 Secret Setting。
- 每个 Feature 可独立 Disable，并设置 User、Role 和 Site Quota。

## 4. Database 工作

- 新增 `ai_providers` 的非 Secret Metadata；Secret 使用 Environment 或 Secret Manager Reference。
- 新增 `ai_requests`：Feature、Operator、Model、Prompt Version、Status、Token、Latency、Cost、Error Category。
- 新增 `content_embeddings`：Content Type、Content ID、Chunk、Content Hash、Embedding Model、Vector、Updated At。
- 新增 `ai_suggestions`：Source Revision、Suggestion、Diff、Accepted At、Accepted By。
- 使用 `pgvector` 前先通过 Migration 与 Extension Availability 检查。

## 5. API Contract

```text
POST /api/ai/editor/rewrite
POST /api/ai/editor/summary
POST /api/ai/editor/metadata
POST /api/ai/comments/{id}/moderate
POST /api/ai/search/ask
GET  /api/ai/usage
POST /api/admin/ai/index/rebuild
GET  /api/admin/ai/index/jobs/{id}
```

Editor API 使用 SSE，并支持 Cancellation。Response Event 至少区分 `start`、`delta`、`usage`、`done` 和 `error`。

## 6. Backend 工作

- 定义 Chat、Embedding、Streaming、Usage 和 Error Mapping Trait。
- 实现 Timeout、Cancellation、有限 Retry、Circuit Breaker 和 Provider Health。
- Prompt Template 使用 Version 和 Test Fixture，不在 Handler 中拼接大段字符串。
- Embedding 按 Sanitized Plain Text Chunk，并以 Content Hash 增量重建。
- Article Update、Publish、Recycle 触发 Index Job；Recycled Content 从 Public Index 移除。
- RAG Response 强制返回 Source Article、Slug、Chunk 与引用片段。
- 记录 Cost，但 Audit Log 不保存不必要的完整敏感 Prompt。

## 7. Admin 与 Public Web 工作

- Article Editor Dialog 增加选中文本 Rewrite、Summary 和 Metadata Suggestion。
- Suggestion Panel 使用 Diff，支持部分 Accept、Reject、Regenerate 和 Undo。
- Comment Detail 显示 AI Risk、Reason、Confidence 和 Source Policy，最终状态由管理员决定。
- AI Setting 显示 Provider 状态、Model、Quota、Feature Toggle，不回显 Secret。
- Public Search 增加独立 Semantic/Ask Mode；普通 Keyword Search 始终可用。
- RAG Answer 显示可点击 Citation，不生成无法追踪的来源标签。

## 8. Evaluation 与 Test

- Contract Test：Provider Adapter 的 Stream、Usage、Timeout 和 Error 行为一致。
- Prompt Regression：固定中文文章样本验证格式、引用和禁止行为。
- Security Test：Prompt Injection、Secret Exfiltration、Unauthorized Draft Access。
- Evaluation Set：Summary Faithfulness、Metadata Relevance、Comment False Positive、Citation Accuracy。
- E2E：生成 Suggestion -> Diff -> Accept -> 保存 Draft；取消 Stream 不修改正文。
- Cost Test：Quota 超限、Provider Failure 和 Disabled Feature 有明确用户状态。

## 9. Acceptance Gate

1. 任一 AI 建议都不能绕过人工确认直接修改 Published Content。
2. Provider Failure、Timeout 和 Cancellation 不导致 Draft 丢失或 Editor 卡死。
3. 所有请求都有 Prompt Version、Model、Usage、Latency、Cost 和 Operator Audit。
4. Semantic Answer 的每项事实性引用可以回到 Published Article 与段落。
5. Evaluation 达到发布前设定阈值，且可以按 Feature 一键关闭。

## 10. 本 Phase 不做

- 不构建 Autonomous Publishing Agent。
- 不自动抓取互联网内容用于训练或 RAG。
- 不将 Private Draft、Email、Password 或 Token 发送给外部 Model。
- 不让 AI 代替确定性的 Markdown Parser、Search Filter 或 Permission Engine。

## 11. 完成后更新

- AI Provider 配置和 Data Flow ADR
- Prompt Version 与 Evaluation Report
- Privacy、Retention、Quota 和 Cost Runbook
- Incident Disable 与 Provider Failover 操作说明

## 12. 实施偏差与状态（第一批落地后追加）

### 已落地（第一批）

- 编辑器助手（rewrite / summary / metadata，SSE 流式）与评论 AI 审核；不含 Embedding / RAG / Semantic Search / ai_suggestions / content_embeddings。
- 用量审计：`ai_requests` 表 + `GET /api/admin/ai/usage`；评论 AI 审核结论落 `comments.ai_risk / ai_reason / ai_confidence`。

### 偏差

1. **端点挂在 `/api/admin/ai/*` 而非 §5 的 `/api/ai/*`**：Session Cookie `aries_admin_session` 的 Path 限定 `/api/admin`，挂在 `/api/ai` 下浏览器不会携带 Cookie；挂 admin 下同时自动获得 `enforce_origin` 的 Origin 校验。
2. **评论 AI 审核没有独立的 `POST /api/ai/comments/{id}/moderate` 端点**：审核挂钩在访客提交流程内自动执行（短超时 10s），结论写入评论行供管理端展示；AI 只能把高置信（confidence ≥ 0.8）垃圾评论标记为 `spam`，其余一律走原 `comment_policy`（pending / auto_approve），Provider 失败时完全回退原流程，不影响访客提交。
3. **AI 设置复用 `setting_groups` 机制**（`/api/admin/settings/ai`），未新建 `ai_providers` 表；`api_key` 为 write-only secret（GET 只回 `api_key_set`），Provider 配置按请求实时读取，更新后下一次请求即生效，无需重启。
4. **AI 不自动 approve / publish / delete**（§3 原则落实）：编辑器端点只返回 Suggestion 文本，由前端 Diff/Accept；评论审核只标记风险，最终状态由管理员决定。
5. **双协议支持**：AI 设置新增 `protocol` 字段（`openai` / `anthropic`，默认 `openai` 兼容旧数据）；`DispatchingAiProvider` 按 protocol 分发到 OpenAI 兼容适配器或 Anthropic Messages API 适配器，超时/重试/错误映射与审计语义两协议一致（`backend/infra/src/ai.rs`）。

### 剩余项（未开始）

- Embedding、`content_embeddings` 与 `pgvector` 索引；RAG / Semantic Search（`/api/ai/search/ask`）与引用回链。
- `ai_suggestions` 表与 Diff 落库；Index Rebuild Job（`/api/admin/ai/index/*`）。
- Evaluation Set 与 Prompt Regression 测试；Quota 与按 Feature 一键关闭的管理界面（设置开关已具备，Quota 未实现）。
