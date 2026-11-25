# 在 Windows runner 上安装 LLVM 23 并修正 llvm-sys 的已知链接问题。
#
# 背景：LLVM 官方 win64 二进制的 llvm-config --system-libs 会输出构建机的绝对路径
# （例如 S:/llvm/.../zstd_static.lib），llvm-sys 把这些当作库名传给 rustc 会失败；
# 同时 zig/zlib/xml2 的 .lib 名字与官方包不一致。
#
# 本脚本：
#   1. 用 curl.exe（流式下载，远快于 Invoke-WebRequest）从 GitHub Releases
#      （或 LLVM_BASE_URL / LLVM_DOWNLOAD_URL 指定的镜像）下载官方 clang+llvm win64 MSVC 包；
#      已解压过则直接复用；
#   2. 优先用 7z 解压（比 bsdtar 快很多），没有 7z 时回退到 tar；
#   3. 写一个 llvm-config-231.bat 垫片，重写 --system-libs 输出；
#      名字必须匹配 llvm-sys 231 的查找顺序，否则它会直接用真实 llvm-config.exe，
#      把构建机绝对路径（S:/...）当库名传给 rustc，触发 "renaming of the library S"。
#   4. 设置 LLVM_SYS_231_PREFIX / PATH 供后续 cargo build 使用。
#
# 不修改仓库源码；只在 CI 环境运行。Windows runner 不预装 LLVM 23，全靠本脚本准备。

$ErrorActionPreference = 'Stop'

$Version = $env:LLVM_VERSION
if (-not $Version) { $Version = '23.1.6' }
$Prefix = Join-Path $env:RUNNER_TEMP 'llvm-23'
$Archive = Join-Path $env:RUNNER_TEMP 'llvm-23.tar.xz'
$RealConfig = Join-Path $Prefix 'bin\llvm-config.exe'

# 默认从官方 GitHub Release 下载；网络不通时可用 LLVM_BASE_URL 指向镜像，
# 或用 LLVM_DOWNLOAD_URL 直接指定完整的 .tar.xz 地址。
$BaseUrl = $env:LLVM_BASE_URL
if (-not $BaseUrl) { $BaseUrl = 'https://github.com/llvm/llvm-project/releases/download' }
$BaseUrl = $BaseUrl.TrimEnd('/')
if ($env:LLVM_DOWNLOAD_URL) {
    $Url = $env:LLVM_DOWNLOAD_URL
} else {
    $Url = "$BaseUrl/llvmorg-$Version/clang+llvm-$Version-x86_64-pc-windows-msvc.tar.xz"
}

function Get-WithRetry {
    param([string]$Uri, [string]$Out, [int]$Tries = 5)
    for ($i = 1; $i -le $Tries; $i++) {
        # Invoke-WebRequest 会把几百 MB 全部缓进内存，在 CI 上极慢；curl.exe 是流式的。
        & curl.exe -L --fail --show-error --retry 3 --connect-timeout 30 --max-time 1800 -o $Out $Uri
        if ($LASTEXITCODE -eq 0) { return }
        if ($i -eq $Tries) { throw "curl download failed with exit code $LASTEXITCODE" }
        Start-Sleep -Seconds (2 * $i)
    }
}

if (Test-Path $RealConfig) {
    $ExistingVersion = & $RealConfig --version
    Write-Host "Reusing LLVM at $Prefix (version $ExistingVersion)"
} else {
    if (Test-Path $Archive) {
        Write-Host "Reusing downloaded archive $Archive"
    } else {
        Write-Host "Downloading $Url"
        Get-WithRetry -Uri $Url -Out $Archive
    }

    Write-Host "Extracting LLVM to $Prefix"
    New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
    $SevenZip = Get-Command 7z.exe -ErrorAction SilentlyContinue
    if ($SevenZip) {
        # 7z 解压几万个文件的速度明显快于 Windows 自带 bsdtar。
        & 7z x $Archive "-o$Prefix" -y | Out-Null
        if ($LASTEXITCODE -ne 0) { throw '7z xz extraction failed' }
        $InnerTar = Get-ChildItem -Path $Prefix -Filter '*.tar' -File | Select-Object -First 1
        if (-not $InnerTar) { throw 'inner LLVM tar not found' }
        & 7z x $InnerTar.FullName "-o$Prefix" -y | Out-Null
        if ($LASTEXITCODE -ne 0) { throw '7z tar extraction failed' }
        Remove-Item -Path $InnerTar.FullName -Force
        $Nested = Join-Path $Prefix "clang+llvm-$Version-x86_64-pc-windows-msvc"
        if (Test-Path $Nested) {
            Get-ChildItem -Force -Path $Nested | Move-Item -Destination $Prefix -Force
            Remove-Item -Path $Nested -Recurse -Force
        }
    } else {
        tar -xf $Archive -C $Prefix --strip-components=1
        if ($LASTEXITCODE -ne 0) { throw 'tar extraction failed' }
    }
    if (-not (Test-Path $RealConfig)) { throw "llvm-config.exe not found at $RealConfig" }
}

# llvm-config 垫片：把 --system-libs 中带绝对路径的 *.lib 改写为纯文件名，
# 并把已知缺失的 zstd/zlib/xml2 库名映射到官方包里的名字。
$Shim = Join-Path $Prefix 'bin\llvm-config-231.bat'
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
# llvm-sys 也会尝试 llvm-config231（无连字符），补一个别名确保垫片一定被选中。
Copy-Item -Path $Shim -Destination (Join-Path $Prefix 'bin\llvm-config231.bat') -Force

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
