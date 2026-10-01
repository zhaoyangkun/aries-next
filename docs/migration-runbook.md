# Production 迁移与切换 Runbook

> 最后验证日期：2026-09-07
>
> 本文档回答：正式 Cutover 当天按什么顺序执行哪些命令、看什么指标、什么情况下回滚、由谁执行。写给执行迁移的 Operator 与负责决策回滚的负责人。演练与验收标准见 `docs/phases/phase-07-migration-and-cutover.md`；字段级映射见 `docs/database-mapping.md`。

## 目录

- [1. 角色与时间点（占位）](#1-角色与时间点占位)
- [2. 准备检查单](#2-准备检查单)
- [3. Cutover 步骤](#3-cutover-步骤)
- [4. Smoke Test](#4-smoke-test)
- [5. 流量切换与监控](#5-流量切换与监控)
- [6. Rollback](#6-rollback)
- [7. 观察期新写入策略（决策点）](#7-观察期新写入策略决策点)
- [8. 预期行为清单（演练基线）](#8-预期行为清单演练基线)

## 1. 角色与时间点（占位)

| 项目 | 值（待填） |
| --- | --- |
| Operator（执行人） | 待定 |
| 回滚决策负责人 | 待定 |
| Maintenance Window | 待定（建议低峰时段，预留 ≥ 4 小时） |
| 观察期 | 待定（建议 ≥ 7 天） |
| Error Rate 告警阈值 | 待定（建议 5xx > 1% 持续 5 分钟） |
| P95 Latency 阈值 | 待定（建议 > 1s 持续 10 分钟） |

## 2. 准备检查单

Cutover 前必须全部确认：

- [ ] 三轮 Rehearsal（Sanitized Sample / Full Snapshot / Cutover Rehearsal）已通过，Report 留档。
- [ ] `.env` 的 `MIGRATION_POSTGRES_URL` 指向真实 PostgreSQL 实例与端口。已知偏差：仓库默认 `.env` 写的是 `5433`，本机实例实际监听 `5432`——以实际部署为准，Preflight 只连 MySQL，不会暴露这个错误，必须用 `migrate`/`validate` 实测连接。
- [ ] `MIGRATION_MYSQL_URL` 只读账号可用；目标库 `DATABASE_SCHEMA` 已随 `aries-server` 启动完成 SQLx Migration（Migrator 只写数据，不建表）。
- [ ] 旧站 Timezone 已确认（决定 `--source-offset`，默认 `+08:00`）。
- [ ] 媒体策略已决策：是否允许迁移期下载旧图床文件（见 §8 的性能提示）；目标 `MEDIA_LOCAL_DIR` 磁盘余量充足。
- [ ] Owner 账号清单已确认（默认最小 id 账号为 `owner`，其余为 `editor`；如需多 Owner 用 `--owner-ids 1,2` 显式指定）。
- [ ] PostgreSQL 备份/恢复流程已演练。

## 3. Cutover 步骤

### 3.1 宣布 Maintenance Window

- 旧站进入只读/维护模式，阻止一切写入（文章、评论、媒体、设置）。

### 3.2 MySQL Final Backup

- 对旧库做最终一致性备份（`mysqldump` 或实例快照），并验证备份可读取（抽样还原到临时库）。
- 备份文件路径与校验值记录到本次 Run 的存档目录。

### 3.3 记录 Cutoff

记录并保存：

- 各源表 Row Count 与最大 ID（`preflight` 输出的 `stats` 即为基线）：

```powershell
aries-migrator preflight `
  --mysql-url "$env:MIGRATION_MYSQL_URL" `
  --postgres-url "$env:MIGRATION_POSTGRES_URL" `
  --report "runs/<run-id>/preflight.json"
```

- Cutoff Timestamp（旧站进入只读的时刻）。
- Preflight 存在 hard error 时必须中止，不得强行迁移；`violations`（超长/空值字段）只有在逐条确认后才允许追加 `--truncate-violations`。

### 3.4 Full Migration

```powershell
aries-migrator migrate `
  --mysql-url "$env:MIGRATION_MYSQL_URL" `
  --postgres-url "$env:MIGRATION_POSTGRES_URL" `
  --pg-schema aries `
  --source-offset "+08:00" `
  --media-base-url "https://<旧站域名>" `
  --archive "runs/<run-id>/archive.jsonl" `
  --report "runs/<run-id>/migrate.json"
```

要点：

- 媒体下载是默认行为；断网演练或纯结构验证可加 `--skip-media-download`（全部记为 `legacy_url` 外链）。
- 命令幂等：中断后原地重跑即 Resume（`ON CONFLICT (id) DO NOTHING`），无需清库。
- 结束时会自动重置所有保留原 ID 的 IDENTITY 列 Sequence。

#### 3.4.1 对脏目标库做清库重迁（仅演练环境）

当目标 Schema 已存在与旧库冲突的遗留数据（如开发期手工录入的同名标签），需要清空后整体重迁时，必须遵守两条红线（2026-09-12 本机演练实测教训）：

- **不得清空 `_sqlx_migrations`**，否则 Server 启动时会重放全部 SQLx Migration 并撞上已存在的表（`relation "users" already exists`）。只 TRUNCATE 业务表：`... WHERE schemaname = 'aries' AND tablename <> '_sqlx_migrations'`。
- 清库后必须补回 Migration 的种子数据，否则公开端点 500：`INSERT INTO site_settings (id) VALUES (1);`（`202608050001` 播种，Server 无任何兜底）。
- 目标库已有数据与旧库发生 **natural key 冲突**（如 `tags.name`、`categories (kind, slug)`）时，`ON CONFLICT (id) DO NOTHING` 无法兜底，Migrate 会以 unique violation 中止；这正是需要清库重迁或逐条修复的信号，禁止擅自改造为覆盖写入。

### 3.5 Validation

```powershell
aries-migrator validate `
  --mysql-url "$env:MIGRATION_MYSQL_URL" `
  --postgres-url "$env:MIGRATION_POSTGRES_URL" `
  --pg-schema aries `
  --archive "runs/<run-id>/archive.jsonl" `
  --migrate-report "runs/<run-id>/migrate.json" `
  --report "runs/<run-id>/validate.json"
```

- 退出码非零即失败：保留全部 Report 与 Archive，按 §6 决策。
- 检查项：行数、主键 min/max、Article/Comment/Page/Journal 内容 Hash 抽查（源侧用同一 `ComrakMarkdownRenderer` 现算）、`article_tags` 关联计数、Archive 对账（源行数 = migrated + skipped + archived_rows + failed）、Sequence 核查。

### 3.6 启动 Aries Next

- 正常启动 `aries-server` 与前端构建产物；确认 `ADMIN_ORIGINS`、Session Cookie、媒体目录指向生产值。

## 4. Smoke Test

按以下顺序人工核验（任一失败即进入 §6 评估）：

- [ ] Admin 登录：用旧站管理员账号密码登录（Legacy bcrypt Hash 验证由服务端兼容，首次登录成功后自动升级为 Argon2id）。
- [ ] 文章列表/详情、受密码保护文章的解锁、回收站。
- [ ] 评论线程层级与审核状态；友链页评论（`type=2`）已入库但前台暂不展示（预期，见 §8）。
- [ ] 媒体库：抽查 `provider='local'` 文件可访问、`legacy_url` 外链可打开。
- [ ] Public 端：首页/文章/分类/标签/归档/搜索/友链/日志/图库/自定义页；`/sitemap.xml`、`/rss.xml`、`/robots.txt`；旧版 URL 的 `301` 跳转。
- [ ] 设置：站点信息、邮件分组（`smtp_password` 只应显示为已设置标记）、评论策略。
- [ ] 新写链路：发布一篇文章、上传一张图、提交并审核一条评论。

## 5. 流量切换与监控

1. DNS/Proxy 切换到 Aries Next，确认 TLS 证书与 Cookie Domain。
2. 监控指标（阈值见 §1）：Error Rate、P95 Latency、Database Connection 数、`404` 比例（重点比对 Legacy URL）、后台 Job 失败数。
3. 旧站与 MySQL 保持只读，不得下线。

## 6. Rollback

### 触发条件（满足其一即由回滚负责人决策）

- `validate` 非零退出，或核心 Count / Content Hash 不一致且无法解释。
- Login、Article Detail 或 Media 访问大面积失败。
- Legacy URL 发生系统性 `404`。
- PostgreSQL Error Rate / Latency 超过 §1 阈值。

### 步骤

1. DNS/Proxy 切回旧站，解除旧站维护模式。
2. Aries Next 停写（停服或置只读）。
3. 保留 PostgreSQL 目标库现场（不删除），归档 `runs/<run-id>/` 下全部 Report 与 Archive。
4. 观察期内 Aries Next 产生的新写入按 §7 的既定策略处理。
5. 复盘并修复后，从 §3 重新执行（可复用同一 Archive 命名新 run-id）。

## 7. 观察期新写入策略（决策点）

Aries Next 切换后的新写入不能自动反向写回 MySQL。观察期内二选一，Cutover 前必须在此文档中勾选：

- [ ] **策略 A：冻结写入**。观察期内 Admin 只读（暂停发布/评论），Rollback 无任何数据损失。
- [ ] **策略 B：允许写入**。接受 Rollback 时丢失观察期新写入，或事后人工补录（按 `audit_logs` 与新内容清单）。

## 8. 预期行为清单（演练基线）

以 2026-09-07 对本机真实旧库（55 篇文章、16 条评论、112 张图床图片、59 条设置项）的演练为基线，以下行为是**预期而非异常**：

- 55 篇 published 文章全部以 `created_at` 充当 `published_at`（旧库无发布时间列），Report 中聚合为一条 note。
- 43 个未识别设置 Key（社交信息、twikoo/local-comment 组件、图床配置、`head_content`/`footer_content` 等）进入 Archive，不进目标库。
- `sm.ms.token`、`7bu.token` 等疑似 Secret Key 只记名不记值；`邮件设置.pwd` 只写入 `setting_groups`，不进 Report/Archive。
- 4 条评论（#1/#2/#3/#13）因目标文章/页面悬挂整行进入 Archive。
- 图库分类 slug 为空时生成 `gallery-{id}`；Tag/分类空 slug 同理生成占位值并记 repaired。
- 媒体下载：旧图床 `bu.dusays.com` 已全部失效（404），演练中 112 张图片全部落 `legacy_url`。若正式迁移仍尝试下载：单文件 15s 超时 + 2 次重试，122 个文件串行最坏约 1.5 小时；赶时间用 `--skip-media-download`，事后可重跑 `migrate --only media`（幂等，已存在的行会跳过，不会重复下载已落盘文件——注意已落 `legacy_url` 的行同样跳过，如需重下需先清理对应 `media_assets` 行）。
- Archive 文件逐行记录去向（含 `whole_row` 标记区分整行归档与字段级归档），是数据完整性的最终兜底，必须随 Run 存档。
