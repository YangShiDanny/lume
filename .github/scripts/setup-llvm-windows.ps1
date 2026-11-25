# 在 Windows runner 上安装 LLVM 23，并修正 llvm-sys 的已知链接问题。
#
# 背景：LLVM 官方 win64 二进制的 llvm-config --system-libs 会输出构建机的绝对路径
# （例如 S:/llvm/.../zstd_static.lib）。llvm-sys 231 会把每个 token 当库名交给 rustc，
# 变成 -l dylib:S:/.../zstd_static，rustc 把冒号解析成「重命名」语法，于是报
# "renaming of the library `S` was specified"。
#
# 为什么不能用 .bat 垫片：llvm-sys 只按顺序尝试 llvm-config-231.exe、
# llvm-config-231、llvm-config231.exe、…、llvm-config.exe。带路径且无扩展名的
# CreateProcess 不会补 .bat，PATHEXT 也不参与，所以 llvm-config-231.bat
# 永远不会被选中，最终仍然执行真实的 llvm-config.exe。
#
# 本脚本：
#   1. 用 curl.exe 下载官方 clang+llvm win64 MSVC 包（支持镜像/直链覆盖及缓存复用）；
#   2. 优先用 7z 解压，没有 7z 时回退到 tar；
#   3. 把真实的 llvm-config.exe 改名为 llvm-config-real.exe，然后用 rustc 编译一个
#      原生 llvm-config.exe 垫片：除 --system-libs 外全部透传，--system-libs 则把
#      绝对路径收敛为纯 .lib 文件名，并修正已知的库名差异；
#   4. 设置 LLVM_SYS_231_PREFIX / LLVM_REAL_CONFIG / PATH 供后续 cargo build 使用。
#
# 不修改仓库源码；只在 CI 环境运行。Windows runner 不预装 LLVM 23，全靠本脚本准备。

$ErrorActionPreference = 'Stop'

$Version = $env:LLVM_VERSION
if (-not $Version) { $Version = '23.1.6' }
$Prefix = Join-Path $env:RUNNER_TEMP 'llvm-23'
$Archive = Join-Path $env:RUNNER_TEMP 'llvm-23.tar.xz'
# llvm-sys 实际会执行的路径（必须是原生可执行文件，不能是 .bat）
$ShimConfig = Join-Path $Prefix 'bin\llvm-config.exe'
# 官方原始 llvm-config，改名后由垫片调用
$RealConfig = Join-Path $Prefix 'bin\llvm-config-real.exe'

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
} elseif (Test-Path $ShimConfig) {
    Write-Host "Reusing extracted LLVM at $Prefix (shim not installed yet)"
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
}

if (-not (Test-Path $RealConfig) -and -not (Test-Path $ShimConfig)) {
    throw "llvm-config.exe not found under $Prefix\bin"
}

if (-not (Test-Path $RealConfig)) {
    Move-Item -Path $ShimConfig -Destination $RealConfig -Force
}

# 编译原生垫片。必须叫 llvm-config.exe：这是 llvm-sys 的最后一个探测名字，
# 也是唯一能可靠覆盖官方二进制的名字。用 rustc 而不是 C 编译器，避免依赖 vcvars。
if (-not (Get-Command rustc -ErrorAction SilentlyContinue)) {
    throw 'rustc is required to build the llvm-config shim; install the Rust toolchain first'
}

$ShimSource = Join-Path $env:RUNNER_TEMP 'llvm-config-shim.rs'
$ShimCode = @"
use std::env;
use std::io::Write;
use std::path::Path;
use std::process::{exit, Command};

const REAL: &str = r#"$RealConfig"#;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let system_libs = args.iter().any(|arg| arg == "--system-libs");
    let output = match Command::new(REAL).args(&args).output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("llvm-config shim: cannot run {REAL}: {error}");
            exit(1);
        }
    };
    if !output.status.success() {
        let _ = std::io::stderr().write_all(&output.stderr);
        exit(output.status.code().unwrap_or(1));
    }
    if system_libs {
        // llvm-sys 在 MSVC 上要求每个 token 都是 `foo.lib`；把
        // `S:/llvm/.../zstd_static.lib` 这样的绝对路径收敛成纯文件名。
        for token in String::from_utf8_lossy(&output.stdout)
            .split(|c: char| c.is_ascii_whitespace())
        {
            if token.is_empty() { continue; }
            let basename = Path::new(token)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| token.to_string());
            if !basename.to_ascii_lowercase().ends_with(".lib") { continue; }
            let lower = basename.to_ascii_lowercase();
            let fixed: String = match lower.as_str() {
                "zstd_static.lib" | "zs.lib" => "zstd.lib".to_string(),
                "xml2s.lib" | "libxml2s.lib" => "libxml2.lib".to_string(),
                "zlib_static.lib" => "zlib.lib".to_string(),
                _ => basename.clone(),
            };
            println!("{fixed}");
        }
    } else {
        let _ = std::io::stdout().write_all(&output.stdout);
    }
    let _ = std::io::stderr().write_all(&output.stderr);
}
"@
Set-Content -Path $ShimSource -Value $ShimCode -Encoding UTF8
& rustc --edition=2021 -O -o $ShimConfig $ShimSource
if ($LASTEXITCODE -ne 0) { throw 'failed to compile llvm-config shim' }
Write-Host "Installed llvm-config shim at $ShimConfig"

# 官方包可能缺少这些别名库，复制一份满足链接器。
$LibDir = Join-Path $Prefix 'lib'
foreach ($pair in @(@('zstd.lib', 'zstd_static.lib'), @('libxml2.lib', 'xml2s.lib'))) {
    $src = Join-Path $LibDir $pair[0]
    $dst = Join-Path $LibDir $pair[1]
    if ((Test-Path $src) -and -not (Test-Path $dst)) {
        Copy-Item -Path $src -Destination $dst
    }
}

# 冒烟检查：确认垫片与真实 llvm-config 都可用，且 --system-libs 不再带绝对路径。
$VersionOutput = & $RealConfig --version
Write-Host "LLVM version: $VersionOutput"
$ShimVersion = & $ShimConfig --version
if ($ShimVersion -ne $VersionOutput) {
    throw "llvm-config shim version mismatch: '$ShimVersion' vs '$VersionOutput'"
}
$SystemLibs = & $ShimConfig --system-libs --link-static
Write-Host "Sanitized --system-libs: $SystemLibs"
if ($SystemLibs -match '[A-Za-z]:[/\\]') {
    throw 'llvm-config shim still emits absolute library paths'
}

$env:LLVM_REAL_CONFIG = $RealConfig
"LLVM_SYS_231_PREFIX=$Prefix" | Out-File -FilePath $env:GITHUB_ENV -Append
"LLVM_REAL_CONFIG=$RealConfig" | Out-File -FilePath $env:GITHUB_ENV -Append
"$Prefix\bin" | Out-File -FilePath $env:GITHUB_PATH -Append
