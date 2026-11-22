# LumeC

LumeC 是 Lume 语言的实验性实现，包含 Rust 编写的运行时内核、Lume
语法和标准库源码，以及 Visual Studio Code 扩展。本文介绍如何安装环境、
运行 `.lume` 文件、检查源码和运行测试。

> **项目状态**
>
> Lume 正在积极开发中，语法和运行时接口可能变化。当前 `lume-bootstrap`
> 可以解析并解释执行一部分 Lume 程序；LLVM JIT 和外部 bitcode 加载已经
> 在内核中提供，但完整的「Lume 源码 → IR → LLVM」编译链仍在开发中。
> 因此，下面的 `run` 命令运行的是当前解释器，不代表已完成自举编译器。

## 目录结构

```text
crates/lumec_core/    Rust 运行时、LLVM JIT、lume-bootstrap 命令
lib/                  Lume 语法、标准库和构建工具源码
examples/             示例程序
docs/                 语言与内置函数文档
editor/vscode‑lume/    Visual Studio Code 扩展
```

## 环境要求

- Linux x86_64
- Rust stable 工具链（Edition 2024）
- LLVM **23.1** 开发文件及 `llvm-config`
- Clang（只有编译混合项目中的 C 模块时才需要）

`lumec_core` 使用 Inkwell 的 `llvm23-1` 功能，因此只安装普通 Rust 工具链
还不够。以 Ubuntu 为例，可通过 LLVM 官方软件源安装 LLVM 23：

```bash
wget https://apt.llvm.org/llvm.sh
chmod +x llvm.sh
sudo ./llvm.sh 23
sudo apt-get install llvm-23-dev clang-23
```

如果 `llvm-config` 不在默认 PATH 中，设置 LLVM 安装目录：

```bash
export LLVM_SYS_231_PREFIX=/usr/lib/llvm-23
export PATH="$LLVM_SYS_231_PREFIX/bin:$PATH"
```

确认工具版本后，在仓库根目录构建：

```bash
llvm-config --version
cargo build --workspace
```

## 运行一个 `.lume` 文件

仓库自带一个可运行示例。所有命令都从仓库根目录执行。

### 1. 查看示例

打开 [`examples/demo_basic/src/main.lume`](examples/demo_basic/src/main.lume)：

```lume
fn add(a:int, b:int) -> int
ret a + b

fn main() -> int
    result:int = if add(20, 22) == 42
        42
    else
        0
    end
ret result
```

程序通过 `main` 函数启动；`ret` 后面的表达式是返回值。这里的 `if`
是块表达式，结果赋给 `result`。

### 2. 检查语法

```bash
cargo run -p lumec_core -- check examples/demo_basic/src/main.lume
```

成功时会显示 `OK` 和解析到的函数数量。如果文件有语法错误，会打印文件
与行号。

### 3. 运行程序

```bash
cargo run -p lumec_core -- run examples/demo_basic/src/main.lume
```

首次运行会先由 Cargo 编译 `lume-bootstrap`，随后执行文件，预期输出：

```text
42
```

构建过一次后也可直接调用二进制：

```bash
cargo build -p lumec_core
./target/debug/lume-bootstrap run examples/demo_basic/src/main.lume
```

`run` 可以省略，下面两种形式等价：

```bash
cargo run -p lumec_core -- examples/demo_basic/src/main.lume
cargo run -p lumec_core -- run examples/demo_basic/src/main.lume
```

### 4. 运行数学与日志示例

```bash
cargo run -p lumec_core -- run examples/demo_math_logging.lume
```

示例会打印一条日志和一个浮点结果。math/log API、可选 SLEEF 原生链接方式
及其准确度说明见[数学与日志内置函数](docs/math-and-logging.md)。

## Lume 语法速览

Lume 使用 `#` 编写单行注释，不使用花括号作为代码块边界。函数、条件和
循环的块通过缩进组织；正常结束的块使用 `end`。`ret` 会直接结束当前函数。

```lume
# 求 1 到 n 的和
fn sum_to(n:int) -> int
    total:int = 0
    i:int = 1
    while i <= n
        total := total + i
        i := i + 1
    end
ret total

fn main() -> int
ret sum_to(10)
```

运行后输出 `55`。变量声明可以带类型和初始值（例如 `total:int = 0`）；
`=` 是普通赋值，`:=` 是可在表达式中使用的海象赋值。更多语法和当前限制
见[语言说明](docs/lume_spec.md)。

## 命令一览

```bash
# 编译运行时
cargo build --workspace

# 检查源文件
cargo run -p lumec_core -- check path/to/program.lume

# 运行源文件
cargo run -p lumec_core -- run path/to/program.lume

# 运行测试
cargo test --workspace

# 检查 Rust 格式
cargo fmt --manifest-path crates/lumec_core/Cargo.toml -- --check
```

## Visual Studio Code 扩展

扩展提供 Lume 语法高亮、`#` 行注释、关键字/函数补全、悬停提示、函数符号、
基础诊断，以及运行和检查当前文件的命令。

在仓库根目录构建扩展：

```bash
cd editor/vscode‑lume
npm ci
npm run compile
npm run package
```

生成的 `.vsix` 文件可在 VS Code 中通过「Extensions: Install from VSIX...」
安装。运行和检查命令需要扩展能够找到 `lume-bootstrap`：可先在仓库根目录
运行 `cargo build -p lumec_core`，或设置 `LUME_BOOTSTRAP` 环境变量指向二进制。

## 持续集成

GitHub Actions 会在推送和 Pull Request 时安装 LLVM 23，并执行 Rust 格式、
测试、Clippy、Release 构建、Lume 示例检查/运行和 VS Code 扩展编译。

## 文档索引

- [Lume 语言说明](docs/lume_spec.md)
- [数学与日志内置函数](docs/math-and-logging.md)
- [VS Code 扩展说明](editor/vscode‑lume/README.md)
