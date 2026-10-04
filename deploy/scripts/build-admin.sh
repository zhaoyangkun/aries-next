#!/usr/bin/env bash
# 构建 Admin SPA 静态产物（apps/admin/dist，由 aries-server 通过 include_dir 编入 Binary，
# 运行时挂载在 /admin/ 下直出，无需单独域名或静态目录）。
#
# 用法：
#   deploy/scripts/build-admin.sh
#   deploy/scripts/build-admin.sh --api-base https://api.example.com
#
# 说明：
#   - 构建固定带 --base=/admin/，与服务端挂载路径一致；vite dev（pnpm dev:admin）不受 base 影响。
#   - Admin 构建产物不含任何域名：「打开公开站」入口运行期从站点设置 site_url 读取，
#     构建期无需传域名；--api-base 仅在 API 与 Admin 不同源部署时才需要
#     （写入 apps/admin/.env 的 VITE_API_BASE_URL，该文件已 Git 忽略）。
#   - 构建后需重新编译 aries-server（cargo build -p aries-server）产物才会更新。
#   - 可在任意目录执行，脚本自行定位仓库根。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

ADMIN_ENV="apps/admin/.env"
API_BASE=""

usage() {
  sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --api-base)
      API_BASE="${2:?--api-base 需要参数}"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "未知参数：$1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

command -v pnpm >/dev/null 2>&1 || {
  echo "未找到 pnpm：请先安装 Node.js 24+ 并 corepack enable" >&2
  exit 1
}

if [[ -n "$API_BASE" ]]; then
  printf 'VITE_API_BASE_URL=%s\n' "$API_BASE" >"$ADMIN_ENV"
  echo "已写入 $ADMIN_ENV"
fi

pnpm install --frozen-lockfile
pnpm --filter @aries/admin build:embedded

echo
echo "构建完成：$ROOT/apps/admin/dist（挂载路径 /admin/）"
echo "生效：重新编译服务端（cargo build -p aries-server）后访问 https://<站点域名>/admin/"
