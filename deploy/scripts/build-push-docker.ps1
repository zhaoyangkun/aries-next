# 构建并推送 aries-server 与 aries-web 到阿里云镜像仓库。
# 前置：已 docker login registry.cn-hangzhou.aliyuncs.com；可在任意目录执行本脚本。
# 用法：powershell -File deploy/scripts/build-push-docker.ps1 [-Tag 0.0.2]
[CmdletBinding()]
param(
    [string]$Tag = 'latest'
)

$ErrorActionPreference = 'Stop'

# 脚本位于 deploy/scripts/，切到仓库根目录，保证 Dockerfile 相对路径与构建上下文正确
Set-Location (Join-Path $PSScriptRoot '..\..')

function Build-Push([string]$Dockerfile, [string]$Image) {
    docker buildx build -f $Dockerfile `
        --build-arg USE_CN_MIRROR=1 `
        -t "${Image}:$Tag" `
        --provenance=false --sbom=false `
        --push .
    if ($LASTEXITCODE -ne 0) {
        throw "$Image 构建或推送失败（exit code $LASTEXITCODE），后续镜像已跳过。"
    }
}

Build-Push 'deploy/server.Dockerfile' 'registry.cn-hangzhou.aliyuncs.com/zhaoyangkun/aries-server'
Build-Push 'deploy/web.Dockerfile' 'registry.cn-hangzhou.aliyuncs.com/zhaoyangkun/aries-web'
Write-Host '全部镜像构建并推送完成。'
