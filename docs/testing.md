# Lume 测试总览

本文列出仓库现有**全部测试项**、本地运行命令和 CI 覆盖关系，方便贡献者快速查找
“改了某块代码该跑什么”。完整语言规范见 `docs/lume_spec.md`，路线图见 `todo.md`。

## 1. 两个 CI 工作流

| 工作流 | 文件 | 职责 |
| --- | --- | --- |
| `Tests` | `.github/workflows/test.yml` | **专门测试**：Rust 单测、Clippy、示例矩阵、socket 回环、stage1 自举、VS Code 编译、LLVM 多语言测试 |
| `CI` | `.github/workflows/ci.yml` | 三平台构建门禁、Release、Pages 部署、Wiki 同步；同时保留基础测试作为发布前复核 |

两个工作流都在 `push` / `pull_request` / 手动触发时运行。`Tests` 在 Ubuntu 24.04
上跑全量，并为 C/Rust/Fortran 单设一个 LLVM 后端 job；`CI` 负责 Windows/macOS/Linux
三平台的构建与产物。

## 2. 一键本地全量测试

按 CI 等价顺序执行（推荐用 `+stable`，nightly rustfmt 与 stable 有差异）：

```bash
cargo +stable fmt --all -- --check
git diff --check
cargo +stable clippy --workspace --all-targets --features libffi/system -- -D warnings
cargo +stable test --workspace --features libffi/system
cargo +stable build -p lumec_core --features libffi/system --release --quiet
bash scripts/test-examples.sh target/release/lume-bootstrap
LUME_BIN=target/release/lume-bootstrap bash examples/server_echo/test_echo.sh
bash scripts/test-llvm-languages.sh
(cd editor/vscode‑lume && npm ci && npm run compile)
```

## 3. 测试矩阵

| 类别 | 覆盖内容 | 运行命令 | CI |
| --- | --- | --- | --- |
| 格式与工作区 | stable rustfmt、`git diff --check` | `cargo +stable fmt --all -- --check`、`git diff --check` | 两者 |
| Rust 单元测试 | 93 项 core + 9 项 Immix | `cargo +stable test --workspace --features libffi/system` | 两者 |
| 严格 Clippy | `-D warnings`，含所有 targets | `cargo +stable clippy --workspace --all-targets --features libffi/system -- -D warnings` | 两者 |
| 示例矩阵 | check/run/jit/llvm-ir/header/docs + 输出断言 | `bash scripts/test-examples.sh <BIN>` | 两者 |
| Socket 回环 | 单客户端 + 多客户端并发 echo | `bash examples/server_echo/test_echo.sh` | 两者 |
| Stage1 自举 | Lume 前端 → 文本 IR → Rust 后端 JIT | `cargo test ... bootstrap::tests` + 示例矩阵中的 `bootstrap`/`bootstrap-ir` | 两者 |
| LSP | stdio 握手、诊断、补全、hover、UTF-16 位置 | 包含在 `cargo test` 的 `lsp::tests` 中 | 两者 |
| VS Code 扩展 | TypeScript 编译 | `npm ci && npm run compile` | `Tests` |
| LLVM 多语言 | C/Rust/Fortran → bitcode、符号校验、`llvm-link`、`lli` | `bash scripts/test-llvm-languages.sh` | `Tests` |

### 3.1 Rust 单元测试

`cargo test --workspace --features libffi/system` 当前通过 **93 项 core + 9 项 Immix
doc-test**。按模块：

- `buffer`：字节 buffer 的 little-endian 往返、边界检查、`memcpy` 重叠拒绝、`memmove`
  重叠复制、pin 释放。
- `clock`：可注入单调/墙钟时钟，线程隔离。
- `codegen`：定宽 cast、buffer JIT 内置、`if` phi 合并、`while`、短路布尔、各分支
  `ret` 不产生非法合并。
- `evaluator`：定宽构造越界、GC buffer 内置、`memcpy` 偏移、`memmove` 重叠、时间内置。
- `ffi`：签名解析（ABI 前缀、变参提升、void 参数拒绝）、按值 struct/union、FFI buffer
  对齐与 C 字符串、共享库加载与类型化调用、C 回调与 panic 隔离、回调传给 C `qsort`。
- `gc`：bump 分配对齐、标记、根保活、pin 阻止搬迁并嵌套计数、非堆指针拒绝、
  搬迁后字段/根改写、阈值触发收集。
- `ir` / `ir_text`：IR 校验（terminator、phi 前驱）、文本 IR 解析与 ValueId 连续性检查。
- `jit`：整数算术与比较、`if` 合并且函数复用、递归、bitcode 外部函数、注册内置、
  延迟定义/错误元数拒绝、`while` 与多函数调用、时间内置。
- `lsp`：framed 消息解析、initialize/open/completion、局部变量补全、hover、诊断范围、
  UTF-16 位置换算。
- `manifest` / `project`：manifest 与 export 段解析、docs 生成、Darwin 下划线符号兼容。
- `bootstrap`：5 项 stage1 测试（单函数 lowering/JIT、嵌套 `while`、`if`/`else`、
  自举入口、多函数 IR 解析）。
- `parser` / `typecheck` / `modules` / `typeobj`：语法与控制流、闭包、泛型、match、结构体、
  FFI 属性、日志/数学、类型兼容、相对 import 去重、自写 lexer/parser 测试、C 符号调用与
  可变内存传参。

### 3.2 示例矩阵

`scripts/test-examples.sh` 逐项检查并返回预期输出：

- `demo_basic`：`check`，`run` 输出 `42`。
- `demo_buffer`：`check`，`run` 输出 buffer 与字符串转换结果。
- `demo_buffer_jit`：`check`，`run`/`jit` 均输出 `50727435`，`llvm-ir` 生成成功。
- `demo_math_logging`：解释执行通过。
- `demo_llvm_jit`：`jit`、`llvm-ir` 通过。
- stage1：`bootstrap examples/demo_llvm_jit.lume` 输出 `55`，`bootstrap-ir` 生成 LLVM IR。
- `demo_c_interop`：`check`、`header`、`run`/`jit` 均输出 `8256`、`llvm-ir`、`docs`
  包含模块与外部符号。

### 3.3 Socket 回环

`examples/server_echo/test_echo.sh` 编译并运行两个 C 客户端：

- `test_echo.c`：单客户端发送 `hello lume` 并校验回显（`ECHO_OK hello lume`）。
- `test_echo_multi.c`：客户端 A 保活期间连接客户端 B，B 必须得到回显；随后 A 再发一条
  也必须成功（`ECHO_MULTI_OK`）。旧的顺序 accept/echo 循环无法通过此项。

服务由 `mono_ms()` 空闲窗口自动退出，脚本负责清理进程与临时目录。

### 3.4 多语言 LLVM 后端测试

`scripts/test-llvm-languages.sh` 把三种语言统一到 LLVM 工具链，避免“各语言用不同后端”
导致的公平性争议：

| 语言 | 源码 | 产出 bitcode 的方式 | 导出符号 |
| --- | --- | --- | --- |
| C | `tests/llvm-languages/add.c` | `clang -target <rustc host> -emit-llvm -c` | `lume_test_add_c` |
| Rust | `tests/llvm-languages/add.rs` | `rustc --emit=llvm-ir` → `llvm-as`（`#![no_std]` + `wrapping_add`，无 panic/eh 依赖） | `lume_test_add_rust` |
| Fortran | `tests/llvm-languages/add.f90` | `flang/flang-new -emit-llvm -c`，`bind(C)` 保证符号无下划线 | `lume_test_add_fortran` |

测试步骤：

1. 每个模块用 `llvm-nm --defined-only` 校验导出符号。
2. 生成一个 C `main`，同时调用所有可用语言的实现。
3. `llvm-link` 合并成单一 bitcode。
4. `lli` 执行，期望输出 `LLVM_LANG_OK`。

约定：

- C/Rust 是必测项；Fortran 编译器（flang / flang-new / flang-23）不存在时打印
  `SKIP` 并继续，不判失败——CI 的 `llvm-languages` job 会先尝试 `apt install flang-23`。
- `lli` 不存在时跳过执行步骤，但 bitcode 生成、符号校验与链接仍然通过。
- clang 的 `-target` 显式对齐 rustc host triple，避免 `llvm-link` 的 triple 警告。
- 该测试是 LLVM 工具链一致性测试，不是性能基准；基准/对比模块尚未落地。

## 4. 如何新增测试项

1. 语言/运行时行为测试放进 `crates/lumec_core/src/*` 的 `#[cfg(test)]` 模块，随
   `cargo test` 自动运行。
2. 可运行示例放进 `examples/`，并在 `scripts/test-examples.sh` 加 `check_ok` /
   `run_expected` 断言。
3. 多进程/网络集成测试放进对应示例目录的 `test_*.sh`，由 `ci.yml` 和 `test.yml`
   同时调用。
4. 新语言后端按 `tests/llvm-languages/` 的模式添加：导出 `extern "C"` 符号、产出
   LLVM bitcode、加入 `check_symbol` 与链接执行。
5. 每新增一项，同步更新本文的测试矩阵和 CI 工作流，确保别人能找到。

## 5. 当前未覆盖

- 完整 stage2 自托管（stage0 加载与类型检查尚未由 Lume 接管）。
- P2 多语言项目端到端（Rust/Zig bitcode 与 Lume manifest 深度集成）。
- GPU 加速与硬件验证。
- socket 的 TLS、IPv6、写半关闭背压处理。
- 跨语言 GC 契约的端到端测试（外部 C 长期持有指针、忘记 unpin 的诊断）。
