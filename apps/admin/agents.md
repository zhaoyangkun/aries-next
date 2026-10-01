# AGENTS.md

本目录是基于 shadcn-vue-admin 模板重建的 Aries 管理端。项目级规范见仓库根目录 `AGENTS.md`，优先以它为准。

## 本包特有约定

- 页面在 `src/pages/` 下由 vue-router 5 文件路由生成；页面 meta 用 `<route lang="yaml">` 声明（`public: true` 表示免登录，`permission` 对接后端权限）。
- 布局在 `src/layouts/`（default / blank），由 vite-plugin-vue-layouts 装配。
- `vue` API、`src/composables`、`src/stores`、`src/components` 由 unplugin 自动导入；业务 API 层（`src/modules/**/api`、`src/shared/api`，axios）保持显式 import。
- i18n 只保留简体中文（`src/plugins/i18n/zh.json`）；新文案直接写中文，不必进语言包。
- 主题预设/圆角/内容布局走 `src/stores/theme.ts`（tweakcn 体系）。

## 命令

- `pnpm dev` / `pnpm build` / `pnpm typecheck` / `pnpm test`（vitest run）
- 合并前以仓库根目录的全量验证命令为准（见根 `AGENTS.md`）。
