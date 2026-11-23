# Lume 开发路线图

> 2026-11-23 更新。旧版是语言设计稿；完整语法规范保留在 `docs/lume_spec.md`，本文件只维护“下一步做什么、做到什么算完”。
>
> 本轮勾选依据用户口述进度：通用动态 C FFI 与 Immix 合并问题修复**已完成但未提交**，工作区中其他已有改动保留，本次只更新本文件，没有碰 `.rs` / `Cargo.toml` / `docs`。

## 状态快照

- 已完成（未提交）：通用动态 C FFI（共享库加载、符号解析、显式 C 签名调用、边界检查缓冲区、C 回调、回调错误查询与 panic 隔离）；Immix 并入 core 及 pin / 非堆指针 / 分配统计修复；FFI、回调与 GC pin 文档；libc FFI 示例。
- 已完成（CI 侧）：三平台构建 / Release / Pages / Wiki 同步；LLVM 23 安装脚本修复（Linux 补 `libpolly-23-dev`，Windows 改流式下载 + 7z + 缓存，macOS runner 改 `macos-15`）。
- 待做：静态 / AOT FFI（manifest v2、`[ffi.extern]` / `[ffi.export]`、`#[ffi_export]`、头文件生成、LLVM JIT 接通动态 FFI）；定宽整数、GC buffer、rawptr / unsafe、HashMap、时间原语、socket；多语言项目工具链；GPU SDK 封装与硬件验证。
- 已知技术债：全 workspace Clippy 仍被 `codegen.rs` / `evaluator.rs` 的既有 lint 阻塞；本次 FFI / Immix 代码未新增 Clippy 报错。

## 排序原则

1. 先地基后上层：时间 → buffer → rawptr → HashMap → socket → 服务器 demo。
2. 每块必须有可运行示例 + 单测，不堆“只设计不落地”的文档。
3. 跨语言边界坚持 C ABI（i64 / 指针），不穿透 GC；GPU 先显式搬运，不自动统一内存。
4. 动态 FFI 已完成，但它不等于静态 / AOT FFI；socket 可走动态 FFI，仍先等 `mono_ms()` 和 buffer 落地。
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
- [ ] 全 workspace Clippy 通过：仍被既有 lint 阻塞，见 P0-4。

### P0-3 静态 / AOT FFI（仍未完成）

- [ ] `crates/lumec_core/src/main.rs`：manifest 从只读 `path =` 升级为解析 `[[module]]`、`[ffi.extern]`、`[ffi.export]`；clang 产出的 `.bc` 纳入构建图，并校验导出符号是否存在。
- [ ] `crates/lumec_core/src/jit.rs`：把 `load_bitcode()`、`register_builtin()` 与 evaluator 的外部调用分派统一；`validate_calls` 对缺失符号给出明确错误。
- [ ] 移除 `evaluator.rs` / `typecheck.rs` 里的 `c_matmul` 桩，改成真实 extern 分派。
- [ ] `#[ffi_export]`：parser / typecheck / codegen 标记 C-ABI 函数，符号按 manifest 导出；先支持手工模板生成头文件。
- [ ] LLVM JIT 接通动态 FFI：目前 JIT 路径尚未复用 `ffi.rs` 的加载 / 调用能力。
- [ ] 按值 struct/union、C 变参、非默认 ABI。
- **验收**：`examples/demo_c_interop` 跑通 Lume→C→Lume 双向调用；JIT 与 AOT 两种模式都能解析外部符号。

### P0-4 Clippy 基线债（不属于本次 FFI / Immix）

- [ ] `crates/lumec_core/src/codegen.rs:161`、`180`：`map` 改 `inspect`（`clippy::manual_inspect`）。
- [ ] `crates/lumec_core/src/codegen.rs:225`、`226`：`and_then(...map...)` 改 `zip`（`clippy::manual_option_zip`）。
- [ ] `crates/lumec_core/src/evaluator.rs:460`、`464`、`466`：移除 `return`（`clippy::needless_return`）。
- [ ] 清理后重跑 `cargo clippy --workspace --all-targets -- -D warnings`，并恢复 CI 的 Clippy 门禁。
- **说明**：这是工作区既有 lint；本次 FFI / Immix 新增代码未触发新 Clippy 错误。要么一次性修掉，要么单独决定放宽 CI，别改坏功能验收。

---

## P1 ｜小型服务器最小闭环（严格按依赖顺序）

### P1-1 时间原语（最小、独立，建议第一个做）

- [ ] `builtin.rs` / `evaluator.rs` / `jit.rs` 增加 `now_ms() -> i64`（Unix epoch 毫秒）与 `mono_ms() -> i64`（单调时钟，专用于超时）。
- [ ] 时钟可注入，单测用假时钟，不靠 `sleep`。
- [ ] `typecheck.rs` 注册签名；VS Code 补全同步。
- **验收**：`mono_ms()` 不回退；超时逻辑用假时钟可稳定复现。

### P1-2 定宽整数 + 二进制 buffer + memcpy

- [ ] 类型层：`Type::UInt(8|16|32)` / `Int(8|16|32)`；保留 `int = i64`。注解写 `x:u8 = ...`，字面量先走构造式转换 `u8(255)` / `u32(0xFF)`，避免大改 lexer。
- [ ] 解释器 `Value` 增加定宽变体；JIT 层用 i8 / i32 存储、类型系统约束宽度，转换 builtin 负责越界检查。
- [ ] buffer：GC 管理的字节数组，长度写入对象 header/payload prefix。内置 `buf_new(n)`、`buf_len`、`buf_get_u8` / `buf_set_u8`、`buf_get_u32` / `buf_set_u32`（明确小端）、`memcpy(dst, src, n)`、`str_to_buf`、`buf_to_str`。
- [ ] 明确 `ffi_buffer_*` 与 GC buffer 的关系：前者是 FFI 本机内存，后者由 GC 管理；不要混用两套生命周期。
- [ ] `memcpy` 重叠行为明确：禁止重叠，另做 `memmove`。
- **验收**：越界 / 负长度 / 长度不匹配都报错；u8 / u32 往返正确；buffer 能被 GC 回收。

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

### P1-5 Socket

- [ ] 主路径走已完成的动态 FFI：C 侧 `net.c` 暴露 `lume_socket` / `bind` / `listen` / `accept` / `recv` / `send` / `close` / `poll`；Lume 侧用 `ffi_unsafe_load` / `ffi_symbol` / `ffi_unsafe_call`，`sockaddr_in` 用 FFI buffer 或后续 GC buffer + u32 拼装。
- [ ] AOT / manifest 阶段再切到 `[ffi.extern]` 声明；不要同时维护两套分派语义。
- [ ] 非阻塞 + `poll` 做单线程事件循环；空闲连接用 `mono_ms()` 判定超时关闭。
- [ ] 备选：`core_socket_*` Rust builtin 只在 FFI 未通时用于验证语义，二选一，避免维护两套。
- **验收**：`examples/server_echo` 可启动；多个客户端并发 echo；accept / recv / send / close 无 fd 泄漏。

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

- [x] 文档：`docs/ffi.md` 与 GC pin 契约已补；`docs/lume_spec.md` 已同步；`docs/immix-gc.md` 与 `README.md` 已更新。
- [ ] 文档：`docs/server.md`、`docs/gpu.md`；`docs/lume_spec.md` 继续补 unsafe / ptr / buffer / HashMap / 时间语法。Wiki 同步 CI 会自动把 `docs/` 推送到 GitHub Wiki。
- [x] 工具链：VS Code 语法、补全、主题、图标与网页编辑器已可用；REPL 无需改。
- [ ] 工具链：VS Code 补全跟上 `now_ms` / `mono_ms`、`buf_*`、`ptr_*`、HashMap、GPU 等新关键字。
- [x] CI：三平台构建 / Release / Pages / Wiki 同步；LLVM 23 安装脚本已修（Linux Polly、Windows 流式下载 + 7z + 缓存、macOS runner）。
- [ ] CI：每阶段把新示例加入 `ci.yml`；清理 P0-4 的 Clippy 基线债后恢复门禁。
- [x] 验证（本轮）：`cargo test --workspace` 66 项通过；格式与 diff 检查通过；`lumec_immix` Clippy 通过；`examples/ffi_strlen.lume` 的 `run` / `check` 通过。
- [ ] 验证（剩余）：`cargo clippy --workspace --all-targets -- -D warnings` 全绿。

---

## 现在就做的三步

1. **你**：决定何时提交当前工作区（FFI / Immix / 文档 / 示例）。提交前不用再等我改 `.rs`，本轮只动了 `todo.md`。
2. **建议并行**：从 P1-1 时间原语 + P1-2 buffer 开始，这两块不碰你刚完成的 FFI / GC 文件；socket 已解除“没有 FFI”阻塞，但仍等 `mono_ms()` 和 buffer；rawptr 等 buffer 落地后再动。
3. **顺序红线**：没有 buffer 不做 rawptr；动态 FFI 已通但 socket 仍需要 buffer + 单调时钟；没有单调时钟不做锁超时；静态 / AOT FFI 和 GPU SDK 封装都是独立后续项，不要卡住 P1。
