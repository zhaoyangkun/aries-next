# Phase 03：Media Library 与生产级 Editor

> 状态：✅ 已完成　最后验证日期：2026-09-06
>
> 本文档回答：Media Library 与文章编辑器的范围、安全约束与验收标准。实际交付记录见文末 §11。

## 1. 目标

提供安全、可复用的 Media Library，并将文章 Dialog Editor 提升为支持上传、选图、自动保存、恢复和文件导入的生产工具。

## 2. 产品决策

- 使用 Storage Adapter 隔离 Local Filesystem、S3 Compatible 和未来 Provider。
- 第一批实现 Local 与 S3 Compatible；旧 sm.ms、imgbb、Tencent COS URL 原样迁移，不立即复制全部 SDK。
- Database 保存 Asset Metadata 和 Storage Key，不将 Binary 写入 PostgreSQL。
- 删除 Media 前必须检查引用；默认 Soft Delete，物理对象清理由 Background Job 完成。

## 3. Database 工作

- 新增 `media_assets`：Storage Provider、Object Key、URL、Original Name、MIME、Size、Width、Height、Hash、Alt、Status、Uploader。
- 新增 `media_usages` 或可查询的引用关系，至少覆盖 Article Cover、Article Content 和 Gallery。
- 新增 `background_jobs`，支持 Thumbnail、Metadata Probe、Import 和 Cleanup。
- Hash 用于重复文件提示，不以用户可控 File Name 作为 Object Key。

## 4. API Contract

```text
GET    /api/admin/media
POST   /api/admin/media
POST   /api/admin/media/remote
POST   /api/admin/media/batch-delete
GET    /api/admin/media/{id}
PUT    /api/admin/media/{id}
DELETE /api/admin/media/{id}
GET    /api/admin/media/{id}/usages
POST   /api/admin/articles/imports
GET    /api/admin/imports/{id}
POST   /api/admin/imports/{id}/commit
```

以上为实际端点（与原计划差异：Upload 直接挂在 `POST /api/admin/media`，并补充了批量删除）。Upload 使用明确 Size Limit，并返回稳定 Asset ID；Remote Import 必须校验 Scheme、Redirect、DNS 和响应大小，防止 SSRF。

## 5. Backend 工作

- 实现 `Storage` Trait、Local Adapter 和 S3 Compatible Adapter。
- 验证 MIME、Extension、Magic Bytes、Size 和 Image Dimension。
- 清理 File Name，生成不可预测 Object Key，阻止 Path Traversal。
- 支持 Markdown Image Reference 解析和 Media Usage 更新。
- Import Job 先 Parse Front Matter 与 Markdown，输出 Preview、Warning 和 Conflict，再由用户 Commit。
- Background Job 具备 Retry、Idempotency、Error Detail 和 Audit。

## 6. Admin 工作

- 实现 Media Library：Grid/List、Search、Type Filter、Provider Filter、Upload Progress、Metadata Edit。
- 媒体交互：点击图片 → 右侧 Sheet（详情/编辑/引用/删除）→ 点击图片缩略图 → 全屏 Dialog 大图预览（`max-w-4xl`，`max-h-[80vh]`，透明背景，关闭按钮带白色阴影）。
- 实现 `AppMediaPicker`，用于 Article Cover、Markdown Image 和 Gallery。
- Article Editor Dialog 支持拖放上传、粘贴图片、插入 Asset、Alt Text 和 Upload Progress。
- Autosave 显示 Saving、Saved、Offline、Conflict 和 Failed 状态。
- Draft Recovery 区分 Server Draft、Local Emergency Backup 和 Revision，不能自动覆盖。
- Markdown Import 使用分步 Dialog：选择文件、Preview、解决 Slug 冲突、Commit。

## 7. Test 与验证

- Unit Test：Object Key、MIME Validation、Markdown Usage Parser。
- Integration Test：Upload、Metadata、引用检查、Soft Delete、Cleanup Job。
- Security Test：伪造 MIME、超大文件、Path Traversal、SVG Script、Remote SSRF。
- E2E：上传图片 -> 选择封面 -> 插入正文 -> 保存文章 -> 重开 Editor -> Asset 仍正确显示。
- Import E2E：含 Front Matter 的 Markdown -> Preview -> Commit 为 Draft -> 人工 Publish。

## 8. Acceptance Gate

1. 管理员可以上传、搜索、选择和复用 Media Asset。
2. 被 Article 或 Gallery 引用的 Asset 不会被直接物理删除。
3. Editor 在 Network Failure 后保留 Emergency Backup，并能明确恢复来源。
4. Markdown Import 不会绕过 Draft、Sanitization、Slug Conflict 和 Audit。
5. Upload 与 Editor 的 Desktop、Mobile 核心流程通过验证。

## 9. 本 Phase 不做

- 不实现 Video Transcoding 或 Digital Asset Management Workflow。
- 不承诺兼容所有旧图床上传 API。
- 不由 AI 自动生成图片或自动替换 Cover。

## 10. 完成后更新

- Storage 配置示例与 Deployment Volume 文档
- `docs/database-mapping.md` Picture 到 Media Asset 映射
- Object Storage CORS、Backup 和 Lifecycle 说明

## 11. 交付记录

- Backend：`Storage` Trait + Local 与 S3 Compatible 两个 Adapter（`backend/infra/src/storage/`）；MIME、Magic Bytes、Size 校验与不可预测 Object Key；`media_assets`、`background_jobs` 表；Markdown 图片引用解析与 Usage 跟踪；Markdown Import 分步 Job（Parse → Preview → Commit）。
- Admin：Media Library 页（Grid、搜索、筛选、上传、详情 Sheet、大图预览）、`AppMediaPicker`、Article Editor 内拖放/粘贴上传与 Alt 编辑、Markdown Import 分步 Dialog。
- 匿名媒体文件服务：`/api/media/files/*path`；媒体清理由 `backend/server/src/worker.rs` 的 Background Job 轮询执行。
- Contract Test 位于 `backend/server/tests/media_api.rs`。

已知限制：

- Admin 的 Vditor 运行时资源默认走 unpkg CDN；自托管需把 `vditor/dist` 拷入 `public/` 并配置 `cdn` 选项。
- Desktop 与 Mobile 真机视觉检查随 Phase 04 收尾一并完成。
