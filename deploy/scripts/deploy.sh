#!/usr/bin/env bash
# 一键部署：环境检查 → docker compose up -d --build → 等待就绪。
# Admin SPA 由 server 镜像构建阶段编入 Binary（/admin/ 直出），无需单独构建产物。
#
# 用法：
#   deploy/scripts/deploy.sh
#   deploy/scripts/deploy.sh --timeout 600
#   deploy/scripts/deploy.sh -- server web
#
# 说明：
#   - 必须在装有 Docker 的主机上执行（生产机或已配置远程 Docker Context 的开发机）。
#   - 默认对全部服务 up -d --build；只想重建部分服务时把服务名放在 -- 之后。
#   - 本地调试 Admin 用 pnpm dev:admin；把新构建的 dist 编入本地二进制用
#     deploy/scripts/build-admin.sh && cargo build -p aries-server。
#   - 可在任意目录执行，脚本自行定位仓库根。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

COMPOSE_FILE="deploy/docker-compose.yml"
ENV_FILE=".env"
TIMEOUT=300
EXTRA_SERVICES=()

usage() {
  sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --timeout)
      TIMEOUT="${2:?--timeout 需要参数（秒）}"
      shift 2
      ;;
    --)
      shift
      EXTRA_SERVICES=("$@")
      break
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

compose() {
  docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" "$@"
}

require_env_var() {
  local key="$1"
  if ! grep -q "^${key}=.\+" "$ENV_FILE" 2>/dev/null; then
    echo ".env 缺少 ${key}（或值为空）：请按 docs/docker-deployment.md「环境变量配置」一节补齐" >&2
    exit 1
  fi
}

wait_state() {
  local container="$1" want="$2" service="$3"
  local deadline=$((SECONDS + TIMEOUT))
  until [[ "$(docker inspect -f '{{.State.Status}}' "$container" 2>/dev/null || true)" == "$want" ]]; do
    if ((SECONDS > deadline)); then
      echo "等待 $container 变为 $want 超时（${TIMEOUT}s），最近日志：" >&2
      compose logs --tail=30 "$service" >&2 || true
      exit 1
    fi
    sleep 3
  done
}

wait_healthy() {
  local container="$1" service="$2"
  local deadline=$((SECONDS + TIMEOUT))
  until [[ "$(docker inspect -f '{{.State.Health.Status}}' "$container" 2>/dev/null || true)" == "healthy" ]]; do
    if ((SECONDS > deadline)); then
      echo "等待 $container healthy 超时（${TIMEOUT}s），最近日志：" >&2
      compose logs --tail=30 "$service" >&2 || true
      exit 1
    fi
    sleep 3
  done
}

echo "==> 检查前置条件"

[[ -f "$ENV_FILE" ]] || {
  echo "未找到 $ENV_FILE：请先按 docs/docker-deployment.md「环境变量配置」创建" >&2
  exit 1
}
[[ -f "$COMPOSE_FILE" ]] || {
  echo "未找到 $COMPOSE_FILE" >&2
  exit 1
}

docker compose version >/dev/null 2>&1 || {
  echo "未找到 docker compose plugin" >&2
  exit 1
}

require_env_var "DATABASE_PASSWORD"
require_env_var "BOOTSTRAP_SECRET"
require_env_var "ADMIN_ORIGINS"

echo "==> 前置条件通过"

echo "==> 构建并启动服务：docker compose up -d --build ${EXTRA_SERVICES[*]:-全部服务}"
compose up -d --build ${EXTRA_SERVICES[@]+"${EXTRA_SERVICES[@]}"}

echo "==> 等待服务就绪（超时 ${TIMEOUT}s）"
wait_healthy aries-postgres-1 postgres
wait_state aries-server-1 running server
wait_state aries-web-1 running web
wait_state aries-caddy-1 running caddy

compose ps

ADMIN_ORIGIN="$(grep '^ADMIN_ORIGINS=' "$ENV_FILE" | head -1 | cut -d= -f2- | cut -d, -f1)"
echo
echo "部署完成。验收："
echo "  curl -fsS ${ADMIN_ORIGIN}/api/health/ready"
echo "  浏览器访问 ${ADMIN_ORIGIN}/admin/ 与公开站，按 docs/docker-deployment.md「首次部署」做 Smoke Test"
