# 在 Windows runner 上安装 LLVM 23 并修正 llvm-sys 的已知链接问题。
#
# 背景：LLVM 官方 win64 二进制的 llvm-config --system-libs 会输出构建机的绝对路径
# （例如 S:/llvm/.../zstd_static.lib），llvm-sys 把这些当作库名传给 rustc 会失败；
# 同时 zig/zlib/xml2 的 .lib 名字与官方包不一致。
#
# 本脚本：
#   1. 从 GitHub Releases 下载官方 clang+llvm win64 MSVC 压缩包；
#   2. 解压到 $env:RUNNER_TEMP/llvm-23；
#   3. 用 PowerShell 模块写一个 llvm-config.bat 垫片，重写 --system-libs 输出；
#   4. 设置 LLVM_SYS_231_PREFIX / PATH 供后续 cargo build 使用。
#
# 不修改仓库源码；只在 CI 环境运行。

$ErrorActionPreference = 'Stop'

$Version = $env:LLVM_VERSION
if (-not $Version) { $Version = '23.1.6' }
$Prefix = Join-Path $env:RUNNER_TEMP 'llvm-23'
$Archive = Join-Path $env:RUNNER_TEMP 'llvm-23.7z'
$Url = "https://github.com/llvm/llvm-project/releases/download/llvmorg-$Version/clang+llvm-$Version-x86_64-pc-windows-msvc.tar.xz"

function Get-WithRetry {
    param([string]$Uri, [string]$Out, [int]$Tries = 5)
    for ($i = 1; $i -le $Tries; $i++) {
        try {
            Invoke-WebRequest -Uri $Uri -OutFile $Out -UseBasicParsing
            return
        } catch {
            if ($i -eq $Tries) { throw }
            Start-Sleep -Seconds (2 * $i)
        }
    }
}

Write-Host "Downloading $Url"
Get-WithRetry -Uri $Url -Out $Archive

Write-Host "Extracting LLVM to $Prefix"
New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
tar -xf $Archive -C $Prefix --strip-components=1
if ($LASTEXITCODE -ne 0) { throw 'tar extraction failed' }

$RealConfig = Join-Path $Prefix 'bin\llvm-config.exe'
if (-not (Test-Path $RealConfig)) { throw "llvm-config.exe not found at $RealConfig" }

# llvm-config 垫片：把 --system-libs 中带绝对路径的 *.lib 改写为纯文件名，
# 并把已知缺失的 zstd/zlib/xml2 库名映射到官方包里的名字。
$Shim = Join-Path $Prefix 'bin\llvm-config-23.bat'
$ShimContent = @'
@echo off
setlocal enabledelayedexpansion
set "REAL=%LLVM_REAL_CONFIG%"
set "ARGS="
:loop
if "%~1"=="" goto done
set "ARGS=!ARGS! %~1"
shift
goto loop
:done
if "%ARGS%"==" --system-libs" (
  for /f "delims=" %%L in ('"%REAL%" --system-libs') do (
    set "line=%%L"
    set "line=!line:S:/=/!"
    for %%F in (!line!) do (
      set "lib=%%~nxF"
      if /i "!lib!"=="zstd_static.lib" set "lib=zstd.lib"
      if /i "!lib!"=="zs.lib" set "lib=zstd.lib"
      if /i "!lib!"=="xml2s.lib" set "lib=libxml2.lib"
      echo !lib!
    )
  )
) else if "%ARGS%"==" --system-libs --link-static" (
  for /f "delims=" %%L in ('"%REAL%" --system-libs --link-static') do (
    set "line=%%L"
    set "line=!line:S:/=/!"
    for %%F in (!line!) do (
      set "lib=%%~nxF"
      if /i "!lib!"=="zstd_static.lib" set "lib=zstd.lib"
      if /i "!lib!"=="zs.lib" set "lib=zstd.lib"
      if /i "!lib!"=="xml2s.lib" set "lib=libxml2.lib"
      echo !lib!
    )
  )
) else if "%ARGS%"==" --system-libs --link-shared" (
  for /f "delims=" %%L in ('"%REAL%" --system-libs --link-shared') do (
    set "line=%%L"
    set "line=!line:S:/=/!"
    for %%F in (!line!) do (
      set "lib=%%~nxF"
      if /i "!lib!"=="zstd_static.lib" set "lib=zstd.lib"
      if /i "!lib!"=="zs.lib" set "lib=zstd.lib"
      if /i "!lib!"=="xml2s.lib" set "lib=libxml2.lib"
      echo !lib!
    )
  )
) else (
  "%REAL%" %ARGS%
)
'@
Set-Content -Path $Shim -Value $ShimContent -Encoding Ascii

# 官方包可能缺少这些别名库，复制一份满足链接器。
$LibDir = Join-Path $Prefix 'lib'
foreach ($pair in @(@('zstd.lib', 'zstd_static.lib'), @('libxml2.lib', 'xml2s.lib'))) {
    $src = Join-Path $LibDir $pair[0]
    $dst = Join-Path $LibDir $pair[1]
    if ((Test-Path $src) -and -not (Test-Path $dst)) {
        Copy-Item -Path $src -Destination $dst
    }
}

$VersionOutput = & $RealConfig --version
Write-Host "LLVM version: $VersionOutput"

"LLVM_SYS_231_PREFIX=$Prefix" | Out-File -FilePath $env:GITHUB_ENV -Append
"LLVM_REAL_CONFIG=$RealConfig" | Out-File -FilePath $env:GITHUB_ENV -Append
"$Prefix\bin" | Out-File -FilePath $env:GITHUB_PATH -Append
