# 动态 C FFI

Lume 的解释器提供显式的动态 C FFI，可用同一套 C ABI 调用常规共享库和
CUDA、Level Zero、HIP、Vulkan 等 SDK 的宿主 API。这里只加载并调用 SDK 导出
的函数，不做 GPU 加速，也不自动管理 GPU 资源。

> **安全边界：** 加载共享库会运行其初始化代码；调用 C 函数时，错误的符号
> 签名、无效指针或不满足 SDK 约定都可能导致进程崩溃或内存破坏。Lume 将
> 这些入口明确命名为 `ffi_unsafe_*`。仅加载可信库，并核对头文件中的 C
> 原型、平台 ABI 与对象生命周期。

## 在 Lume 中调用 C 函数

签名格式为 `"返回类型(参数类型, ...)"`。支持 `void`、`i8/i16/i32/i64`、
`u8/u16/u32/u64`、`isize/usize`、`f32/f64` 和 `pointer`（也可写 `ptr`）。
`isize/usize` 按运行平台指针宽度处理。`void` 只能用作返回类型。

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

## 当前限制

- 只支持默认 C calling convention 与列出的标量/指针类型；暂不支持 C 可变参数、
  按值结构体/union、long double、向量 ABI、非默认 calling convention。
- 结构体指针可作为 `pointer` 传递；Rust 宿主需自行保证 `repr(C)` 布局；Lume
  可用 `ffi_buffer` 按头文件布局构造简单对象，但不提供自动 struct/union ABI 布局。
- 目前没有 CUDA/HIP/Vulkan/Level Zero 专用封装、设备枚举或加速后端。调用库函数
  需要应用自行准备运行时、上下文、设备与资源，并按 SDK 顺序释放。
- 此 FFI builtin 在当前解释器中执行；LLVM JIT 的 IR 目前没有通用动态符号调用
  lowering。
