# syntax=docker/dockerfile:1
# Admin SPA 构建阶段：产物编入 aries-server Binary（include_dir），
# 运行时由 Axum 在 /admin/ 下从内存直出，无需单独域名/静态目录
FROM node:24-bookworm-slim AS admin-builder
# 国内构建加速（可选）：docker build --build-arg USE_CN_MIRROR=1 ...
# 切换 corepack 下载源与 npm registry 到 npmmirror；不传或为空则保持官方源，构建行为不变
ARG USE_CN_MIRROR=""
RUN if [ -n "$USE_CN_MIRROR" ]; then \
      npm config set registry https://registry.npmmirror.com; \
      export COREPACK_NPM_REGISTRY=https://registry.npmmirror.com; \
    fi \
 && corepack enable && corepack prepare pnpm@11.9.0 --activate
WORKDIR /build
COPY . .
# base=/admin/：产物由服务端挂在 /admin/ 下，资源引用必须带该前缀
RUN pnpm install --frozen-lockfile && pnpm --filter @aries/admin build:embedded

# 构建整个 Workspace 后仅保留 aries-server Binary
FROM rust:bookworm AS builder
# 国内构建加速（可选）：docker build --build-arg USE_CN_MIRROR=1 ...
# 仅切换 crates sparse 索引镜像；不传或为空则保持官方源，构建行为不变
ARG USE_CN_MIRROR=""
RUN if [ -n "$USE_CN_MIRROR" ]; then \
      printf '[source.crates-io]\nreplace-with = "rsproxy"\n[source.rsproxy]\nregistry = "sparse+https://rsproxy.cn/index/"\n' > "$CARGO_HOME/config.toml"; \
    fi
WORKDIR /build
COPY . .
# Admin SPA 产物（来自 admin-builder 阶段）编入 binary：include_dir 在编译期读取该目录，
# 必须位于 cargo build 之前；本地构建则直接复用仓库里已构建的 apps/admin/dist
COPY --from=admin-builder /build/apps/admin/dist ./apps/admin/dist
# rust:bookworm 镜像预装 latest stable 工具链，直接用于构建（仓库不再带 rust-toolchain.toml：
# channel=stable 会被 rustup 当成工具链别名再下载一遍，国内构建会卡在 Downloading components）。
# 只构建 Server 及依赖
RUN cargo build --release -p aries-server

FROM debian:bookworm-slim
# 与 builder 相同开关：切换 Debian 软件源镜像（仅影响 ca-certificates 安装）
ARG USE_CN_MIRROR=""
RUN if [ -n "$USE_CN_MIRROR" ]; then \
      sed -i 's|deb.debian.org|mirrors.ustc.edu.cn|g' /etc/apt/sources.list /etc/apt/sources.list.d/debian.sources 2>/dev/null || true; \
    fi \
 && apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates gosu \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --create-home aries \
 && mkdir -p /var/lib/aries/media /var/lib/aries/logs \
 && chown -R aries:aries /var/lib/aries
COPY --from=builder /build/target/release/aries-server /usr/local/bin/aries-server
COPY deploy/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh
# 独立运行（非 Compose）时的合理默认：对应上面预建并 chown 过的子目录；
# Compose 的 environment: 同名变量优先，覆盖行为不变
ENV MEDIA_LOCAL_DIR=/var/lib/aries/media \
    LOG_DIR=/var/lib/aries/logs
WORKDIR /var/lib/aries
# 以 root 启动 entrypoint：修复数据目录属主后降权到 aries 运行（postgres 官方镜像同款模式），
# bind mount / named volume 场景下都不再需要手工处理属主
ENTRYPOINT ["docker-entrypoint.sh"]
EXPOSE 8088
CMD ["aries-server"]
# 仓库根目录执行；首次编译 10–40 分钟，建议加 --progress=plain 看输出
# docker build -f deploy/server.Dockerfile --progress=plain --build-arg USE_CN_MIRROR=1 -t aries-server:0.0.1 .
# docker tag aries-server:0.0.2 registry.cn-hangzhou.aliyuncs.com/zhaoyangkun/aries-server:latest
# docker push registry.cn-hangzhou.aliyuncs.com/zhaoyangkun/aries-server:latest