docker buildx build -f deploy/server.Dockerfile \
  --build-arg USE_CN_MIRROR=1 \
  -t registry.cn-hangzhou.aliyuncs.com/zhaoyangkun/aries-server:latest \
  --provenance=false --sbom=false \
  --push .
# docker buildx build -f deploy/server.Dockerfile `
#    --build-arg USE_CN_MIRROR=1 `
#    -t registry.cn-hangzhou.aliyuncs.com/zhaoyangkun/aries-server:latest `
#    --provenance=false --sbom=false `
#    --push .

docker buildx build -f deploy/web.Dockerfile \
  --build-arg USE_CN_MIRROR=1 \
  -t registry.cn-hangzhou.aliyuncs.com/zhaoyangkun/aries-web:latest \
  --provenance=false --sbom=false \
  --push .
# docker buildx build -f deploy/web.Dockerfile `
#   --build-arg USE_CN_MIRROR=1 `
#   -t registry.cn-hangzhou.aliyuncs.com/zhaoyangkun/aries-web:latest `
#   --provenance=false --sbom=false `
#   --push .
