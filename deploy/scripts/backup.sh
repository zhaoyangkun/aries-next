#!/usr/bin/env bash
# 每日备份：pg_dump（Custom Format）+ media Volume 打包，自动清理过期归档。
#
# 用法：
#   deploy/scripts/backup.sh                  # 备份到 ./backups，保留 14 天
#   BACKUP_DIR=/var/backups/aries KEEP_DAYS=30 deploy/scripts/backup.sh
#
# cron 示例（每天 03:00，日志随系统日志）：
#   0 3 * * * cd /opt/aries && deploy/scripts/backup.sh >> logs/backup.log 2>&1
#
# 说明：
# 只备份 pgdata（经 pg_dump 导出）与 aries_data Volume 里的 media/ 子目录；caddy_data / logs 不备份。
#   - 直接用 docker 命令而非 compose：compose 的 `:?` 插值变量要求 .env 完整，
#     备份场景不该被构建变量卡住；容器名 / Volume 名由 compose 的 name: aries 固定。
#   - 备份文件应异地复制（rclone/rsync/scp），本脚本不内置，建议在下方标记处追加。
#   - 可在任意目录执行，脚本自行定位仓库根；要求 Compose 栈处于运行状态。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

BACKUP_DIR="${BACKUP_DIR:-$ROOT/backups}"
KEEP_DAYS="${KEEP_DAYS:-14}"
STAMP="$(date +%F-%H%M%S)"
POSTGRES_CONTAINER="${POSTGRES_CONTAINER:-aries-postgres-1}"
DATA_VOLUME="${DATA_VOLUME:-aries_data}"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
fi

mkdir -p "$BACKUP_DIR"

docker inspect "$POSTGRES_CONTAINER" >/dev/null 2>&1 || {
  echo "未找到容器 $POSTGRES_CONTAINER：请先启动 Compose 栈（deploy/scripts/deploy.sh）" >&2
  exit 1
}

echo "==> 备份数据库 → $BACKUP_DIR/aries-$STAMP.dump"
docker exec -i "$POSTGRES_CONTAINER" pg_dump -U aries -Fc aries >"$BACKUP_DIR/aries-$STAMP.dump"

echo "==> 备份媒体文件 → $BACKUP_DIR/media-$STAMP.tar.gz"
docker run --rm -v "$DATA_VOLUME":/data:ro -v "$BACKUP_DIR":/backup alpine \
  sh -c "tar czf /backup/media-$STAMP.tar.gz -C /data/media ."

# 异地复制归档建议在此追加，例如：
#   rclone copy "$BACKUP_DIR" remote:aries-backup --include 'aries-*' --include 'media-*'

echo "==> 清理 ${KEEP_DAYS} 天前的归档"
find "$BACKUP_DIR" -name 'aries-*.dump' -mtime +"$KEEP_DAYS" -delete
find "$BACKUP_DIR" -name 'media-*.tar.gz' -mtime +"$KEEP_DAYS" -delete

echo "==> 备份完成，当前归档："
ls -lh "$BACKUP_DIR"
