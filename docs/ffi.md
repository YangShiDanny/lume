# C FFI：动态调用、静态 / AOT 与导出

Lume 的解释器提供显式的动态 C FFI，可用同一套 C ABI 调用常规共享库和
CUDA、Level Zero、HIP、Vulkan 等 SDK 的宿主 API。这里只加载并调用 SDK 导出
的函数，不做 GPU 加速，也不自动管理 GPU 资源。

> **安全边界：** 加载共享库会运行其初始化代码；调用 C 函数时，错误的符号
> 签名、无效指针或不满足 SDK 约定都可能导致进程崩溃或内存破坏。Lume 将
> 这些入口明确命名为 `ffi_unsafe_*`。仅加载可信库，并核对头文件中的 C
> 原型、平台 ABI 与对象生命周期。

## 在 Lume 中调用 C 函数

签名格式为 `"[abi:]返回类型(参数类型, ...)"`。支持 `void`、`i8/i16/i32/i64`、
`u8/u16/u32/u64`、`isize/usize`、`f32/f64` 和 `pointer`（也可写 `ptr`）。
`isize/usize` 按运行平台指针宽度处理。`void` 只能用作返回类型。可选 ABI 前缀
支持 `default`/`c`、`sysv`/`unix`、`cdecl`、`stdcall`、`win64`、`fastcall`；不可用于
当前平台的名称会直接报错，不会静默退回默认 ABI。

按值结构体写作 `{i32,f64}`，按值 union 写作 `<i32,f64>`。结构体字段按声明顺序
参与 ABI 布局；union 仅支持标量成员，并按最大成员表示。可用 `ffi_struct(type, values)`
构造值、`ffi_struct_get(value, index)` 读取字段；这些值只能用于动态 FFI 调用和回调，
不会穿透 JIT 的 `i64` 值 ABI。

C 变参签名把实际变参类型写在 `...` 之后，例如 `i32(pointer, ..., i32, f64)`：
`...` 前的固定参数至少一个，`...` 后必须列出每次调用实际传入的变参类型。小整型和
`f32` 会按 C 默认参数提升为 `i32`/`f64`。Lume 侧回调不支持变参签名。

以下示例加载 Linux C 运行库并调用 `abs`：

```lume
fn main() -> int
    c_runtime:any = ffi_unsafe_load("libc.so.6")
    abs_fn:any = ffi_symbol(c_runtime, "abs")
    result:any = ffi_unsafe_call(abs_fn, "i32(i32)", [-42])
    print(result)
ret 0
```

使用 `ffi_unsafe_load(path)` 打开共享库，`ffi_symbol(library, name)` 查找符号，
`ffi_unsafe_call(symbol, signature, args)` 按签名调用。参数使用 Lume 数组；
整数、浮点数、字符串（转换为本次调用期间有效的 NUL 结尾 C 字符串）和指针
会按签名做检查。返回的无符号 64 位值保留为 Lume `UInt`，指针返回值是 opaque
指针值，可作为后续 FFI 调用的 `pointer` 参数。`ffi_null()` 创建空指针。

通过 `ffi_buffer(size, alignment)` 创建零初始化、显式对齐的本机内存区域。用
`ffi_buffer_read(buffer, offset, type)` 和
`ffi_buffer_write(buffer, offset, type, value)` 按偏移读写标量/指针字段；用
`ffi_buffer_write_cstr` / `ffi_buffer_read_cstr` 写入或读取有界的 NUL 结尾字符串。
缓冲区访问会做范围检查且不要求字段自然对齐。把 buffer 作为 C `pointer` 参数
传入时，C 可以在调用期间读写该内存；若 C 保留指针，必须让 buffer 至少与
C 的使用期一样长。手工布局 C struct 时要按目标头文件计算每个字段偏移、
大小、对齐和嵌套指针生命周期。

例如 CUDA Driver API 可由调用方按已安装 SDK 的原型配置调用（这里仅展示
宿主侧 API，不会启动 kernel）：

```lume
fn query_cuda_devices() -> int
    driver:any = ffi_unsafe_load("libcuda.so.1")
    get_count:any = ffi_symbol(driver, "cuDeviceGetCount")
    count_buffer:any = ffi_buffer(4, 4)
    status:any = ffi_unsafe_call(get_count, "i32(pointer)", [count_buffer])
    count:any = ffi_buffer_read(count_buffer, 0, "i32")
    print(status)
    print(count)
ret 0
```

CUDA、Level Zero、HIP、Vulkan 库名称取决于系统安装位置。例如 Linux 常见名称：

| API | 常见库 |
| --- | --- |
| CUDA Driver API | `libcuda.so.1` |
| Level Zero Loader | `libze_loader.so.1` |
| HIP Runtime | `libamdhip64.so` |
| Vulkan Loader | `libvulkan.so.1` |

Windows 常见 DLL 名称分别为 `nvcuda.dll`、`ze_loader.dll`、`amdhip64.dll`、
`vulkan-1.dll`。请以目标机器上的 SDK 文档与实际导出符号为准；不同平台的
typedef 宽度和函数原型可能不同。GPU API 的 opaque handle 要依头文件声明写成
`pointer` 或适当的整数类型。

## C 回调

用 `ffi_callback(signature, closure)` 将 Lume 闭包包装成 C 可调用函数指针：

```lume
fn make_callback()
    ret ffi_callback("i32(i32)", fn(value:int) -> int => value + 1)
```

将返回的 callback 值作为 `pointer` 参数传给接受函数指针的 C API。**只要 C
代码可能再次调用该回调，就必须一直保留 callback 值**；释放后调用其地址是
未定义行为。C 回调在 Lume 中发生错误或 panic 时不会让 Rust unwind 穿越 C
边界：返回类型的零值会交给 C，错误可用 `ffi_callback_error(callback)` 取出
（无错误时返回空字符串）。若 C API 在另一个线程调用回调，必须确保被捕获的
Lume 状态以及该 API 本身允许跨线程访问。

## Rust embedding API

Rust 宿主可使用 `lumec_core::ffi` 的显式签名 API；原生函数调用是 `unsafe`，
加载库也是 `unsafe`：

```rust,ignore
use lumec_core::ffi::{FfiSignature, FfiValue, ForeignLibrary};

let library = unsafe { ForeignLibrary::open("libc.so.6")? };
let abs = library.symbol("abs")?;
let signature = FfiSignature::parse("i32(i32)")?;
let result = unsafe { abs.call(&signature, &[FfiValue::Signed(-42)])? };
```

`ForeignSymbol` 持有共享库的引用，因此符号不会因 library handle 被提前释放而
悬空。对调用期间的 C 字符串参数，FFI 层会保留 NUL 结尾缓存直到函数返回。
除指针和字符串外，调用参数必须与签名声明的数值类型一致并在其范围内。

## GC 对象 pin

Immix GC 的 `pin` 会增加 pin 计数，并让 pinned 对象及其字段引用的对象在收集时
保持存活且不会搬迁；每次 pin 都需要配对一次 unpin。Rust 宿主优先使用作用域
guard，避免遗忘 unpin：

```rust,ignore
let payload = heap.alloc_value(value, type_id);
let pinned = heap.pin_guard(payload)?;
unsafe {
    call_native_api(pinned.as_ptr());
}
```

如果 C 会在函数返回后继续持有该地址，必须延长 pin 的生命周期，直到 C 确认
不再访问；不能让作用域 guard 提前释放。`LumeCore::gc_pin` /
`gc_unpin` 提供与 JIT GC 堆配套的显式引用计数 API。GC 指针传给任意 FFI 之前
必须 pin；仅登记为普通 root 仍可能在收集期间搬迁。

## 静态 / AOT FFI（manifest）

`lume-bootstrap` 现在支持项目 manifest 的 `[[module]]`、`[ffi.extern]` 与
`[ffi.export]`。`examples/demo_c_interop/manifest.toml` 是最小可运行示例：

```toml
[project]
name = "demo_c_interop"
build_mode = "jit"

[[module]]
path = "src/main.lume"
lang = "lume"
export = ["calc_sum", "entry"]

[[module]]
path = "src/calc.c"
lang = "c"
cflags = ["-O1"]
export = ["c_matmul"]

[ffi.export]
export = ["calc_sum"]

[ffi.extern]
c_matmul = { params = ["i64", "pointer"], ret = "i64" }
```

- `run <manifest.toml>`：把 C 模块编译成 LLVM bitcode 与共享库，加载共享库后
  在解释器中按 `[ffi.extern]` 的显式签名调用外部符号。
- `jit <manifest.toml>` / `llvm-ir <manifest.toml>`：把 bitcode 链接进 JIT
  模块，按 C 函数真实类型做整数/指针/浮点参数与目标转换，并在生成代码前校验
  外部符号与调用元数。若 `[ffi.extern]` 提供 `library = "..."`，JIT 会通过
  `ffi.rs` 的 libffi trampoline 动态解析该符号，而不依赖 bitcode。
- `header <manifest.toml>`：根据 `#[ffi_export]` 与 `[ffi.export]` 生成 C
  头文件，包含导出的 Lume 函数和声明的外部 C 函数。

构建时会用 `llvm-nm --defined-only` 检查 C 模块的 `export` 列表，缺失符号会
直接报错。C 模块应把需要导出的函数标记为对外可见（Windows 下可加
`__declspec(dllexport)`）。

### 从 C 回调 Lume

解释器和 JIT 均可用 `ffi_export_ptr(function_name, signature)` 生成指向 Lume
函数的 C 函数指针，交给 C 后按常规 C ABI 回调：

```lume
#[ffi_export]
fn calc_sum(n:int) -> int
ret n * (n + 1) / 2

fn main() -> int
    callback:any = ffi_export_ptr("calc_sum", "i64(i64)")
ret c_matmul(128, callback)
```

`examples/demo_c_interop` 的 interpreter 与 JIT 路径都输出 `8256`
（`calc_sum(128)`）。JIT 在编译前收集 `ffi_export_ptr` 的 `(函数, 签名)`，编译后为
目标函数注册 libffi trampoline；C 回调直接调用已编译的 Lume 函数，不经过解释路径。
这样同一份 manifest 同时覆盖 Lume→C 与 C→Lume 两个方向，也覆盖 bitcode 静态链接
和动态 FFI 两条分派路径。JIT 回调仅支持当前线程的同步调用；C 若跨线程回调，仍需
由宿主保证状态与 GC 安全。

## 当前限制

- 支持的 C 类型包括标量、指针和由标量字段组成的 struct/union；不支持 bitfield、
  long double、向量 ABI、嵌套聚合以及超过 128 字节的按值聚合。
- union 按最大标量成员表示；Lume 侧只暴露成员 0 的读取。需要精确 C 布局时，仍可
  用 `ffi_buffer` 按头文件手工构造结构体指针。
- C 变参必须在签名中显式列出每个实际变参类型；不接受 `i32(...)` 这种只有省略号的
  声明。Lume 函数不能作为变参回调。
- ABI 名称解析到当前平台的 libffi ABI。x86-64 Windows 的 `stdcall` 与 Win64 默认
  ABI 等价；32 位 Windows 才提供独立的 `stdcall`/`fastcall`。不可用组合会报错。
- 按值聚合与变参调用走解释器/libffi 动态 FFI；JIT 的外部 `i64` ABI 仍只携带标量
  与指针。JIT 已支持 buffer builtin、`ffi_export_ptr` 回调和 manifest bitcode 外部
  符号，但不会把按值聚合塞进 JIT 的 `i64` 值栈。
- 目前没有 CUDA/HIP/Vulkan/Level Zero 专用封装、设备枚举或加速后端。调用库函数
  需要应用自行准备运行时、上下文、设备与资源，并按 SDK 顺序释放。
- 头文件是按 manifest 生成的手工模板，不解析 C 头文件，也不生成 Rust/Zig 绑定。
