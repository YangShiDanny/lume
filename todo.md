# Lume 开发路线图

> 2026-11-23 更新。旧版是语言设计稿；完整语法规范保留在 `docs/lume_spec.md`，本文件只维护“下一步做什么、做到什么算完”。
>
> 本轮先完成 P0 的静态 / AOT FFI 与 Clippy 基线债；P1-1 / P1-2 及本轮“地基补完”
> （JIT 定宽 cast / buffer、memmove、FFI ABI / 变参 / 按值聚合、JIT 侧 C 回调 Lume）
> 均已完成。随后修复 stable/nightly rustfmt 差异导致的 CI 格式门禁失败；P1-5 socket
> 最小 echo 已落地（顺序客户端），并发 poll 集合仍待做。所有改动均未提交。

## 状态快照

- 已完成（未提交）：通用动态 C FFI（共享库加载、符号解析、显式 C 签名调用、边界检查缓冲区、C 回调、回调错误查询与 panic 隔离）；Immix 并入 core 及 pin / 非堆指针 / 分配统计修复；FFI、回调与 GC pin 文档；libc FFI 示例。
- 已完成（未提交，P1）：`now_ms()` / `mono_ms()`（可注入时钟）、定宽整数静态类型与 JIT 零扩展 cast、GC 管理字节缓冲区与 `buf_*` / `memcpy` / `memmove` / 字符串转换、JIT buffer 内置函数、`examples/demo_buffer.lume`（解释器，含字符串转换）与 `examples/demo_buffer_jit.lume`（run/jit 通用，不含字符串）。
- 已完成（未提交，地基补完）：FFI ABI 前缀、C 变参（显式列出实参类型并做默认提升）、按值 struct/union（libffi 布局）、JIT 侧真实 `ffi_export_ptr` 回调；`demo_c_interop` 的 run / jit 输出均为 `8256`。
- 已完成（CI 侧）：三平台构建 / Release / Pages / Wiki 同步；LLVM 23 安装脚本修复（Linux 补 `libpolly-23-dev`，Windows 改流式下载 + 7z + 缓存，macOS runner 改 `macos-15`）。
- 已完成（未提交，P0）：manifest 解析与 C bitcode 构建/符号校验、JIT 外部符号分派、`#[ffi_export]` 头文件生成、`ffi_export_ptr` 回调桥、移除 `c_matmul` 桩、全 workspace 严格 Clippy 通过并恢复 CI 门禁。
- 已完成（未提交，P1-5 第一步）：`examples/server_echo` 的跨平台 C ABI 垫片、manifest extern 动态 FFI 绑定、`ffi_buffer` 接收、`mono_ms()` 空闲超时与 C 客户端回环测试；当前是顺序客户端循环。
- 待做：P1-5 多客户端并发 poll 集合；rawptr / unsafe、HashMap；多语言项目工具链；GPU SDK 封装与硬件验证。

## 排序原则

1. 先地基后上层：时间 → buffer → rawptr → HashMap → socket → 服务器 demo。
2. 每块必须有可运行示例 + 单测，不堆“只设计不落地”的文档。
3. 跨语言边界坚持 C ABI（i64 / 指针），不穿透 GC；GPU 先显式搬运，不自动统一内存。
4. 动态 FFI、`mono_ms()`、解释器与 JIT buffer、JIT C 回调均已完成；P1-5 已有可运行 echo 最小闭环，下一步补多客户端并发。
5. 全 workspace Clippy 单独清理，别和 FFI / Immix 的功能验收混在一起。

---

## P0 ｜马上收尾

### P0-1 通用动态 C FFI（已完成，未提交）

- [x] `crates/lumec_core/src/ffi.rs`：共享库加载、符号解析、显式 C 签名调用。
- [x] `crates/lumec_core/src/ffi.rs`：对齐且带边界检查的 FFI 缓冲区，支持结构体指针的手工布局。
- [x] `crates/lumec_core/src/ffi.rs`：C 回调包装；回调错误可查询；panic 不 unwind 穿过 C 边界。
- [x] `crates/lumec_core/src/evaluator.rs`：Lume 侧接入 `ffi_unsafe_load`、`ffi_symbol`、`ffi_unsafe_call`、`ffi_callback`、`ffi_buffer_*`。
- [x] 安全边界：加载与调用接口明确标为 `unsafe`；C 函数原型、指针有效期与回调保活由调用方负责，文档写死。
- [x] 文档与示例：新增 `docs/ffi.md`、`examples/ffi_strlen.lume`；更新 `docs/lume_spec.md`、`docs/immix-gc.md`、`README.md`。
- [x] 当前支持范围：默认 C ABI 的标量与指针；按值 struct/union、C 变参、非默认 ABI 暂不支持，已明确记为未做。
- [x] 验证：`examples/ffi_strlen.lume` 的 `run` / `check` 通过；`cargo test --workspace` 66 项通过；格式与 `git diff --check` 通过。

> 注意：原路线图里的 `demo_c_interop` 双向调用、manifest extern、JIT bitcode 属于**静态 / AOT 路径**，不是本轮完成的动态 FFI，见 P0-3。

### P0-2 Immix 并入 core（已完成，未提交）

- [x] 清理 `gc.rs` 临时调试打印；统一 header / payload 指针约定（`resolve` / `header`）。
- [x] `collect_with` visitor 传真实字段槽位；搬迁后根与字段都改写。
- [x] `pin` 保活对象、阻止搬迁，支持嵌套计数；新增作用域 pin guard 与 `LumeCore::gc_pin` / `gc_unpin`。
- [x] 修复非堆指针被 GC 内部误解引用；修复收集后分配字节统计不更新。
- [x] 更新 `docs/immix-gc.md` 与 `docs/ffi.md` 的 GC pin 契约。
- [x] 验证：`cargo test --workspace` 66 项通过；`lumec_immix` Clippy 通过；示例 `run` / `check` 通过。
- [x] 全 workspace Clippy 通过：既有 lint 已清理，见 P0-4。

### P0-3 静态 / AOT FFI（已完成）

- [x] `manifest.rs`：解析 `[[module]]`、`[ffi.extern]`、`[ffi.export]`，支持 `params/ret` 与 `signature` 两种 extern 写法。
- [x] `project.rs`：C 模块编译为 bitcode 与共享库；用 `llvm-nm --defined-only` 校验导出符号；路径相对 manifest 解析。
- [x] `main.rs`：新增 `header <manifest.toml>`；`run` / `jit` / `llvm-ir` / `check` 均支持 manifest；`build_mode = "jit"` 时 JIT 优先选 `entry`。
- [x] `jit.rs`：`load_bitcode()`、`register_builtin()` 与外部签名分派统一；`validate_calls` 对缺失符号/错误元数给出明确错误；外部调用按 C 函数真实类型做 i64/指针/浮点转换。
- [x] `jit.rs`：`register_dynamic_extern()` 用 `ffi.rs` 的 libffi trampoline 把共享库符号接入 JIT。
- [x] `evaluator.rs` / `typecheck.rs`：移除 `c_matmul` 桩，改为 manifest 驱动的真实 extern 分派。
- [x] `parser.rs` / `typecheck.rs` / `project.rs`：识别 `#[ffi_export]`，类型检查 C ABI 可映射性，并生成 C 头文件。
- [x] `evaluator.rs` / `jit.rs`：`ffi_export_ptr(name, signature)` 在解释器和 JIT 中把 Lume 函数包装成真实 C 回调指针。
- [x] 按值 struct/union、C 变参、非默认 ABI：动态 FFI / 头文件生成已支持；JIT `i64` 值 ABI 仍不承载按值聚合，已写入 `docs/ffi.md`。
- **验收**：`examples/demo_c_interop` 的 `run` 与 `jit` 均输出 `8256`（Lume→C→Lume）；`llvm-ir` 可见 `call i64 @c_matmul`；`check` / `header` 通过。

### P0-4 Clippy 基线债（已完成）

- [x] `codegen.rs`：`map` 改为显式 `if let`，`and_then(...map...)` 改为 `Option::zip`。
- [x] `evaluator.rs`：移除 `binary()` 中多余 `return`。
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 全绿，CI Clippy 门禁恢复严格模式。
- **说明**：新增 `manifest.rs` / `project.rs` / JIT extern 分派后也一并通过了严格 Clippy。

---

## P1 ｜小型服务器最小闭环（严格按依赖顺序）

### P1-1 时间原语（已完成，未提交）

- [x] `clock.rs` + `evaluator.rs` + `jit.rs`：增加 `now_ms() -> i64`（Unix epoch 毫秒）与 `mono_ms() -> i64`（单调时钟，专用于超时）；JIT 默认注册为 builtin。
- [x] `clock::with_clock` 注入假时钟；解释器和 JIT 单测用假时钟，不靠 `sleep`。
- [x] `typecheck.rs` 注册签名；VS Code 补全同步。
- **验收**：`cargo test --workspace` 68 项通过；假时钟可稳定复现时间读取。

### P1-2 定宽整数 + 二进制 buffer + memcpy（已完成）

- [x] 构造式转换 `u8(x)` / `u32(x)` 已接入类型检查、运行时与 JIT 零扩展 cast；越界 / 负数直接报错。
- [x] `typecheck::Type` 保留 `i8/i16/i32/u8/u16/u32/u64` 宽度，窄整数可隐式加宽到 `int`，反向赋值报错；JIT 以零扩展 `i64` 表示定宽值。
- [x] `buffer.rs`：GC 管理的字节数组，长度写入 payload 前缀，存活期间 pin；内置 `buf_new`、`buf_len`、`buf_get_u8` / `buf_set_u8`、`buf_get_u32` / `buf_set_u32`（小端）、`memcpy`、`memmove`、`str_to_buf`、`buf_to_str`。
- [x] JIT 注册 buffer 内置函数；句柄是 pin 后稳定地址，所有访问仍做边界检查；`str_to_buf` / `buf_to_str` 仍需字符串值，JIT 暂不提供。
- [x] 明确 `ffi_buffer_*` 与 GC buffer 的关系：前者是 FFI 本机内存，后者由 GC 管理并 pin；不要混用两套生命周期。
- [x] `memcpy` 禁止重叠；`memmove` 支持同一缓冲区内重叠复制，解释器与 JIT 均可调用。
- [x] 示例与文档：`examples/demo_buffer.lume`、`docs/lume_spec.md`、VS Code 补全；越界 / 负长度 / 无效 UTF-8 都有明确错误。
- **验收**：`cargo test --workspace` 全绿；`check` / `run` / `jit` / `llvm-ir examples/demo_buffer.lume` 通过；buffer 释放后 pin 计数归零。

### P1-3 rawptr / ptr<T> + unsafe 块

- [ ] 语法：`p: ptr<u8> = buf_ptr(buf)`；`unsafe ... end` 缩进块（与现有 if/while 风格一致）。
- [ ] `typecheck.rs` 跟踪 unsafe 上下文：块外解引用直接编译失败。
- [ ] 内置 / IR：`ptr_read_u8`、`ptr_write_u8`、`ptr_read_u32`、`ptr_write_u32`、`ptr_copy`、`ptr_offset`、`ptr_cast`。
- [ ] 安全规则：指针只能来自 buffer 或 FFI 回调；禁止整数直接转指针；FFI 传入的指针在回调期间视为已 pin，离开回调即失效；unsafe 块结束前结果必须写回 buffer 或复制出来。
- **验收**：安全代码里解引用编译失败；FFI + unsafe 读写 buffer 成功；越界访问有可诊断错误。

### P1-4 HashMap 容器

- [ ] core 原语 `Map`（开放寻址 / 线性探测，键支持 int 与 string，值为通用 Value），GC 需能 trace 键和值。
- [ ] `lib/std/map.lume` 包装：`map_new`、`map_put`、`map_get`、`map_delete`、`map_has`、`map_len`、`map_keys`。
- [ ] 类型 `HashMap<K, V>` 在 typecheck 中保留 K/V；键相等复用现有 `values_equal` / `is`。
- **验收**：10 万条插入 / 查找；删除后 GC 不悬挂；进程内字符串键哈希稳定。

### P1-5 Socket（最小闭环已完成，并发待做）

- [x] 主路径走动态 FFI：`examples/server_echo/src/net.c` 封装 `listen` / `accept` / `recv` / `send` / `close` / `last_error`，fd 与长度统一 `i64`；`manifest.toml` 用 `[ffi.extern]` 绑定，解释器通过 libffi 动态分派。
- [x] `sockaddr_in` 不进入 JIT；Lume 侧用 `ffi_buffer` 提供接收内存，`mono_ms()` 判定空闲超时并关闭监听 fd。
- [x] `test_echo.sh` + `test_echo.c` 回环验证 `hello lume`，CI 示例矩阵已接入。
- [ ] 非阻塞 + `poll` fd 集合，做单线程多客户端并发事件循环；当前每次只服务一个客户端。
- [ ] AOT/JIT 静态 extern 路径仍保留后续阶段；当前只维护一套动态 FFI 分派语义。
- [ ] 备选：`core_socket_*` Rust builtin 暂不实现，避免维护两套语义。
- **当前验收**：`examples/server_echo` 可启动并回显；单客户端 accept / recv / send / close 无 fd 泄漏；多客户端并发仍待补。

### P1-6 锁与超时

- [ ] `lock_new()`、`lock_acquire(lock, timeout_ms) -> bool`、`lock_release(lock, token)`；底层可用 Rust `Mutex::try_lock_for`，token 防止误释放。
- [ ] 超时基于 `mono_ms()`，返回 false 而不是阻塞。
- **验收**：超时路径单测通过；竞争场景下不会双持锁。

### P1-7 端到端 demo

- [ ] `examples/server_echo/`：manifest + `main.lume` + `net.c`；路由表用 HashMap，连接超时用锁 + `mono_ms()`。
- [ ] README 写清启动 / 测试方法；加入 `ci.yml` 的示例检查矩阵。
- **验收**：`lume-bootstrap run examples/server_echo/manifest.toml` 后 `nc localhost <port>` 能回显。

---

## P2 ｜胶水语言：多语言项目

- [ ] manifest v2：`[[module]]` 支持 `lang = lume | c | rust | zig`，每个语言一个“前端命令模板”，统一产出 `.bc`；`llvm-link` 合并，`llc` 做 AOT。动态 FFI 已完成，但**不等于** manifest / bitcode 路径完成。
- [ ] 类型映射表：C / Rust / Zig 基础类型 ↔ Lume 定宽类型与指针；`lume-build gen-header` 自动生成 C 头文件。
- [ ] 依赖与镜像：路径依赖 / git 依赖 / 镜像源（GitHub 不通时可用镜像），生成 lockfile。
- [x] 跨语言 GC 契约（实现与文档部分）：只传 i64 / 指针；`pin` 保活、阻止搬迁、嵌套计数；`gc_pin` / `gc_unpin` 与作用域 guard；`docs/ffi.md` 写清“C 若返回后继续持有地址，必须延长 pin 生命周期”。
- [ ] 跨语言 GC 契约（验收部分）：补端到端测试，覆盖外部 C 长期持有指针、回调保活、忘记 unpin 的诊断。
- [ ] 风险记录：Rust / Zig 输出可链接 bitcode 的工具链限制，先保证 C 路径稳，再逐个接。
- **验收**：`examples/mixed_project` 同时含 `.lume + .c + .rs`（或 `.zig`），JIT 与 AOT 两种模式都跑通；生成的头文件可被 C 编译器直接 include。

---

## P3 ｜GPU 加速

- [x] 前置事实：动态 FFI 可按普通共享库调用 CUDA、Level Zero、HIP、Vulkan 的宿主 API；本次未做专用 SDK 封装、未做 GPU 加速、未在 GPU SDK / 硬件环境验证。
- [ ] 阶段 A：FFI 调 CUDA / HIP driver；`gpu_init`、`kernel_load`、`gpu_launch(grid, block, args_ptr, n)`、`gpu_sync()`，错误码统一。
- [ ] 阶段 B：`lib/std/gpu.lume` 包装；`gpu_array` 独立于 GC 堆，显式 `copy_to_device` / `copy_from_device`，不做自动统一内存。
- [ ] 阶段 C：编译器把纯数据并行的 map / reduce 循环 lowering 成 kernel；最后做，不阻塞前两阶段。
- [ ] 风险：macOS 无 CUDA（Metal 路径不同）；先 Linux + CUDA，驱动层用 trait 抽象。
- **验收**：vector-add 1000 万元素，CPU / GPU 结果逐项一致并输出计时。

---

## 横切工作

- [x] 文档：`docs/ffi.md` 已补动态 FFI、manifest 静态/AOT、头文件生成与 `#[ffi_export]`；`docs/lume_spec.md` 已同步 FFI、时间、定宽构造、GC buffer 与 JIT 限制；`docs/immix-gc.md` 与 `README.md` 已更新。
- [x] 文档：`docs/server.md` 已记录 socket 最小闭环、C ABI、超时语义与当前范围；`docs/gpu.md` 仍待做。
- [ ] 文档：`docs/gpu.md`；`docs/lume_spec.md` 继续补 unsafe / ptr / HashMap 语法。Wiki 同步 CI 会自动把 `docs/` 推送到 GitHub Wiki。
- [x] 工具链：VS Code 语法、补全、主题、图标与网页编辑器已可用；REPL 无需改；补了 `now_ms` / `mono_ms` / `buf_*` / `memcpy` / `memmove` / `ffi_struct*` 与 `#[ffi_export]` / `ffi_export_ptr` 补全。
- [x] 工具链：实现 `lume-bootstrap --lsp` stdio 服务，提供关键字/类型/内置函数/函数/局部变量补全、hover 与 parser/type checker 实时诊断；VS Code 扩展连接语言服务。
- [x] 工具链：完善 TextMate 高亮的 FFI / buffer / math 内置函数和 `#[ffi_export]` 属性；manifest `docs` 子命令生成模块与外部签名 Markdown；`scripts/test-examples.sh` 统一执行示例检查、输出断言和文档生成验证。
- [ ] 工具链：VS Code 补全跟上 `ptr_*`、HashMap、GPU 等新关键字。
- [x] CI：三平台构建 / Release / Pages / Wiki 同步；LLVM 23 安装脚本已修（Linux Polly、Windows 流式下载 + 7z + 缓存、macOS runner）；`examples/demo_buffer.lume`（解释器）、`examples/demo_buffer_jit.lume`（run/jit）、`examples/demo_c_interop/manifest.toml` 的 check/header/run/jit/llvm-ir、以及 `examples/server_echo/test_echo.sh` 回环测试已加入示例矩阵；Clippy 门禁恢复严格模式；stable/nightly rustfmt 差异已按 CI 的 stable rustfmt 对齐。
- [ ] CI：后续阶段继续把新示例加入 `ci.yml`。
- [x] 验证（本轮）：`cargo test --workspace` 78 项 core 测试 + 9 项 Immix 测试通过；stable rustfmt / strict Clippy / `git diff --check` 全绿；示例矩阵 `ffi_strlen`、`demo_buffer`、`demo_buffer_jit`（run/jit 同为 `50727435`）、`demo_c_interop`（run/jit 同为 `8256`）与 `server_echo` 回显测试全部通过。
- [x] 验证（辅助工具）：`cargo test --workspace` 85 项 core 测试 + 9 项 Immix 测试通过；严格 Clippy、格式检查和 VS Code 扩展编译通过；示例自动测试脚本及独立 LSP stdio 握手/诊断/补全/hover 测试通过。
- [ ] 验证（剩余）：GPU / socket 等后续阶段示例尚未加入。

---

## 现在就做的三步

1. **你**：决定何时提交当前工作区（P0 静态/AOT FFI、Clippy、P1 时间 + buffer、地基补完、Immix、socket 最小 echo、文档与示例）。
2. **建议下一步**：把 P1-5 的顺序 accept 循环升级为 `poll` fd 集合，支持多客户端并发；随后再进 rawptr / HashMap。
3. **顺序红线**：socket 当前只维护动态 FFI 一套语义；锁超时仍等锁原语；rawptr 不要绕过 buffer 生命周期；按值聚合在 JIT 外部 ABI 中仍不承载，socket 的 `sockaddr_in` 先用 `ffi_buffer` / `u32()` 拼装。
