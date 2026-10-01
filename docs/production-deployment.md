# 生产部署手册

> 最后验证日期：2026-09-12（对照 `Cargo.toml`、`apps/admin/vite.config.ts`、`apps/web/nuxt.config.ts`、`backend/server/src/http/auth.rs`、`.env.example` 核对）

**回答什么问题**：如何在一台 Linux VPS 上从零部署 Aries Next 生产环境（Bare-metal 拓扑：构建产物、Systemd、Nginx、HTTPS、数据库初始化、备份与更新回滚）。
**写给谁**：部署与运维人员。Docker Compose 拓扑见 `docker-deployment.md`；环境变量的含义、约束与默认值见 `docker-deployment.md`（部署与安全配置的唯一权威文档），本文不重复，只给操作步骤。

## 目录

- [部署形态](#部署形态)
- [环境准备](#环境准备)
- [构建产物](#构建产物)
- [Systemd 服务](#systemd-服务)
- [Nginx 与 HTTPS](#nginx-与-https)
- [数据库初始化与首个账号](#数据库初始化与首个账号)
- [数据备份](#数据备份)
- [日志与监控](#日志与监控)
- [更新与回滚](#更新与回滚)
- [上线验收](#上线验收)

## 部署形态

单 VPS 即可承载全部组件，进程间通过本机回环通信：

```text
Internet ──HTTPS──> Nginx ──┬── /              -> aries-web   (Nuxt SSR, 127.0.0.1:3000)
                            ├── admin 站点      -> admin dist  (静态文件) + /api -> aries-server
                            └── (可选) api 子域 -> aries-server (Axum, 127.0.0.1:8088)
                                    │
                                    └──> PostgreSQL 17（本机或独立实例）
```

三个进程：

| 进程 | 产物 | 监听 | 说明 |
| --- | --- | --- | --- |
| `aries-server` | `cargo build --release` 出的单 Binary | `0.0.0.0:8088` | 启动时自动执行 SQLx Migration，含后台 Worker |
| `aries-web` | Nuxt SSR（nitro `node-server`） | `127.0.0.1:3000` | 仅由 Nginx 访问，不暴露公网 |
| admin | Vue SPA 静态文件 | 无 | 由 Nginx 直接服务，API 请求同源转发到 `aries-server` |

推荐 Admin 与 API 同站点部署（`https://admin.example.com` 同时服务静态文件和 `/api/*`），这样 `VITE_API_BASE_URL` 留空即可走同源，也无需为 Admin 单独配 CORS Origin。

## 环境准备

以 Ubuntu 24.04 为例（其他发行版类比）：

```bash
# 1. 系统依赖：Rust（rustup，≥ 1.85）、Node.js 24、pnpm 11、PostgreSQL 17
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
curl -fsSL https://deb.nodesource.com/setup_24.x | sudo -E bash -
sudo apt install -y nodejs postgresql-17 nginx build-essential pkg-config libssl-dev
sudo corepack enable && sudo corepack prepare pnpm@11.9.0 --activate

# 2. 运行用户与目录（不使用 root 跑业务进程）
sudo useradd --system --create-home --shell /usr/sbin/nologin aries
sudo mkdir -p /opt/aries/{server,web,admin} /var/lib/aries/media /var/log/aries
sudo chown -R aries:aries /opt/aries /var/lib/aries /var/log/aries

# 3. 防火墙只放行 22/80/443
sudo ufw allow OpenSSH && sudo ufw allow 'Nginx Full' && sudo ufw enable
```

PostgreSQL 侧：创建业务库与账号，预装 `citext` 扩展（Schema 由 Backend 启动时自动创建，无需手工建表）：

```sql
CREATE USER aries WITH PASSWORD '<strong-password>';
CREATE DATABASE aries OWNER aries;
\c aries
CREATE EXTENSION IF NOT EXISTS citext WITH SCHEMA public;
```

## 构建产物

在开发机或 CI 上构建，只把产物传到服务器（服务器上无需 Rust/Node 工具链）。三个产物相互独立，可分别发布。

```bash
# 1. Backend（Binary 路径 target/release/aries-server）
cargo build --release -p aries-server

# 2. Admin 静态文件（输出 apps/admin/dist/）
#    同站点部署时 VITE_API_BASE_URL 留空，API 走同源 /api/admin；
#    产物不含域名，「打开公开站」入口运行期读站点设置 site_url
pnpm --filter @aries/admin build

# 3. Web SSR（输出 apps/web/.output/，整个目录上传）
#    构建期无域名变量；服务端后端地址由运行期环境变量 NUXT_INTERNAL_API_BASE 决定
pnpm --filter @aries/web build
```

> 三个产物均不含构建期固化的域名：浏览器端请求走同源 `/api`（Nginx 转发），Web 服务端经 `NUXT_INTERNAL_API_BASE` 直连后端，SEO 绝对地址取站点设置 `site_url`——改域名只改 Nginx 配置与站点设置，无需重新构建。

上传到服务器：

```bash
scp target/release/aries-server            aries@server:/opt/aries/server/
scp -r apps/admin/dist/.                   aries@server:/opt/aries/admin/
scp -r apps/web/.output/.                  aries@server:/opt/aries/web/
```

## Systemd 服务

两个服务的配置项不同：Rust 环境变量集中在 `EnvironmentFile`；Node 侧只有监听地址，后端地址由 `NUXT_INTERNAL_API_BASE` 环境变量在运行期注入（默认回退 `http://localhost:8088/api/public`，同机部署可不设）。

`/etc/systemd/system/aries-server.service`：

```ini
[Unit]
Description=Aries Next API Server
After=network.target postgresql.service

[Service]
User=aries
WorkingDirectory=/opt/aries/server
EnvironmentFile=/opt/aries/server/.env
ExecStart=/opt/aries/server/aries-server
Restart=on-failure
RestartSec=3
# 生产关键变量见 docker-deployment.md「环境变量配置」；.env 权限必须 600，属主 aries
# APP_ENV=production / DATABASE_* / BOOTSTRAP_SECRET / ADMIN_ORIGINS /
# SESSION_COOKIE_SECURE=true / MEDIA_LOCAL_DIR=/var/lib/aries/media / LOG_DIR=/var/lib/aries/logs

[Install]
WantedBy=multi-user.target
```

`/etc/systemd/system/aries-web.service`：

```ini
[Unit]
Description=Aries Next Public Web (Nuxt SSR)
After=network.target

[Service]
User=aries
WorkingDirectory=/opt/aries/web
Environment=NITRO_HOST=127.0.0.1
Environment=NITRO_PORT=3000
ExecStart=/usr/bin/node /opt/aries/web/server/index.mjs
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now aries-server aries-web
systemctl status aries-server aries-web    # 两者均应为 active (running)
curl -fsS http://127.0.0.1:8088/api/health/ready   # 依赖就绪
curl -fsS http://127.0.0.1:3000/                   # SSR 首页
```

## Nginx 与 HTTPS

公开站与 Admin 两个 Server Block，HTTP 统一重定向到 HTTPS：

```nginx
# /etc/nginx/sites-available/aries-web
server {
    listen 80;
    server_name www.example.com;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl;
    server_name www.example.com;

    # certbot 会自动补全 ssl_certificate 两行
    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

```nginx
# /etc/nginx/sites-available/aries-admin
server {
    listen 80;
    server_name admin.example.com;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl;
    server_name admin.example.com;

    root /opt/aries/admin;
    index index.html;

    # 上传大小上限，须与后端媒体限制匹配（默认图片 5MB / 文件 2MB，留余量）
    client_max_body_size 10m;

    # Admin 静态文件 + SPA Fallback
    location / {
        try_files $uri $uri/ /index.html;
    }

    # API 同源转发；必须保留 Origin 头（CSRF 防线依赖它），不要覆盖
    location /api/ {
        proxy_pass http://127.0.0.1:8088;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    gzip on;
    gzip_types text/css application/javascript application/json image/svg+xml;
}
```

签发证书并启用：

```bash
sudo ln -s /etc/nginx/sites-available/aries-web /etc/nginx/sites-enabled/
sudo ln -s /etc/nginx/sites-available/aries-admin /etc/nginx/sites-enabled/
sudo certbot --nginx -d www.example.com -d admin.example.com
sudo nginx -t && sudo systemctl reload nginx
```

安全响应头（`X-Content-Type-Options` 等）已由 Backend 与 Nuxt 内置；如需在 Nginx 层补 HSTS，见 `docker-deployment.md` 的「安全加固」一节（仅全站 HTTPS 就绪后开启）。

## 数据库初始化与首个账号

Schema 与 Migration 由 `aries-server` 首次启动自动完成。首个 Owner 通过一次性 Bootstrap 接口创建（需 `BOOTSTRAP_SECRET`，接口带限流）：

```bash
# 确认未初始化
curl -fsS https://admin.example.com/api/admin/auth/bootstrap/status

# 创建首个 Owner；已有 User 后该接口永久拒绝再次调用
curl -fsS -X POST https://admin.example.com/api/admin/auth/bootstrap \
  -H 'Content-Type: application/json' \
  -d '{"bootstrap_secret":"<BOOTSTRAP_SECRET>","login":"admin","password":"<strong-password>","display_name":"Admin"}'
```

字段名以 `docs/openapi.yaml` 为准。创建成功后立即登录 Admin 确认，并在「系统 → 设置」中配置站点信息与 AI / Email 集成（存于数据库，非环境变量）。

## 数据备份

最小方案：每日 `pg_dump` + 媒体目录归档，异地保存。

```bash
# /opt/aries/backup.sh（crontab: 0 3 * * * /opt/aries/backup.sh）
#!/usr/bin/env bash
set -euo pipefail
DEST=/var/backups/aries
mkdir -p "$DEST"
pg_dump -h 127.0.0.1 -U aries -Fc aries > "$DEST/aries-$(date +%F).dump"
tar czf "$DEST/media-$(date +%F).tar.gz" -C /var/lib/aries media
# 异地复制（rsync 到另一台机器 / 对象存储），并清理 30 天前本地归档
find "$DEST" -name '*.dump' -mtime +30 -delete
find "$DEST" -name '*.tar.gz' -mtime +30 -delete
```

- PostgreSQL 数据库是唯一的结构化数据源（含 `setting_groups` 中的集成配置），媒体文件按 `MEDIA_PROVIDER` 落在本地目录或 S3，两者都要备份。
- 备份的价值取决于恢复是否演练过：每季度按 `migration-runbook.md` 在一台干净机器上恢复一次并验证。

## 日志与监控

- Backend 文件日志在 `LOG_DIR`（按天切分，保留 `LOG_RETENTION_DAYS` 天），stdout 同时进 journald：`journalctl -u aries-server -f`。
- Web SSR 日志同样由 journald 收集：`journalctl -u aries-web -f`。
- 探活端点：`/api/health/live`（进程）、`/api/health/ready`（含数据库）。监控只探公网入口 `https://www.example.com/api/health/ready` 即可覆盖全链路。
- 排障时用响应头 `x-request-id` 串联 Backend 日志。

## 更新与回滚

发布新版本（二进制与前端产物独立发布，互不阻塞）：

```bash
# 1. 构建并上传新产物（步骤同「构建产物」）
# 2. Backend：Migration 随启动自动执行
sudo systemctl restart aries-server
# 3. Web：替换 .output 后重启；Admin：替换 dist 后无需重启
sudo systemctl restart aries-web
# 4. 验证探活与关键路径（登录、发文章、公开页、媒体访问）
```

回滚注意：**Migration 只前进不后退**。若新版 Migration 已执行后需要回退 Binary，必须先人工评估新旧代码与 Schema 的兼容性（通常是安全的：新增列/索引旧代码忽略），不兼容时按 `migration-runbook.md` 手工编写逆向 SQL。因此「先备份、再升级」是硬前提。

## 上线验收

按 `docker-deployment.md` 的「上线验收清单」逐项执行（HTTPS、Cookie 属性、Origin 校验、Session 撤销、探活、媒体持久化、日志、Secret 不泄漏），全部通过后方可切换 DNS。
