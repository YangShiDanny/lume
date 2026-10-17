# Lume

一门静态类型、编译到 C 的语言。目标是「简单调用又最高效」。

你写直观的递归和循环，编译器负责把它们展开成查表、多线程和带 `restrict` /
向量化提示的 C 代码，剩下的交给 clang。

```lume
struct Point {
    x: float
    y: float

    fn len() -> float {
        return sqrt(x * x + y * y)
    }
}

@precompute(0, 30)
fn fib(n: int) -> int {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}

@cache
fn csteps(n: int) -> int {
    if n == 1 { return 0 }
    if n % 2 == 0 { return 1 + csteps(n / 2) }
    return 1 + csteps(3 * n + 1)
}

fn par_sum(a: [int], n: int) -> int {
    total: int = 0
    par for i in 0..n {
        total += a[i]        // 自动识别为 reduction，每线程私有累加后合并
    }
    return total
}

fn main() {
    p := Point(x: 3.0, y: 4.0)
    say("len = {p.len()}")           // 5.000000
    say("fib(30) = {fib(30)}")       // 832040，编译期已算完，运行时查表
    say("sum = {par_sum([1,2,3,4,5,6,7,8,9,10], 10)}")  // 55
}
```

`write(...)` 与 `say(...)` 使用相同的字符串插值格式，但不自动换行；固定数组字段可用
`[int; 256]` 声明，并通过 `[0; 256]` 零初始化。

## 构建与运行

```bash
cargo run -- run main.lume          # 编译并运行
cargo run -- build main.lume        # 只生成 main.c + 可执行文件
```

依赖 `clang` 在 PATH 里。默认 `-O3`，并默认加 `-fno-math-errno`
（让 `sqrt`/`pow` 成为真正的纯函数，编译器证明过的 `const` 标注才站得住）。

| 选项 | 作用 |
|---|---|
| `--native` | `-march=native`，按本机 CPU 生成向量指令 |
| `--lto` | `-flto` |
| `--fastmath` | `-ffast-math`（只影响 clang，编译器自身不做 fast-math 变换） |
| `-O2` | 把默认的 `-O3` 降回去 |

Windows 上目前只能由用户手动跑（见 `run.bat`，走 WSL）。

## 编译器自动做的优化

不用加任何开关：

- **数组参数加 `restrict`** —— Lume 的数组是值语义，两个数组参数不可能互相
  别名。说出来之后 clang 才敢把循环向量化。
- **纯函数打 `__attribute__((const))`** —— 由 `src/codegen/purity.rs` 做保守
  证明（只写局部变量、只引用参数和局部 `let`、只调数学白名单或已证明的函数）。
  证明不了的（读数组、`say`、字段访问）一律放弃。重复调用因此被消除。
- **每个 `for` 前发 `#pragma clang loop vectorize(enable) interleave(enable)`**。
- **`par for` 自动 reduction** —— 识别 `v += / -= /= *= rhs`，每线程私有累加器，
  结束后合并，语义等同 OpenMP 的 `reduction`。要求该变量在循环体里只出现在
  累加式中（出现 3 次就放弃，归约后语义会变）。
- **`par for` worker 提到文件顶层** —— 嵌套函数是 GCC 扩展，clang/MSVC 不认。
- **小循环自动串行** —— 元素数少于 `LUME_PAR_SERIAL_THRESHOLD`（1024）时直接
  调用 worker，不开线程。一次 `pthread_create` 几十 µs，小数据上开线程净亏。

## 测试

`main.lume` 是优化验证套件，20 项，每行末尾印期望值。跑：

```bash
cargo run -- run main.lume
```

WSL + clang -O3 实测全对。想加新优化，往 `main.lume` 里加一项、把期望值印在
同行最省事。

## 源码结构

```
src/
  lexer/        词法
  parser/       语法 → AST
  checker/      类型检查
  codegen/
    mod.rs      驱动、纯函数分析入口
    items.rs    函数/方法/单态化/缓存函数签名与定义
    expr.rs     表达式
    stmt.rs     语句、循环
    parallel.rs par for：reduction 识别 + worker 提升
    purity.rs   纯函数证明
    types.rs    类型推断
    runtime.rs 发出的 C runtime header
    say.rs      say / 字符串插值
  cli.rs        clang 调用、@precompute 子进程
```

## 已知限制

- 多维数组、切片（`a[1..3]`）、map、`match` 表达式、`spawn`、默认参数、
  `pub`/模块系统：AST 里已有定义，codegen 未实现（见 `src/ast.rs` 文件头的待办清单）。
- `par for` 不支持嵌套；循环体不能写外层变量（只有归约变量能跨线程累加）。
  两种情况都会生成 `#error`，不会静默出错。
- `say` 插值的类型推断依赖编译期已知的信息；调用普通函数时查返回类型表
  （`fn_ret_types`），查不到才退回 `int64_t`。
