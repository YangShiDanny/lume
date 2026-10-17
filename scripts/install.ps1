# install.ps1
# 把 lume.exe 所在目录加进用户 PATH

$ErrorActionPreference = "Stop"

$exeDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$exePath = Join-Path $exeDir "lume.exe"

if (-not (Test-Path $exePath)) {
    Write-Host "错误：$exeDir 下找不到 lume.exe" -ForegroundColor Red
    exit 1
}

$userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($null -eq $userPath) { $userPath = "" }

if ($userPath -split ';' -contains $exeDir) {
    Write-Host "已安装：$exeDir 已在 PATH 里" -ForegroundColor Green
} else {
    $newPath = if ($userPath) { "$userPath;$exeDir" } else { $exeDir }
    [Environment]::SetEnvironmentVariable("PATH", $newPath, "User")
    Write-Host "已把 $exeDir 加入用户 PATH" -ForegroundColor Green
}

$clang = Get-Command clang -ErrorAction SilentlyContinue
if (-not $clang) {
    Write-Host ""
    Write-Host "警告：没找到 clang。lume 编译 C 需要它。" -ForegroundColor Yellow
    Write-Host "安装：winget install LLVM.LLVM" -ForegroundColor Yellow
    Write-Host "装完新开一个终端。" -ForegroundColor Yellow
} else {
    Write-Host "clang 已找到：$($clang.Source)" -ForegroundColor Green
}

Write-Host ""
Write-Host "完成。关掉所有终端，重新打开，然后运行：" -ForegroundColor Cyan
Write-Host "  lume run hello.lume" -ForegroundColor Cyan