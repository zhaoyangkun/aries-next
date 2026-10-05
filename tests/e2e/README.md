# @aries/e2e — Playwright E2E 测试

面向真实运行环境的端到端测试：Admin SPA（Vue 3）与公开站（Nuxt 4 SSR）通过浏览器驱动，
覆盖跨端回归场景。定位为 **opt-in 的手工 / 预发布验证**，默认不挂入根 `pnpm test` 与 CI。

## 前置条件

以下服务必须先手动启动（Playwright 配置**不会**自动拉起任何服务）：

1. PostgreSQL（`.env` 中 `DATABASE_*` 指向的实例，默认本机 5433）；
2. Backend：`cargo run -p aries-server`（默认监听 `0.0.0.0:8088`）；
3. Admin：`pnpm dev:admin`（<http://127.0.0.1:5173>，`/api` 代理到 8088）；
4. 公开站：`pnpm dev:web`（<http://127.0.0.1:3000>）；
5. 数据库中已存在可用的管理员账号（首个 Owner 通过 Admin「初始化系统」页 + `BOOTSTRAP_SECRET` 创建，
   见根 `README.md` / `AGENTS.md`）。

首次使用还需安装 Playwright 浏览器（仓库的 pnpm `allowBuilds` 未放行 postinstall，
浏览器不会随 `pnpm install` 自动下载）：

```powershell
pnpm --filter @aries/e2e exec playwright install chromium
```

## 运行

```powershell
# 仓库根目录
pnpm install

# 必备：登录凭据只走环境变量，禁止写进代码或提交 Git
$env:E2E_ADMIN_USERNAME = "owner"
$env:E2E_ADMIN_PASSWORD = "your-password"

pnpm test:e2e                       # = pnpm --filter @aries/e2e test
pnpm --filter @aries/e2e exec playwright test --list   # 只列出用例，校验配置
```

### 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `E2E_ADMIN_USERNAME` | （无，必填） | Admin 登录用户名或邮箱；缺失时登录场景 skip |
| `E2E_ADMIN_PASSWORD` | （无，必填） | Admin 登录密码；缺失时登录场景 skip |
| `E2E_ADMIN_BASE_URL` | `http://127.0.0.1:5173` | Admin SPA 地址，可指向已运行的其他实例 |
| `E2E_WEB_BASE_URL` | `http://127.0.0.1:3000` | 公开站地址，同上 |

## Project 与场景

- `admin` project（`tests/admin/`）：baseURL 指向 Admin SPA。
  - `article-reorder.spec.ts`：**核心回归场景**（对应文章手动排序的真实 Bug——Admin 调整
    顺序后公开站不同步）。流程：登录 → 文章列表切到「按排序（可拖拽）」→ 找一对相邻、
    未置顶且已发布的文章，用「下移」箭头交换 → 打开公开站首页断言两者相对顺序反转 →
    最后把文章移回原位，保证用例可重复执行。
  - 断言宽松健壮：未设置凭据、没有可用文章对、被交换文章不在公开站首页第一页时均
    `test.skip`，不误报失败。
  - 该场景会真实改写 `sort_order`（并在结尾恢复），因此配置固定 `workers: 1` 串行执行；
    **不要对生产数据库运行**。
- `web` project（`tests/web/`）：baseURL 指向公开站。
  - `public-smoke.spec.ts`：首页 SSR 可访问且文章列表区块渲染（无文章时空态也算通过）。

## 已知限制

- 页面普遍**没有 `data-testid`**，选择器依赖 label 关联的 `id`（登录页 `#login` / `#password`）、
  原生 `select[aria-label=...]`（排序字段）与按钮 `aria-label`（`上移《标题》` / `下移《标题》`）。
  文案或 aria-label 变更会导致用例失效，后续可考虑为关键交互补 `data-testid`。
- 跨端顺序断言只覆盖公开站首页**第一页**；文章数超过一页时被交换文章可能落在后续页，
  此时用例 skip 而非失败。
- 行内「置顶」「已发布」状态靠文本匹配判断，文章标题本身包含这两个词时可能误判
  （概率极低，出现后会在 skip / 断言中暴露）。

## 后续接入 CI 的建议

当前有意保持 opt-in（依赖 PostgreSQL + 三个长驻进程，服务依赖重）。如要接入：

1. 在 `.github/workflows/ci.yml` 新增独立 `e2e` job（不要塞进现有 `frontend` job），
   复用 `rust` job 的 `postgres` service 与 `DATABASE_*` / `BOOTSTRAP_SECRET` 环境变量；
2. `cargo run -p aries-server` 后台启动并等待 `/api/health/ready` 就绪，
   `pnpm dev:admin` / `pnpm dev:web` 后台启动并分别等待 5173 / 3000 端口
   （或用 `pnpm build` + 预览进程减少 dev server 抖动）；
3. 调用 `/api/admin/bootstrap` 创建 Owner（参考 `backend/server/tests/common/mod.rs`
   的 `bootstrap_owner`），把凭据注入 `E2E_ADMIN_USERNAME` / `E2E_ADMIN_PASSWORD`；
4. `pnpm --filter @aries/e2e exec playwright install --with-deps chromium` 后执行
   `pnpm test:e2e`，失败时上传 `tests/e2e/test-results/` 的 trace 与截图作为 Artifact；
5. 注意 E2E 使用与单测相同的 `public` Schema，与 `cargo test` 并行运行时互不影响
   （单测走随机 Schema 隔离），但同 job 内 E2E 用例之间仍需保持串行。
