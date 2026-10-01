#!/bin/sh
# 降权启动模式（postgres / mysql 官方镜像同款）：
# 容器以 root 启动本脚本，修复数据目录属主后通过 gosu 降权到 aries（uid 10001）运行主进程；
# 以非 root 直接启动（如 K8s runAsNonRoot、docker run --user）时跳过修复，原样执行。
set -e

if [ "$(id -u)" = "0" ]; then
    # 挂载点（bind mount / named volume）的属主对运行用户不可写时在这里修复；
    # 个别文件共享实现不允许容器内 chown，失败不阻断启动（best-effort）
    chown -R aries:aries /var/lib/aries 2>/dev/null || true
    exec gosu aries "$@"
fi

exec "$@"
