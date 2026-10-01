# syntax=docker/dockerfile:1
# 多阶段构建 Nuxt SSR 产物；无任何构建期域名变量（构建一次、任意域名部署），
# 服务端后端地址由运行期环境变量 NUXT_INTERNAL_API_BASE 注入（见 docker-compose.yml）
FROM node:24-bookworm-slim AS builder
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
RUN pnpm install --frozen-lockfile && pnpm --filter @aries/web build

# 运行时阶段：node:24-alpine（无原生模块，纯 JS 产物可安全使用 musl 版 Node，比 bookworm-slim 小约 200MB）
FROM node:24-alpine
WORKDIR /app
ENV NITRO_HOST=0.0.0.0 \
    NITRO_PORT=3000
COPY --from=builder /build/apps/web/.output /app/.output
EXPOSE 3000
CMD ["node", ".output/server/index.mjs"]
# 仓库根目录执行（上下文必须是根，COPY . . 需要整个 pnpm workspace）
# docker build -f deploy/web.Dockerfile --progress=plain --build-arg USE_CN_MIRROR=1 -t aries-web:0.0.1 .
