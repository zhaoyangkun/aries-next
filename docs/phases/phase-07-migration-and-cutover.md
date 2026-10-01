# Phase 07：数据迁移与正式切换

> 状态：🚧 进行中（Migrator ETL / Validate / Report 已实现并经一次真实数据演练；三轮正式 Rehearsal 与 Cutover 未做）　最后验证日期：2026-09-07
>
> 本文档回答：如何把旧 MySQL Aries 的数据安全迁移到 PostgreSQL，并完成可回滚的正式切换。

## 1. 目标

将旧 MySQL Aries 的全部受支持数据安全迁移到 PostgreSQL，在可验证、可回滚的前提下完成 Production Cutover，并保持 Legacy URL 与 SEO 权重。

## 2. 前置条件

- Phase 02 至 Phase 06 的目标 Schema 和 API Contract 已冻结。
- `docs/database-mapping.md` 已覆盖所有 Legacy Table 的字段级映射。
- 旧站 Timezone、Character Set、Theme、Storage 和 Comment Provider 已盘点。
- PostgreSQL Backup、Restore、Monitoring 和 Capacity 已验证。
- Legacy MySQL 可以进入只读窗口。

当前进展：前置条件中的 Schema 与 API Contract 已随 Phase 02–06 完成而就绪。Migrator 已实现完整 ETL（`preflight` / `migrate` / `validate` 子命令，幂等 Load 与 Archive/Report 机制，见 `docs/database-mapping.md`「迁移工具现状」），Legacy bcrypt 密码兼容已接入（首次登录升级 Argon2id）；Cutover 与 Rollback 的操作步骤已落地为 `docs/migration-runbook.md`。已于 2026-09-07 用本机真实旧库完成一次全量演练（migrate + validate + 重跑幂等通过）。尚未做的是三轮正式 Rehearsal 与真实 Cutover（运维动作，需维护窗口）。

## 3. 迁移范围

- Users 与 Legacy Password Hash Upgrade Metadata。
- Categories、Articles、Tags、Article Tags。
- Comments 与 Thread Reference。
- Pages、Journals、Galleries、Pictures/Media、Links、Navigation。
- Site、Email、Comment、Parameter、Social、Theme Setting 的安全映射。
- Visit Count、Comment Count、Sort Order、Published At 和 Soft Delete 状态。
- Legacy Slug、URL 和 Redirect Mapping。

旧 Theme Template、JavaScript、明文 Secret 和任意 Head/Footer HTML 不直接迁移。

## 4. Migrator Pipeline

```text
Preflight
  -> Extract
  -> Transform
  -> Load
  -> Validate
  -> Report
```

- Preflight：重复键、非法状态、孤儿记录、Zero Date、乱码、非法 JSON、缺失文件。
- Extract：按主键分批读取，记录 Source Range 和 Checkpoint。
- Transform：Timezone 转 UTC、状态归一、Slug 冲突处理、Secret 分类、HTML 标记风险。
- Load：按 Foreign Key 依赖顺序事务写入，支持 Resume 与 Idempotency。
- Validate：Count、ID Range、Content Hash、Association、Foreign Key、Sequence 和 Redirect。
- Report：记录 Migrated、Skipped、Repaired、Failed 和人工决策。

## 5. 必须明确的转换策略

- Legacy `is_published` 与 `is_recycled` 转为 Article `status`。
- `content` 与 `md_content` 的真实含义通过样本确认后映射到 Markdown Source 与 Rendered HTML。
- Rendered HTML 迁移后重新 Sanitize；Source 存在时优先重新 Render。
- Legacy Password 不明文解密；兼容 Hash 在首次登录成功后升级 Argon2id。
- Picture URL 保留，但无法访问或缺失文件进入 Report，不自动丢弃引用。
- Theme Setting 只迁移到受支持 Appearance 字段，未知 Key 进入 Archive JSON 或 Report。
- Secret Setting 不写入 Report 明文，也不迁移到普通 Setting 表。

## 6. Rehearsal

至少执行三类环境：

1. Sanitized Sample：快速迭代 Mapping 与 Transform。
2. Full Snapshot：使用完整数据量验证性能、Count、Hash 与 Relation。
3. Cutover Rehearsal：按 Production Runbook 执行 Backup、Read-only、Migration、Smoke Test 和 Rollback。

每次 Rehearsal 使用唯一 Run ID，并保存机器可读 JSON Report 与中文摘要。

## 7. Cutover Runbook

操作化版本（真实命令、Operator/阈值占位、观察期写入策略决策点、演练基线）见 `docs/migration-runbook.md`，本文只保留阶段框架：

1. 宣布 Maintenance Window，阻止旧站写入。
2. 创建 MySQL Final Backup，并验证 Backup 可读取。
3. 记录 Source Row Count、最大 ID 和 Cutoff Timestamp。
4. 执行 Full Migration 与 Validation。
5. 启动 Aries Next，执行 Admin 与 Public Smoke Test。
6. 验证 DNS/Proxy、TLS、Cookie、Upload、Email、Sitemap、RSS 和 Redirect。
7. 切换流量并监控 Error Rate、Latency、Database Connection 和 `404`。
8. 保持旧站和 MySQL 只读，进入观察期。

## 8. Rollback

触发条件至少包括：

- 核心 Count 或 Content Hash 不一致。
- Login、Article Detail 或 Media 大面积失败。
- Legacy URL 发生系统性 `404`。
- PostgreSQL Error Rate 或 Latency 超过预设阈值。

Rollback 时将流量切回只读旧站或解除旧站维护模式。Aries Next 切换后产生的新写入不能自动反向写回 MySQL，因此观察期内是否允许新写入必须在 Runbook 中明确。

## 9. Test 与验证

- Migrator Unit Test：每种字段转换和冲突规则。
- Integration Test：分批、Resume、Idempotency、Transaction Rollback。
- Validation Test：Count、Hash、Reference、Sequence、Orphan 和 Redirect。
- E2E：迁移后的管理员登录、文章发布、评论审核、Media 访问和 Public Route。
- SEO Crawl：抽样或全量 Legacy URL 状态、Canonical 和 `301` Chain。

## 10. Acceptance Gate

1. Full Snapshot Rehearsal 无未解释的 Count、Hash 或 Relation 差异。
2. Cutover Rehearsal 与 Rollback Rehearsal 都成功。
3. Legacy URL Redirect Map 完整，关键 URL 不出现 Redirect Chain 或 `404`。
4. Secret、Private Comment Data 和 Password 未出现在日志或 Report。
5. Production Runbook 明确 Operator、时间点、命令、判断阈值和回滚负责人。

## 11. 本 Phase 不做

- 不使用长期双写作为默认方案。
- 不在迁移过程中顺便重写正文内容。
- 不因少量失败而静默跳过记录。

## 12. 完成后更新

- `docs/database-mapping.md`
- Production Migration Runbook
- Redirect Map 与 SEO Validation Report
- Backup、Restore 和 Rollback 记录
