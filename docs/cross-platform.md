# Lume 跨平台安装与使用

Lume 支持 **Linux x64、macOS arm64、Windows x64** 三大桌面平台。本文说明预编译包的安装、从源码构建，以及 REPL 的跨平台行为。

## 平台支持矩阵

| 平台 | 预编译产物 | CLI 入口 | LLVM 23 来源 |
| --- | --- | --- | --- |
| Linux x64 | `lume-linux-x64.tar.gz` | `lume` | apt.llvm.org |
| macOS arm64 | `lume-macos-arm64.tar.gz` | `lume` | Homebrew `llvm@23` |
| Windows x64 | `lume-windows-x64.zip` | `lume.cmd` / `lume.ps1` | LLVM 官方 win64 包 |

> Intel Mac 与 Linux 非 x64 架构没有预编译包，需从源码构建。Windows 的 LLVM 由 CI 脚本修正 `llvm-config` 输出后构建，见 `.github/scripts/setup-llvm-windows.ps1`。

## 安装预编译包

预编译包在每次 `v*` tag 推送时由 `.github/workflows/release.yml` 自动构建并上传到 GitHub Releases，附带 SHA-256 校验和。

### Linux / macOS

```bash
curl -fsSL https://raw.githubusercontent.com/YangShiDanny/lume/main/install.sh | bash
```

或指定版本与安装前缀：

```bash
LUME_VERSION=v0.1.0 LUME_PREFIX=$HOME/.lume bash install.sh
```

安装脚本会把包解压到 `$LUME_PREFIX/lib/lume`，并在 `$LUME_PREFIX/bin/lume` 创建启动器。若该目录不在 PATH：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Windows

在 PowerShell 中运行：

```powershell
irm https://raw.githubusercontent.com/YangShiDanny/lume/main/install.ps1 | iex
```

或指定版本：

```powershell
$env:LUME_VERSION='v0.1.0'; irm .../install.ps1 | iex
```

脚本把包解压到 `%LOCALAPPDATA%\lume`，并在 `%LOCALAPPDATA%\lume\bin\lume.cmd` 创建启动器。把该目录加入 PATH 后即可用 `lume`。

## 包内容

```text
lume-<platform>/
├── bin/lume-bootstrap[.exe]   # Rust 运行时
├── lume / lume.cmd / lume.ps1 # 平台入口
├── lume_launcher.py           # 跨平台公共逻辑
├── lume_repl.py               # REPL（逐行执行 + 语法高亮）
├── lib/                       # bootstrap、语法、标准库（必须随二进制分发）
└── README.md / docs/*.md
```

`lume-bootstrap` 无参数启动时会读取当前目录下的 `lib/bootstrap.lume`，因此不要只复制二进制而丢掉 `lib/`。

## 命令

```text
lume                    进入 REPL
lume -i | --repl        同上
lume run <file.lume>    解释执行
lume check <file.lume>  语法检查
lume jit <file.lume>    LLVM JIT 执行
lume llvm-ir <file.lume>
lume manifest <file.toml>
lume --lsp
lume --build            构建 lume-bootstrap
lume --help
```

Windows 用 `lume.cmd`（CMD/PowerShell 均可），也可用 `lume.ps1`。文件模式保留调用者的相对路径语义。

## REPL 的跨平台行为

- **Python 3.10+ 必需**：用于驱动 REPL。解释器查找顺序：
  1. `LUME_REPL_PYTHON`
  2. 安装目录 `.venv/bin/python`（Windows 为 `Scripts/python.exe`）
  3. `$HOME/.local/share/lume-repl-venv/bin/python`（Windows 为 `Scripts/python.exe`）
  4. PATH 中的 `python3` / `python`
- **语法高亮可选**：`prompt_toolkit` + `pygments` 存在时启用；缺失或终端不兼容时自动退回纯 `input()`，功能不受影响。
- **颜色**：检测到 `NO_COLOR`、非 TTY 或 Windows 旧控制台时，错误信息改为纯文本 `error:` 前缀。
- **历史文件**：统一放在 `~/.lume_repl_history`（Windows 为 `%USERPROFILE%\.lume_repl_history`）。
- **污染的环境变量**：Unix 启动器会清掉 `PYTHONPATH` 和 `LD_LIBRARY_PATH`，避免污染 Python 解释器（某些环境里系统 Python 会因这两个变量报错）。

与平台相关的入口文件：

| 文件 | 作用 |
| --- | --- |
| `lume` | Unix 薄包装，找到 Python 后调用 `lume_launcher.py` |
| `lume.cmd` | Windows CMD 薄包装 |
| `lume.ps1` | Windows PowerShell 薄包装 |
| `lume_launcher.py` | 公共逻辑：定位根目录、帮助、venv 查找、构建与参数转发 |
| `lume_repl.py` | REPL 本体 |

## 从源码构建

需要 Rust stable 与 LLVM 23.1 开发文件（`llvm-config` 必须在 PATH 或设置 `LLVM_SYS_231_PREFIX`）。

```bash
git clone https://github.com/YangShiDanny/lume.git
cd lume
cargo build --release --bin lume-bootstrap
```

各平台 LLVM 安装：

- **Ubuntu / Debian**：`bash .github/scripts/setup-llvm-linux.sh`
- **macOS**：`bash .github/scripts/setup-llvm-macos.sh`（Homebrew `llvm@23`）
- **Windows**：在 Git Bash 里解压 LLVM 官方 win64 包后运行 `.github/scripts/setup-llvm-windows.ps1`，它会写入 `LLVM_SYS_231_PREFIX` 并生成修正 `llvm-config` 输出的垫片。

## CI 与发布

- `.github/workflows/ci.yml`：push / PR 时在三个平台并行执行格式检查、测试、Clippy、Release 构建与 Lume 示例。
- `.github/workflows/release.yml`：推送 `v*` tag 或手动触发时，三平台打包并上传到 GitHub Release，附 SHA-256 校验和。
- 网页编辑器的构建与 Pages 部署仍在 CI 中，独立于原生二进制发布。
