# 控制 target/ 目录无限增长：incremental（增量编译缓存）只增不减，是体积膨胀的主要来源。
# 本脚本在缓存超过阈值时删除它（cargo 会在下次编译时自动重建，不影响正确性）。
#
# 手动运行：  powershell -File scripts/clean-target.ps1
# 自动运行：  挂到任务计划程序（登录时 / 每天），例如：
#   $action  = New-ScheduledTaskAction -Execute 'powershell.exe' `
#                -Argument '-NoProfile -ExecutionPolicy Bypass -File "E:\projects\go\web\aries-next\scripts\clean-target.ps1"'
#   $trigger = New-ScheduledTaskTrigger -AtLogOn
#   Register-ScheduledTask -TaskName 'Aries Clean Target Incremental' -Action $action -Trigger $trigger
[CmdletBinding()]
param(
    # 增量缓存超过该阈值（GB）才清理，避免删了又马上重建的无效抖动
    [double]$ThresholdGb = 2,
    # 仓库根目录（默认取本脚本的上级目录）
    [string]$RepoRoot = ''
)

$ErrorActionPreference = 'Stop'

if (-not $RepoRoot) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}
$targetDir = Join-Path $RepoRoot 'target'
$incremental = Join-Path $targetDir 'debug\incremental'

if (-not (Test-Path $incremental)) {
    Write-Host "无增量缓存（$incremental），无需清理。"
    exit 0
}

$sizeBytes = (Get-ChildItem $incremental -Recurse -File | Measure-Object -Property Length -Sum).Sum
$sizeGb = [math]::Round($sizeBytes / 1GB, 2)

if ($sizeGb -lt $ThresholdGb) {
    Write-Host "增量缓存 $sizeGb GB，低于阈值 $ThresholdGb GB，保持不动。"
    exit 0
}

Remove-Item $incremental -Recurse -Force
Write-Host "已清理增量缓存 $sizeGb GB（$incremental）。下次编译会自动重建。"

# 顺带提示整体体积，便于观察增长趋势
if (Test-Path $targetDir) {
    $totalBytes = (Get-ChildItem $targetDir -Recurse -File | Measure-Object -Property Length -Sum).Sum
    Write-Host ("target/ 当前总体积：{0} GB" -f [math]::Round($totalBytes / 1GB, 2))
}
