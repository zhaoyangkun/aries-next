# Phase 06：扩展内容与外观设置

> 状态：✅ 已完成　最后验证日期：2026-09-06
>
> 本文档回答：自定义页面、日志、图库、友链、导航与外观设置的范围与验收标准。与原始计划的差异以文末 §12 为准。

## 1. 目标

补齐旧 Aries 的自定义页面、日志、图库、友情链接、导航、网站设置与外观能力，同时避免迁移旧主题中的可执行 Template 和任意 Script。

## 2. Domain 范围

- Page：Markdown 自定义页面。
- Journal：短内容 Timeline，可公开或私密。
- Gallery 与 Gallery Category：基于 Media Asset。
- Link 与 Link Category：友情链接。
- Navigation：两级菜单、排序、打开方式和可见性。
- Site Setting、Social Link、Appearance Setting 和 Integration Setting。

## 3. Database 工作

- 新增 `pages`、`journals`、`galleries`、`gallery_items`、`links`、`navigation_items`。
- Category 继续复用 Kind，但为不同 Kind 建立明确 Validation 和 Delete Constraint。
- 新增有类型的 Setting 表或按 Group 约束的 JSONB，保存 Version 与 Updated By。
- Secret Setting 与 Public Setting 分离；Secret 只保存加密引用或由 Environment 提供。
- Navigation 内部链接优先保存 Target Type 与 Target ID，外部链接保存 URL。

## 4. Admin API

```text
/api/admin/pages
/api/admin/journals
/api/admin/galleries
/api/admin/links
/api/admin/navigation
/api/admin/settings/site
/api/admin/settings/appearance
/api/admin/settings/email
/api/admin/settings/integrations
```

以上资源遵循标准 CRUD、Pagination、Optimistic Lock、Audit 和引用检查规则。Navigation Reorder 使用一次原子批量操作，不连续发送 Move Up/Down 请求。

## 5. Public API 与页面

```text
GET /api/public/pages/{slug}
GET /api/public/journals
GET /api/public/galleries
GET /api/public/links
GET /api/public/navigation
```

Nuxt 增加：

- `/pages/{slug}`
- `/journals`
- `/galleries`
- `/links`

所有 Public Content 进入 Sitemap、Canonical 和 Metadata 规则；Private Journal 不得进入 API、Sitemap、RSS 或 Search。

## 6. 外观策略

旧主题 `xue` 与 `boundless-ui` 只作为视觉与内容字段参考，不迁移 Go Template、JavaScript 或远程静态根路径。

Aries Next 使用：

- 共享 Brand Token：Color、Typography、Radius 和 Spacing。
- Public Web Layout Preset：Header、Article Width、Sidebar、Footer 和 Code Theme。
- 可控 Appearance Setting：Logo、Favicon、Light/Dark Preference、Article List Density。
- 受控 Integration Slot：Analytics ID、Verification Token 等结构化配置。

默认禁止任意 `head_content`、`footer_content` 和 Theme Script 注入。确有需求时使用经过评审的 Integration Adapter。

## 7. Admin 工作

- Page 使用与 Article 一致的 Editor 基础能力，但使用独立 Domain API。
- Journal 使用快速创建 Dialog，支持公开/私密状态。
- Gallery 使用 Media Picker、排序和批量编辑 Alt/Location。
- Link 管理支持 Category、URL Validation、Icon、Description 和状态。
- Navigation 使用 Tree/Reorder UI，并提供 Desktop/Mobile Preview。
- Setting 使用分组 Form、Dirty State、Validation 和 Save Result，不使用无限制 Key/Value Editor。
- Appearance 提供实时 Preview，但最终值由 Server 保存并验证。

## 8. Test 与验证

- Integration Test：各 Content CRUD、Visibility、Slug、Reference 和 Reorder Transaction。
- Security Test：Private Journal Leakage、Unsafe URL、Setting Secret Mask、Script Injection。
- E2E：创建 Page、发布 Journal、建立 Gallery、添加 Link、调整 Navigation、修改 Logo。
- SEO Test：Page、Journal、Gallery、Link 的 Canonical 与 Sitemap 行为。
- Responsive Test：Navigation、Gallery Grid、Long Page 和 Empty State。

## 9. Acceptance Gate

1. 旧版 Page、Journal、Gallery、Link 和 Navigation 能映射到明确的新 Domain。
2. Admin 可以完成各扩展内容的日常 CRUD 和排序。
3. Public Web 能展示全部公开内容，并正确排除私密内容。
4. Appearance Setting 不允许运行未审核 Template 或 Script。
5. 扩展内容和设置 E2E 全部通过。

## 10. 本 Phase 不做

- 不提供在线编辑 Vue Component、Nuxt Theme 或 Server Template。
- 不实现 Theme Marketplace。
- 不执行外链自动抓取 Logo，除非增加 SSRF 防护和明确 Job。

## 11. 完成后更新

- `docs/database-mapping.md` 全部待补充业务表
- Public Route 与 Sitemap 文档
- Appearance Token 和 Integration Allowlist 文档

## 12. 实施偏差与状态

以下为对照代码确认的实际实现状态，与上文规划的差异以本节为准：

- ✅ 后台管理、公开只读 API 与 Nuxt 展示端页面均已完成：Admin 页面覆盖 Pages、Journals、Galleries（含条目与图库分类）、Links（含友链分类）、Navigation 与 Settings 分组；Nuxt 页面为 `/custom/{slug}`、`/journals`、`/galleries`、`/galleries/{slug}`、`/links`（注意自定义页的 Web 路由是 `/custom/{slug}`，不是本节 §5 规划的 `/pages/{slug}`；Public API 仍是 `/api/public/pages/{slug}`）。
- Site 设置沿用 Phase 03 的既有 `/api/admin/site-settings`（`site_settings` 单行表），未新建 `/api/admin/settings/site`；`/api/admin/settings/{group}` 覆盖 `appearance` / `email` / `integrations` / `ai` 四组（`setting_groups` 表，`payload jsonb` + `version` 乐观锁，冲突返回 409 `SETTING_VERSION_CONFLICT`；`ai` 组在 Phase 08 加入）。
- email 组的 `smtp_password` 为 write-only：GET 只回 `smtp_password_set`；PUT 中字段省略或 `null` 表示保持不变、空字符串表示清除、非空字符串表示更新。
- Public API（`/api/public/pages/{slug}`、`/api/public/journals`、`/api/public/galleries`、`/api/public/galleries/{slug}`、`/api/public/photos`、`/api/public/links`、`/api/public/navigation`）已就绪并带 Cache-Control（见 `backend/server/src/http/public/extended.rs`）；Sitemap 已纳入 `/journals`、`/links`、`/galleries` 及图库详情页，自定义页暂未进入 Sitemap（`apps/web/server/routes/sitemap.xml.ts`）。
- `links.category_id` 的 FK 为 `ON DELETE SET NULL`：友链是软删除，`RESTRICT` 会让回收站中的友链永远阻塞分类的物理删除；是否存在未删除引用由应用层在删除分类前检查，存在时返回 409 `TAXONOMY_IN_USE`。
- Navigation 删除为物理删除；仍含子节点的节点禁止删除（`parent_id` 为 `ON DELETE RESTRICT`，冲突返回 409 `NAVIGATION_CONFLICT`），最多两级由应用层校验（400 `INVALID_NAVIGATION_HIERARCHY`）。
- 管理端分类 DTO（`CategoryResponse`）含 `kind` 字段（`article` / `link` / `gallery`）；`/api/admin/categories` 保持只返回 `kind = article`，友链与图库分类分别由 `/api/admin/links/categories`、`/api/admin/galleries/categories` 管理。
- Contract Test：`backend/server/tests/pages_api.rs`、`journals_api.rs`、`galleries_api.rs`、`links_api.rs`、`navigation_api.rs`、`settings_groups_api.rs`。
