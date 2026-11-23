# Immix 垃圾回收器（`lumec_immix`）

`crates/lumec_immix` 是一个独立的 Immix 风格 mark-region 收集器 crate。它已经在根 `Cargo.toml` 的 workspace 中，但**尚未接入 `lumec_core`**：现有 `lumec_core::gc::GcHeap` 仍保持原样。本文描述它当前提供的能力、API 语义，以及后续接进 core 时的注意事项。

## 设计概览

- 堆由 **block** 组成，每个 block 切成固定大小的 **line**（默认 256 字节 × 256 行 = 64 KiB）。
- 小对象在普通 block 里 **bump 分配**，按 line 记账。
- 收集分两步：从根集合精确标记对象图；然后
  - 整条没有存活对象的 line 直接标记为空闲并复用；
  - 存活对象占比低于一半的碎片 line 上的对象被 **搬迁（evacuation）** 到新位置，原处留转发指针。
- 搬迁后自动改写根和对象字段中的指针，因此对象地址可能变化。
- 超过一个 block 容量的对象使用 **专用 block**，死亡时整块释放，不参与搬迁。

不做的事：并发 / 增量标记、写屏障、跨代引用、保守栈扫描、弱引用和 finalizer。这些需要 core 侧配合，留到接入时处理。当前实现不做碎片整理以外的压缩，所以不需要写屏障。

## 配置常量与类型

| 名称 | 默认值 | 说明 |
| --- | --- | --- |
| `DEFAULT_LINE_SIZE` | 256 | line 字节数，必须是 8 的正倍数 |
| `DEFAULT_LINES_PER_BLOCK` | 256 | 每 block 的 line 数 |
| `DEFAULT_COLLECTION_THRESHOLD` | 1 MiB | `should_collect()` 触发阈值 |

- `ImmixHeap::new()` / `ImmixHeap::with_options(line_size, lines_per_block)`：建堆，后者会对参数做断言校验。
- `Header`：每个对象起始处的元数据（`#[repr(C)]`），用户负载紧跟其后。可通过 `total_size()`、`payload_size()`、`type_id()`、`line_count()`、`is_marked()`、`is_pinned()` 读取。
- `ImmixError`：`ZeroBlock`、`InvalidLineSize`、`ObjectTooLarge`、`NullRoot`、`RootStackUnderflow`、`InvalidObject`。
- `CollectStats`：`objects_before/after`、`bytes_before/after`、`reclaimed_lines`、`evacuated_objects`。

## 分配与根

```rust
use lumec_immix::ImmixHeap;

let mut heap = ImmixHeap::new();
let obj = heap.alloc(64, /* type_id */ 1).unwrap();
heap.push_root(obj).unwrap();
```

关键 API：

- `alloc(payload_size, type_id) -> Result<*mut u8, ImmixError>`：返回**对象起始指针（header 地址）**，不是负载地址。负载用 `ImmixHeap::payload::<T>(obj)` 取得。
- `alloc_value::<T>(value, type_id)`：把 Rust 值写入负载，返回对象指针。
- `push_root(ptr)` / `pop_root()`：登记 / 弹出 GC 根；`pop_root()` 返回的是**已解析的当前有效地址**。
- `roots()`：返回根切片，内容是原始指针；收集后应通过 `resolve()` 解析再用。
- `should_collect()`：`bytes_since_gc >= 阈值` 时为 `true`。`set_collection_threshold(bytes)` 可调整，设为 0 可禁用自动触发判断。
- `bytes_allocated()` / `bytes_since_gc()` / `block_count()` / `line_size()` / `lines_per_block()` / `block_size()`：状态查询。

## 收集与对象图遍历

```rust
let stats = heap.collect_with(|object, visit| unsafe {
    if ImmixHeap::header(object).type_id() == PARENT_TYPE {
        // 必须传对象字段的“槽位”，不能传局部拷贝。
        visit(ImmixHeap::payload::<*mut u8>(object));
    }
});
println!("reclaimed {} lines, moved {} objects", stats.reclaimed_lines, stats.evacuated_objects);
```

`collect_with` 的 `trace` 回调对每个存活对象调用一次，第二个参数是可变指针槽的 visitor。收集器会：

1. 从 `roots` 出发标记对象图；
2. 搬迁碎片 line 上未 pin 的单 line 对象；
3. 改写所有根以及 visitor 交给它的字段槽位；
4. 回收空闲 line 和死亡的专用大对象块。

**最容易踩的坑**：visitor 需要真实字段槽位 `&mut *mut u8`。写成 `let mut child = *ptr; visit(&mut child);` 只会改局部变量，对象字段不会更新，搬迁后就是悬垂指针。

`collect()` 是 `collect_with(|_, _| {})` 的简写，**不追踪对象内部指针**。只有当你确定所有对象都没有子指针时才用它，否则子对象会被当作垃圾回收。

## 指针解析与固定对象

收集后旧指针可能指向转发指针，统一用 `resolve()` 取当前地址（最多跟随 16 跳）：

```rust
let current = unsafe { ImmixHeap::resolve(old_ptr) };
```

- `pin(obj)` / `unpin(obj)` / `is_pinned(obj)`：固定对象，避免 FFI 持有的对象被搬迁。被固定的对象仍参与标记，但不会被移动。
- `payload::<T>(obj)` / `header(obj)` / `header_mut(obj)`：访问负载和元数据，均为 `unsafe`，调用者需保证对象仍存活且类型正确。

注意：专用大对象 block 里的对象也不参与搬迁，但死亡时整块释放，不要把长期指针指向其中的对象。

## 接入 `lumec_core` 时的待办

1. 用 `ImmixHeap` 替换 `lumec_core::gc::GcHeap`，或做一层适配保持现有调用点不变。
2. 所有读取 GC 指针的地方统一走 `resolve()`；`collect_with` 的 visitor 传真实字段槽位。
3. 接上 JIT / builtin 的分配接口：`alloc` 返回 header 地址，core 侧需要按约定偏移取负载。
4. 把 core 的根集合（局部变量、栈、全局）完整登记到 `push_root`，否则会被误回收。
5. 若需要并发 / 增量或搬运跨 line 对象，再补写屏障与读屏障；目前未实现。

## 构建与测试

```bash
cargo test -p lumec_immix
cargo clippy -p lumec_immix -- -D warnings
cargo fmt -p lumec_immix
```

实现文件：`crates/lumec_immix/src/lib.rs`。当前 9 项测试全部通过，覆盖分配、line 回收、碎片整理、根与字段改写、pin、大对象和阈值触发。
