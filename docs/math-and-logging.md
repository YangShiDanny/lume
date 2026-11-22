# 数学与日志内置函数

本文说明 Lume 当前解释器提供的标量数学函数、SLEEF 兼容函数名和日志接口。
可直接运行示例：

```bash
cargo run -p lumec_core -- run examples/demo_math_logging.lume
```

浮点数字面量应包含小数点，例如 `0.5`、`3.0`。

## 数学函数

### 一元函数

当前提供：

`sin`、`cos`、`tan`、`asin`、`acos`、`atan`、`sinh`、`cosh`、`tanh`、
`asinh`、`acosh`、`atanh`、`exp`、`exp2`、`expm1`、`ln`、`log`、`log2`、
`log10`、`log1p`、`sqrt`、`cbrt`、`floor`、`ceil`、`round`、`trunc`、
`abs`。

### 二元函数

`pow(x, y)`、`atan2(y, x)`、`hypot(x, y)`、`fmod(x, y)`、
`remainder(x, y)`、`copysign(x, y)`。

示例：

```lume
fn main() -> int
    angle:float = 0.5
    sine:float = sin(angle)
    radius:float = sqrt(9.0)
    power:float = pow(radius, 2.0)
    print(sine + power)
ret 0
```

所有函数在当前解释器中通过宿主 Rust `f64` 数学方法计算。`llvm_sin`、
`llvm_cos`、`llvm_exp`、`llvm_log` 和 `llvm_sqrt` 是这些宿主数学操作的
兼容入口；它们**不表示已经将 LLVM libc 独立编译并静态内置**。本项目的
LLVM 依赖用于 IR/JIT 内核。

## SLEEF 兼容名称和原生链接

以下名称在语言中可调用：

`sleef_sin_u10`、`sleef_sin_u35`、`sleef_cos_u10`、`sleef_cos_u35`、
`sleef_tan_u10`、`sleef_exp_u10`、`sleef_log_u10`、`sleef_sqrt_u05`、
`sleef_pow_u10`、`sleef_pow_u35`。

默认构建中，这些名称回退到宿主 `f64` 数学函数，**不保证符合 SLEEF 的
误差界**。如果程序需要 SLEEF 原生 ABI 和相应精度保证，请先构建并安装
SLEEF，再启用 `system-sleef` 特性：

```bash
LD_LIBRARY_PATH=/path/to/sleef/lib \
SLEEF_LIB_DIR=/path/to/sleef/lib \
  cargo run -p lumec_core --features system-sleef -- \
  run examples/demo_math_logging.lume
```

`SLEEF_LIB_DIR` 指向 `libsleef` 所在目录；Linux 运行时还要确保动态链接器
能找到该库，例如设置 `LD_LIBRARY_PATH`。目前原生链接会使用上游导出的
`Sleef_*` 标量符号；SLEEF 当前没有单独的 `Sleef_pow_u35` 符号，因此
`sleef_pow_u35` 也映射到 `Sleef_pow_u10`。

`system-sleef` 是可选依赖，不会在一般构建或 CI 中自动下载、编译 SLEEF。

## 日志函数

提供 `trace(message)`、`debug(message)`、`info(message)`、`warn(message)`、
`error(message)`，以及 `log_at(level, message)`。日志输出到标准错误流，
并附加级别名称：

```lume
fn main() -> int
    log_info("程序已启动")
    warn("将使用默认配置")
ret 0
```

`log_at` 的级别参数是字符串，可选 `trace`、`debug`、`info`、`warn`、
`error`。

`lib/std/log.lume` 提供方便的 Lume 包装函数。当前运行时实现的是轻量级
级别日志接口，**没有将 ferrilog/zerolog-rs crate 链接进来**。上游
`ferrilog` 仓库目前已转为 zerolog-rs，并要求 nightly 及额外 Git 依赖；
如需其完整异步队列、文件轮换或结构化字段能力，需要后续单独集成。

## 完整示例

[`examples/demo_math_logging.lume`](../examples/demo_math_logging.lume) 展示了
浮点运算、SLEEF 兼容函数名、日志和 `print`。默认运行无需额外第三方库：

```bash
cargo run -p lumec_core -- run examples/demo_math_logging.lume
```
