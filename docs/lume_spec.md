# Lume 语言说明

本文介绍当前 `lume-bootstrap` 解析和执行的 Lume 语法。Lume 仍在开发中，
这里描述的是当前实现，不承诺所有未来版本都保持兼容。

## 第一个程序

```lume
# main 是程序入口
fn main() -> int
    greeting:string = "你好，Lume！"
    print(greeting)
ret 0
```

将代码保存为 `hello.lume`，在项目根目录运行：

```bash
cargo run -p lumec_core -- run hello.lume
```

先运行 `cargo build --workspace` 可单独构建二进制，此后可直接执行：

```bash
./target/debug/lume-bootstrap run hello.lume
```

检查语法但不运行：

```bash
cargo run -p lumec_core -- check hello.lume
```

源文件必须含有 `main`、`entry` 或 `boot` 入口函数。运行时会按 `main`、
`entry`、`boot` 的顺序选择第一个存在的函数；当前命令行不支持入口参数。

## 模块导入

使用模块级 `import` 导入相对路径 `.lume` 文件；不带扩展名时自动补上
`.lume`。文件只加载一次，循环导入不会重复解析：

```lume
import "math.lume"

fn main() -> int
ret twice(21)
```

导入文件中的函数和当前文件处于同一个全局函数命名空间，暂不支持别名、
选择性导入或命名空间隔离。

## 注释与代码块

`#` 开始单行注释；字符串中的 `#` 不会开始注释。没有块注释。

```lume
# 这是注释
text:string = "井号 # 在引号内是普通字符"
```

函数、`if` 和 `while` 的语句通过缩进放在块中。正常执行到块末尾时使用
`end` 结束；`ret` 返回并结束整个函数，不需要再用 `end` 结束函数。

```lume
fn absolute(value:int) -> int
    if value < 0
        ret 0 - value
    end
ret value
```

## 函数

使用 `fn` 声明函数，参数和返回类型标注都是可选的：

```lume
fn add(a:int, b:int) -> int
ret a + b

fn say_hi()
    print("你好")
end
```

当前运行时按照声明的参数数量检查函数调用。命令行入口应为无参数函数。

## 变量和赋值

用 `name:type = value` 声明变量并初始化；普通 `=` 赋值要求变量已经声明。
`:=` 是海象赋值，更新已有变量并返回赋入的值。

```lume
count:int = 0
count = count + 1

if count := count + 1
    print(count)
end
```

`check` 和 `run` 会执行基础静态检查，包括重复函数/变量、未声明变量、
基础类型不匹配、函数调用参数数量和标注类型，以及内置函数参数数量。
类型系统仍不完整；数组元素类型、字段 schema、流敏感控制流和泛型约束尚未
完整建模，未知类型会按 `any` 处理。

## 表达式和运算符

支持整数、浮点数、字符串、布尔值、数组、变量、命名函数调用、数组索引、
一元运算和二元运算。浮点数使用带小数点的格式，例如 `2.0`。

优先级从高到低：

1. 一元 `not`、负号 `-`
2. `*`、`/`、`%`
3. `+`、`-`
4. `<`、`<=`、`>`、`>=`、`==`、`!=`、`is`
5. `and`
6. `or`

括号可以显式改变表达式分组：

```lume
result:int = (2 + 3) * 4
ok:bool = result == 20 and not false
```

## 条件和块表达式

`if` 支持 `else`，也可以作为赋值右侧的块表达式使用：

```lume
fn max(a:int, b:int) -> int
    result:int = if a > b
        a
    else
        b
    end
ret result
```

块表达式的值是所选分支最后一个产生值的表达式；若分支没有值表达式，当前
实现以整数 `0` 作为结果。`ret` 始终是函数返回，不是只退出局部块。

## 循环

`while` 在条件为真时重复执行块：

```lume
fn sum_to(n:int) -> int
    sum:int = 0
    i:int = 1
    while i <= n
        sum := sum + i
        i := i + 1
    end
ret sum
```

解释器设有循环迭代保护（每次循环最多 100,000 次），用于避免意外的无限
循环长时间占用进程。

## 数组与内置函数

```lume
values:int = [10, 20, 30]
first:int = values[0]
count:int = len(values)
print(first)
```

内置函数 `print(value)` 输出一个值；`len(value)` 返回字符串或数组的长度。
数学函数和日志函数参见[数学与日志内置函数](math-and-logging.md)。

## 结构体、方法与闭包

结构体值使用命名字段字面量创建；已存在字段可读写：

```lume
fn Point.sum(self:Point) -> int
ret self.x + self.y

fn main() -> int
    point:Point = Point{x: 20, y: 22}
    point.x = 21
ret point.sum()
```

方法声明名使用 `类型名.方法名`，第一个参数为接收者。结构体值在运行时
保留类型名并检查缺失字段，但目前没有独立的 `struct` 声明和字段 schema，
也不支持嵌套字段赋值。

单表达式闭包使用 `fn(参数) => 表达式`，可选参数和返回类型标注：

```lume
fn main() -> int
    base:int = 37
    add_base:fn = fn(value:int) -> int => value + base
ret add_base(5)
```

闭包创建时按值捕获当前环境；目前不支持多语句闭包、递归闭包或可变捕获。

## 泛型与模式匹配

函数可以声明泛型参数；当前实现会擦除泛型参数后执行，尚不做完整约束推导：

```lume
fn identity<T>(value:T) -> T
ret value
```

`match` 表达式支持字面量、绑定名和 `_` 通配模式，按书写顺序匹配：

```lume
fn classify(value:int) -> int
    result:int = match value
        case 0
            12
        case other
            other + 10
    end
ret result
```

如果没有模式匹配且没有通配/绑定分支，运行时会返回明确的非穷尽匹配错误。

## 动态 C FFI

解释器支持显式类型签名的动态 C 共享库调用和 C 回调。加载、调用入口使用
`ffi_unsafe_*` 命名，因为库初始化和不匹配的 C ABI 签名可能执行任意本机代码、
破坏内存或终止进程。签名格式、GPU SDK 常用库名、回调保活要求及 GC pin 示例
见[动态 C FFI 文档](./ffi.md)。

```lume
fn main() -> int
    c_runtime:any = ffi_unsafe_load("libc.so.6")
    abs_fn:any = ffi_symbol(c_runtime, "abs")
    result:any = ffi_unsafe_call(abs_fn, "i32(i32)", [-42])
    print(result)
ret 0
```

签名支持整数、浮点数、指针和 `void` 返回类型，可用 ABI 前缀选择调用约定；变参签名
把实际变参类型写在 `...` 之后，按值 struct/union 分别写作 `{i32,f64}` 与
`<i32,f64>`。通过 FFI 可调用 CUDA、Level Zero、HIP、Vulkan loader 的宿主 API；
当前没有 GPU 加速，也不支持 bitfield、long double、向量 ABI 或嵌套聚合。
`ffi_buffer` 可创建有界本机缓冲区，以便读写 C 输出参数和按目标头文件布局访问
结构体指针。

函数可用 `#[ffi_export]` 标记为 C ABI 导出；manifest 的 `[ffi.export]` 决定
实际生成到头文件中的符号。解释器还提供 `ffi_export_ptr(name, signature)`，
把 Lume 函数包装成可交给 C 的回调指针。静态/AOT 路径的 manifest、
`[ffi.extern]` 声明与符号校验见 `docs/ffi.md`。

## 时间与 GC 字节缓冲区

解释执行路径提供两个时钟原语：`now_ms()` 返回 Unix epoch 毫秒，`mono_ms()`
返回单调时钟毫秒。超时、锁等待等需要稳定相对时间的逻辑应使用 `mono_ms()`。
Rust 宿主可通过 `lumec_core::clock::with_clock` 注入假时钟，测试不依赖 `sleep`。

定宽整数通过构造式转换 `u8(x)` / `u32(x)` 获得，越界会报错；类型检查器保留
`u8` / `u32` / `i8` / `i16` / `i32` 等宽度，窄整数可隐式加宽到 `int`，反向赋值会报
错。JIT 以零扩展的 `i64` 表示这些值。GC 管理的字节缓冲区用 `buf_new(n)` 创建，
长度写在对象 payload 前缀中，缓冲区在存活期间会被 pin，收集时不会搬迁：

```lume
fn main() -> int
    data:any = buf_new(8)
    buf_set_u8(data, 0, u8(255))
    buf_set_u32(data, 4, u32(16909060))
    print(buf_get_u8(data, 0))
    print(buf_get_u32(data, 4))

    copy:any = buf_new(8)
    memcpy(copy, 0, data, 0, 8)
ret 0
```

`buf_get_u32` / `buf_set_u32` 明确使用小端序。`buf_get_u8` / `buf_set_u8` 以及
`buf_new` / `buf_len` 的偏移和长度都有边界检查。`memcpy(dst, src, count)` 从两个
缓冲区起点复制；`memcpy(dst, dst_offset, src, src_offset, count)` 支持显式偏移，
重叠区域会被拒绝。`memmove` 接受相同的两种参数形式，并允许同一缓冲区内的重叠复制。
`str_to_buf` / `buf_to_str` 用 UTF-8 在字符串与缓冲区之间转换。

解释执行路径与 LLVM JIT 都支持 `buf_new`、`buf_len`、`buf_get/set_u8/u32`、
`memcpy` 和 `memmove`；JIT 把缓冲区句柄作为稳定地址传递，内置操作仍做边界检查。
可运行的 JIT 缓冲区样例见 `examples/demo_buffer_jit.lume`，覆盖定宽构造、`memcpy`
和 `memmove`。JIT 仍不支持字符串值的 `str_to_buf` / `buf_to_str`。`ffi_buffer_*`
创建的是独立本机内存，不与 GC 缓冲区共享生命周期。

## Socket（动态 FFI）

IPv4 的最小 echo server 已落地在 `examples/server_echo`：C ABI 垫片
`src/net.c` 封装 `listen` / `accept` / `recv` / `send` / `close`，Lume 侧通过
manifest `[ffi.extern]` 调用，fd 与长度统一用 `i64`。`sockaddr_in` 没进入 JIT，
接收缓冲用 `ffi_buffer` 提供；`mono_ms()` 负责单调时钟空闲超时详见
[docs/server.md](server.md)。当前为单线程顺序 accept 循环，多客户端并发
poll 集合尚未实现。

## LLVM JIT 编译路径

除解释执行的 `run` 外，`jit` 和 `llvm-ir` 命令可将受支持的整数函数、
条件分支、循环和逻辑表达式转换为项目 IR，再交给 LLVM：

```bash
cargo run -p lumec_core -- jit examples/demo_llvm_jit.lume
cargo run -p lumec_core -- llvm-ir examples/demo_llvm_jit.lume
```

此路径目前支持整数常量、局部变量、加减乘除、比较、短路 `and`/`or`/`not`、
命名函数调用、`if`/`while` 语句及 `if`/`while` 块表达式。浮点、数组、
结构体、闭包、泛型、模式匹配仍会明确报告暂不支持。当前解析器和 IR lowering
仍由 Rust 实现；这是可执行的 LLVM 编译子集，但不代表 Lume 编写的前端已经
替换 stage0 或完成自举。

manifest 项目还可在 JIT 中链接 C 模块产出的 bitcode，或通过 `[ffi.extern]
library = "..."` 注册 libffi trampoline 调用共享库符号。`#[ffi_export]` 函数
可生成 C 头文件；`ffi_export_ptr` 在解释器和 JIT 中都会生成真实的 C 回调指针，
JIT 在编译后把函数地址交给 libffi trampoline。JIT 回调限于当前线程同步调用，
且按值聚合仍不进入 JIT 的 `i64` 值 ABI。

## 当前限制

当前 `lume-bootstrap` 的解释器/解析器尚未实现完整语言规范，主要限制包括：

- `if`、`else if` 和 `while` 支持语句形式；`if` 和 `while` 也支持赋值右侧的代码块表达式。`while` 表达式返回最后一次迭代中代码块的值；如果循环一次也未执行，则返回 `0`。
- 数组支持字面量、整数下标读取，以及对已声明数组变量的下标赋值。下标必须是非负整数且位于数组范围内；暂不支持切片和嵌套下标赋值。
- 结构体目前是动态字段值，尚无独立声明/schema；闭包只支持单表达式和按值捕获；泛型仅支持语法擦除，未完整实现类型参数推导。
- 模式匹配支持字面量、绑定名和通配符，暂不支持解构、守卫、类型/范围模式或编译期穷尽性分析。
- 静态类型检查为基础子集；完整错误恢复、流敏感分析、数组元素类型和模块命名空间尚未实现。
- LLVM JIT 前端仅支持上述整数和控制流子集；浮点、聚合值、闭包、泛型和模式匹配尚未降低到 IR。`run` 仍由解释器执行。Lume 编写的 lexer/parser/lower/build 文件仍是骨架，完整 stage1/stage2 自举尚未完成。

发现与实现不符时，请以可运行代码和测试为准，并通过项目 issue/PR 反馈。
