#!/usr/bin/env bash
# 一键部署：环境检查 →（可选）构建 Admin → docker compose up -d --build → 等待就绪。
#
# 用法：
#   deploy/scripts/deploy.sh
#   deploy/scripts/deploy.sh --build-admin --admin-domain https://www.example.com
#   deploy/scripts/deploy.sh --timeout 600
#
# 说明：
#   - 必须在装有 Docker 的主机上执行（生产机或已配置远程 Docker Context 的开发机）。
#   - --build-admin 会先调用 deploy/scripts/build-admin.sh 生成 apps/admin/dist。
#   - 默认对全部服务 up -d --build；只想重建部分服务时把服务名放在 -- 之后，例如：
#       deploy/scripts/deploy.sh -- server web
#   - 可在任意目录执行，脚本自行定位仓库根。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

COMPOSE_FILE="deploy/docker-compose.yml"
ENV_FILE=".env"
TIMEOUT=300
BUILD_ADMIN=0
ADMIN_DOMAIN=""
EXTRA_SERVICES=()

usage() {
  sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --build-admin)
      BUILD_ADMIN=1
      shift
      ;;
    --admin-domain)
      ADMIN_DOMAIN="${2:?--admin-domain 需要参数}"
      shift 2
      ;;
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

if [[ ! -d apps/admin/dist ]]; then
  echo "警告：apps/admin/dist 不存在，Caddy 将提供空目录（Admin 404）。" >&2
  echo "      可先运行：deploy/scripts/build-admin.sh --domain https://<公开站域名>" >&2
fi

echo "==> 前置条件通过"

if [[ "$BUILD_ADMIN" -eq 1 ]]; then
  echo "==> 构建 Admin 静态产物"
  if [[ -n "$ADMIN_DOMAIN" ]]; then
    deploy/scripts/build-admin.sh --domain "$ADMIN_DOMAIN"
  else
    deploy/scripts/build-admin.sh
  fi
fi

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
echo "  浏览器访问 Admin 与公开站，按 docs/docker-deployment.md「首次部署」做 Smoke Test"
