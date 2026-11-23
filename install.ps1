# Lume 安装脚本（Windows PowerShell / PowerShell Core）。
# 从 GitHub Releases 下载 windows-x64 预编译包，解压到本地并创建 lume.cmd 启动器。
param(
  [string]$Repo = 'YangShiDanny/lume',
  [string]$Version = 'latest'
)

$ErrorActionPreference = 'Stop'
$Prefix = Join-Path $env:LOCALAPPDATA 'lume'
$BinDir = Join-Path $Prefix 'bin'
$InstallDir = Join-Path $Prefix 'lume-windows-x64'
$Tmp = Join-Path $env:TEMP 'lume-install.zip'

if ($Version -eq 'latest') {
  $Url = "https://github.com/$Repo/releases/latest/download/lume-windows-x64.zip"
} else {
  $Url = "https://github.com/$Repo/releases/download/$Version/lume-windows-x64.zip"
}

Write-Host "[lume] 下载 $Url"
Invoke-WebRequest -Uri $Url -OutFile $Tmp -UseBasicParsing

Write-Host "[lume] 解压到 $Prefix"
New-Item -ItemType Directory -Force -Path $Prefix, $BinDir | Out-Null
if (Test-Path $InstallDir) { Remove-Item -Recurse -Force $InstallDir }
Expand-Archive -Path $Tmp -DestinationPath $Prefix -Force

# 在 bin 目录创建稳定的 lume.cmd 启动器。
$Launcher = @"
@echo off
"%~dp0..\lume-windows-x64\lume.cmd" %*
"@
Set-Content -Path (Join-Path $BinDir 'lume.cmd') -Value $Launcher -Encoding Ascii

Write-Host
Write-Host "[lume] 安装完成。把下面目录加入 PATH 后即可使用："
Write-Host "  $BinDir"
Write-Host "[lume] 运行 'lume --help' 或 'lume' 进入 REPL。"
