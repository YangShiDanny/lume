# Lume 语言速查

给 AI 的 Lume 编程指南。读完这份就能写 Lume 代码。

> 版本：编译器 v0.1.0 · 本文档由 `src/` 源码逐模块反推生成（2026-10-16），与代码同步。
> 标 ⚠ 的是**会静默出错**的地方，写码时必须避开。
>
> 本次已修复并同步到文档：插值改为 AST 化、函数前向声明、`else { }` 块生效、
> `self.x` 可用、`|>` 可用、算术表达式类型推断、`@cache` 改 O(1) 哈希表、
> 锁与预计算临时文件跨平台、`par for` 闭区间切块、字符串函数缓冲区轮转。
>
> 本轮（性能优化）：数组参数 `restrict`、纯函数 `__attribute__((const))`、
> `par for` 自动 reduction、循环向量化提示、默认 `-O3` + `--native` + `--lto`、
> `par for` worker 提到文件顶层（clang/MSVC 可编译）。
>
> 同时补上两个「codegen 支持但检查器不认」的缺口：方法里可直接写裸字段名
> （`fn len() { return sqrt(x*x + y*y) }`，等价于 `self.x`），以及 `|>` 管道
> 在类型检查阶段按 `f(a, ...)` 脱糖（以前 `9 |> sq()` 会被误报参数个数不对）。

## 一句话介绍

Lume 是一门静态类型、编译到 C 的语言。写直观的递归与循环，编译器负责把 `@precompute` / `@cache` / `par for` 展开成查表与多线程 C 代码。

---

## 编译与运行

```bash
lume build main.lume [选项]   # 生成 main.c + 可执行文件
lume run   main.lume [选项]   # 编译并立即运行
```

选项：
- `--fastmath`：给 clang 加 `-ffast-math`（只影响 clang，编译器自身没做 fast-math 变换）
- `--native`：加 `-march=native`，按本机 CPU 生成向量指令（数值密集循环收益明显）
- `--lto`：加 `-flto`
- `-O2`：把默认的 `-O3` 降回 `-O2`

本项目里用 `cargo run -- run main.lume`（`run.bat` 走 WSL）。

`main.lume` 是本项目的**优化验证套件**，13 项，每行末尾都印了期望值。WSL 实测（clang -O3）全对：

| 项 | 覆盖点 | 实测 |
|---|---|---|
| [1] | 纯函数 `const` → `sq(7)` 调两次被折叠 | 49 / 98 |
| [2] | 前向声明 + 互相递归 | 27 / 1 / 0 |
| [3] | 插值走 AST，浮点算术用 `%f` | 10.000000 |
| [4] | `else { }` 块内容生效 | else 块生效 |
| [5] | 字符串函数缓冲区轮转（8 槽位） | HELLO WORLD |
| [6] | `\|>` 管道 | 81 |
| [7] | 泛型单态化 | 7 / 4.250000 |
| [8] | struct 方法、裸字段、`self.x` | 5 / (6,8) / 10 |
| [9] | 串行 vs `par for` vs 闭区间 reduction | 55 / 55 / 55 |
| [10] | `par for` 元素级 + restrict | 2 4 6 8 10 |
| [11] | float 数组参数 restrict + 向量化提示 | 2.5 / 4.5 / 6.5 |
| [12] | `@precompute` 编译期查表 | fib(30)=832040，**84 ns** |
| [13] | `@cache` 哈希表记忆化 | 128，首次 33 µs → 命中 138 ns |

想加新优化时，往 `main.lume` 里加一项、把期望值印在同行最省事。

规则：
- 源文件扩展名必须是 **`.lume`**（不是 `.lm`）。传别的后缀只 warning，不报错。
- 产物：`<文件名>.c` 和 `<文件名>`（Windows 下 `<文件名>.exe`），都落在当前目录。
- **依赖 `clang` 在 PATH 里**。默认 `-O3`，并默认加 `-fno-math-errno`（让 `sqrt`/`pow` 成为真正的纯函数）。
- Linux 下链接 `-lm -lpthread`；Windows 下不加。
- `@precompute` 额外需要支持 `-fopenmp` 的 clang。

编译器自身会做的优化（不用加开关）：
- 数组/列表参数自动加 `restrict` → clang 敢向量化
- 证明过的纯函数自动加 `__attribute__((const))` → 重复调用被消除
- 每个 `for` 前发 `#pragma clang loop vectorize(enable) interleave(enable)`
- `par for` 自动 reduction（见"并行"一节）

---

## 词法

```lume
// 单行注释
/* 块注释 */
#! 和 #[ 开头的行会被整行跳过
```

### 字面量

| 写法 | 类型 | 说明 |
|---|---|---|
| `123` `1_000_000` | int | 支持 `_` 分隔 |
| `0xFF` | int | 十六进制 |
| `0b1010` | int | 二进制 |
| `3.14` | float | 小数点后必须跟数字（`5.` 不合法） |
| `"hi\n"` | str | 转义 `\n \t \r \\ \" \{ \}` |
| `'A'` | rune | 单字符 |
| `#FF8800` | int | 必须是 6 位十六进制，生成 `0xFF8800` |
| `true` / `false` | bool | |
| `nothing` | — | 生成 C 的 `NULL` |

### 关键字

```
fn struct trait impl if else while for in return break continue match
import use pub const extern as and or not true false nothing
app par async await spawn it
```

保留但**未实现**的：`trait` `impl` `use` `extern` `async` `await` `spawn` `app`。
（词法/语法能解析，codegen 阶段被丢弃或报错。）

### 类型关键字

`int i8 i16 i32 i64 u1 u2 u4 u8 u16 u32 u64 f32 f64 float bool str rune`

---

## 类型

| Lume | 生成的 C | 备注 |
|---|---|---|
| `int` `i64` | `int64_t` | 默认整数 |
| `i8` `i16` `i32` | `int8_t` `int16_t` `int32_t` | |
| `u1` `u2` `u4` `u8` | **全部 `uint8_t`** | ⚠ u1/u2/u4 只是 u8 的别名，没有位宽语义 |
| `u16` `u32` `u64` | `uint16_t` `uint32_t` `uint64_t` | |
| `f32` | `float` | |
| `f64` `float` | `double` | `float` 是 `double` 的别名 |
| `bool` | `bool` | |
| `str` | `const char*` | |
| `rune` | `uint32_t` | |
| 结构体名 | 同名 | |
| `[T]` | `T*` | 列表 |
| `[T; N]` | `T*` | ⚠ N 被解析但 codegen 忽略 |

类型后缀 `T?`（Optional）、`T!`（Error）、`T[U]`（Generic）能解析，但 codegen 里落到 `void*` —— 未实现。

---

## 变量

```lume
x := 5                  // 声明，类型推断
name: str = "hello"     // 声明，显式类型（:= 也接受）
count = 10              // 赋值，变量必须已声明
count += 1              // 复合赋值：+= -= *= /= %=
```

规则：
- 换行结束语句，不需要分号。
- 用 `=` 给未声明的变量赋值 → 类型检查报「未定义的变量」。
- 重复 `:=` 同名变量不报错，就是覆盖定义。

类型推断规则：字面量按自身类型；算术表达式按操作数（有一边是浮点就得到浮点）；
比较/逻辑得到 `bool`；结构体字段访问按字段声明类型；数组元素按元素类型。

不支持只声明不初始化，必须带初始值。

---

## 运算符与优先级

```lume
+  -  *  /  %            // 算术
== != <  >  <= >=        // 比较
and or not               // 逻辑
=  += -= *= /= %=        // 赋值
as                       // 只在 import 别名里用
```

优先级从低到高：`or` < `and` < 比较 < `|>` < `+ -` < `* / %`。

规则：
- **没有 `&&` `||` `!`**，词法层根本不认识这些符号，必须写 `and` / `or` / `not`。
- `|>` 管道把左边作为右边的第一个参数：`a |> f()` → `f(a)`，`a |> f(b)` → `f(a, b)`。

---

## 字符串与 `say`

```lume
name := "world"
age := 18

say("hello {name}")        // hello world
say("{name} is {age}")     // world is 18
say("1 + 2 = {1 + 2}")     // 1 + 2 = 3（整数才行，见下）
say("literal {{brace}}")   // literal {brace}
```

规则：
- `{expr}` 插值，`{{` `}}` 转义；字符串里的 `%` 自动转义成 `%%`。
- `say(x)` 传变量也可以，格式串按类型推断。

### 插值支持什么

`{...}` 里的内容在**解析期**就交给真正的 parser 变成 AST，所以任意表达式都能用，
格式串按表达式的类型推断（不再是按源码文本猜）：

```lume
say("{a * b}")        // 浮点运算也正确 → %f
say("{p.x}")          // 结构体字段 → 按字段类型
say("{p.len()}")      // 方法调用 → 按返回类型
say("{str_len(s)}")   // 内置函数 → 按返回类型
say("{nums[0]}")      // 数组元素 → 按元素类型
```

类型 → 格式：整数 `%lld`、浮点 `%f`、字符串 `%s`、bool `%d`、rune `%c`、颜色 `0x%06X`。
实参会按格式串自动加显式转换，不会出现 varargs 类型不匹配。

插值只在 `say` 里有意义。把带插值的字符串赋给变量或传给函数时，会退化成纯文本（插值部分被丢掉）。

---

## 控制流

```lume
if x > 10 {
    say("big")
} else if x > 5 {
    say("mid")
} else {
    say("small")
}
```

```lume
while x < 100 {
    x += 1
}

for i in 0..10 { }        // 0 到 9（不含 10）
for i in 0..=10 { }       // 0 到 10（含）
for n in nums { }         // 遍历数组

for i in 0..100 {
    if i == 5 { continue }
    if i == 10 { break }
}
```

`else { }` 和 `else if` 都正常。

⚠ **if 作为表达式**（`x := if c { 1 } else { 2 }`）还没实现，会生成非法 C 导致 clang 报错。
需要条件取值就写成语句：

```lume
x := 0
if c { x = 1 } else { x = 2 }
```

---

## match

```lume
match value {
    1 => say("one"),
    2 => say("two"),
    _ => say("other")
}

match value {
    n => say("got {n}")     // 变量绑定，n 拿到 value
}
```

规则：
- 用 `=>`（不是 `->`）；每臂**一个表达式**，逗号分隔。
- `_` 通配。没有穷尽性检查，都不匹配就静默跳过。
- ⚠ **值被强制转为 `int64_t` 再比较**：只能 match 整数/布尔，match 字符串或结构体不会按预期工作。
- ⚠ 结构体模式 `P(a, b)` 和元组模式 `(a, b)` 的条件恒为真（第一版没实现解构）。

---

## 函数

```lume
fn add(a: int, b: int) -> int {
    return a + b
}

fn greet(name: str) {          // 无返回值省略 ->
    say("hello {name}")
}

fn main() { }                  // 自动生成 return 0
```

规则：
- 参数类型可省略（默认 `int64_t`）。
- 返回值类型不匹配会被类型检查拦下。
- 递归正常。

函数支持前向声明：调用后面定义的函数、互相递归都没问题。

### ⚠ 两个要注意的点

1. **默认参数不生效**：`fn connect(host: str, port: int = 8080)` 能解析，但生成的 C 函数不会补默认值，少传参数会 clang 报错。
2. **命名参数不支持**：`connect("localhost", port: 9090)` 直接 parse error。

---

## 结构体与方法

```lume
struct Point {
    x: float
    y: float

    fn len() -> float {
        return sqrt(x*x + y*y)      // 直接访问字段
    }

    fn move(dx: float, dy: float) {
        x += dx
        y += dy
    }
}

p := Point(x: 3.0, y: 4.0)          // 构造用圆括号
l := p.len()
p.move(1.0, 1.0)
say("{p.x}")                        // 字段访问
```

规则：
- 方法里直接写字段名；参数名和字段冲突时可以写 `self.x`（生成 `_self->x`）。
- 方法里也可以正常用 `say`。
- 生成的 C：`typedef struct Point {...} Point;` + `static ... Point_len(Point* _self, ...)`。
- ⚠ 方法调用的接收者必须是**单个变量名**（`p.len()` 可以，`a.b.len()` 不行）。
- 没有继承、没有泛型 struct。

---

## 数组

```lume
nums := [1, 2, 3, 4, 5]

first := nums[0]
nums[0] = 10

for n in nums {
    say("{n}")
}

say("{nums.len()}")     // 字面量数组可以，输出 5
```

规则：
- 元素类型由**第一个元素**推断，元素类型不一致会被类型检查报错。
- 生成 C 栈数组（`int64_t nums[] = {...}`），**固定长度**。
- ⚠ 数组不能跨函数传：`fn f(a: [int])` 退化成 `int64_t*`，长度信息丢失（`a.len()` 会引用不存在的 `a_len`）。
- 没有动态数组、没有 `push`、切片 `nums[1..3]` 能解析但 codegen 未实现。

---

## 泛型

```lume
fn max[T](a: T, b: T) -> T {
    if a > b {
        return a
    }
    return b
}

x := max(3, 5)        // 生成 max_int64_t(3, 5)
y := max(1.5, 2.5)    // 生成 max_double(1.5, 2.5)
```

规则：
- `fn name[T](...)` 声明；调用时**不写** `[T]`，类型从实参推断。
- 单态化：每个类型组合生成一份 C 代码，函数名后缀是 C 类型名。
- 支持多个参数：`fn pair[A, B](a: A, b: B) -> A`。
- ⚠ **泛型函数体跳过类型检查**（类型未具体化，检查无意义）。
- ⚠ 泛型函数里再调泛型函数**不会被单态化收集**，会生成未定义函数。
- 不支持泛型 struct。

---

## 装饰器

### @cache

```lume
@cache
fn fib(n: int) -> int {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}
```

- 只支持**单参数**、有返回值。无返回值时会退化成普通函数。
- 生成 `_fib_impl` + 一张运行时填充的**开放寻址哈希表**（查找 O(1)，负载 0.75 时翻倍扩容）。
- 单线程版本不带锁；`@precompute` 走的是跨平台锁版本（`LUME_MUTEX_*`，Windows/Linux 都能编）。

### @precompute(start, end)

```lume
@precompute(0, 40)
fn fib(n: int) -> int { ... }
```

- 只支持**单参数**、**整数返回**。
- 编译器在编译期真跑一遍 `fib(0)`…`fib(40)`，把结果写成 C 常量表；运行时命中范围直接查表。
- 超出 `[start, end]` 自动回退到 `@cache` 路径。
- 结果缓存到 `.lume_cache/precompute_<hash>.bin`，下次编译直接复用（终端会打印「缓存」还是「新算」）。
- 缓存键是**预计算程序整份 C 源码的 hash**：函数名、参数、函数体、返回类型、范围全都参与，改了代码一定不会命中旧表。
- ⚠ Linux 下用 `-fopenmp` 并行算表（Windows 自动退回串行），依赖 clang 支持。
- ⚠ 表值按 `%lld` 打印解析 —— **非整数返回值会得到垃圾表**。
- ⚠ 被预计算的函数如果调用了别的用户函数，生成的临时程序里没有那个函数，会预计算失败并回退。

`@precompute` 是 Lume 的核心卖点：写自然递归，编译器把它变成查表。

---

## 并行

```lume
par for i in 0..1000000 {
    out[i] = a[i] * 2.0
}

total: float = 0.0
par for i in 0..1000000 {
    total += a[i]        // 自动 reduction，和串行结果一致
}
```

规则：
- `par` 前缀只能修饰 `for`，且迭代器**只支持范围**。
- 自动按 `cpu_count` 切块、最多 64 个线程，引用的外部数组打包进 worker 结构体，
  并在 worker 里拷成带 `restrict` 的局部指针（循环体可以向量化）。
- worker 函数生成在文件顶层，不嵌在函数里 —— clang / gcc / MSVC 都能编过。
- 循环体支持：赋值、`let`、`if`（可嵌套）；其他语句生成 `/* par body: 暂不支持 */`。
- `0..N` 与 `0..=N` 切块都正确，区间按 `[start, stop)` 均分给各线程，不会重复也不会漏。
- ⚠ 循环体里不要 `say`（多线程 printf 交错）。

**自动 reduction**：识别 `v += rhs` / `v -= rhs` / `v *= rhs`，每个线程用私有累加器，
结束后合并回 `v`，等价于 OpenMP 的 `reduction(+:v)`。触发条件很严格：

- `v` 不是循环变量，循环体里没有被 `let` 重新声明
- `v` 在整个循环体里**只出现在这些累加式里**（出现次数必须正好是 `2 × 累加语句数`）。
  循环体里再多读一次 `v`，编译器就放弃归约

⚠ 不满足条件时**不归约也不加锁**，多线程写同一个变量就是数据竞争，结果随线程数变。
要并行求和就老老实实写 `total += a[i]`。

---

## 内置函数

直接可用，**不需要 import**：

| 函数 | 返回 | 说明 |
|---|---|---|
| `say(...)` | void | 打印，支持插值 |
| `now()` | int | 当前纳秒时间戳（单调时钟） |
| `str_len(s)` `str_upper(s)` `str_lower(s)` `str_trim(s)` | int / str | 字符串处理 |
| `str_to_int(s)` `str_to_float(s)` | int / float | 解析 |
| `int_abs(x)` `int_min(a,b)` `int_max(a,b)` | int | |
| `float_sqrt(x)` / `sqrt(x)` | float | 两种名字都行 |
| `float_abs` `float_pow`/`pow` | float | |
| `float_sin`/`sin` `float_cos`/`cos` `float_tan`/`tan` | float | |
| `float_floor`/`floor` `float_ceil`/`ceil` `float_round`/`round` | float | |

其他 C 标准函数**没有 extern 通路**，用不了。

`str_upper` / `str_lower` / `str_trim` 返回内部缓冲区，内部有 8 个轮转槽位，
所以 `say("{str_upper(a)} {str_upper(b)}")` 不会互相覆盖。
单个表达式里超过 8 个结果时才会开始复用（缓冲区上限 4095 字符）。

---

## 类型检查能报什么

编译前会跑一遍 checker，报错带 `help` 提示：

- 未定义的变量 / 未定义的函数（含拼写检查提示）
- 函数参数个数不匹配、参数类型不匹配
- 返回类型与声明不符
- `if` / `while` 条件不是 bool
- `and` `or` 两边不是 bool、`not` 作用的不是 bool
- 比较 `==` `<` 等两边类型不同
- 算术运算类型不匹配
- 数组索引不是 int
- 数组元素类型不一致
- `let` 声明类型与初始值类型不符

**不检查**：泛型函数体、`match` 穷尽性、数组越界。

---

## 已知限制速查

| 别写 | 原因 |
|---|---|
| `x := if c { 1 } else { 2 }` | if 表达式未实现，生成非法 C |
| `fn f(a: int = 1)` | 默认值不生效 |
| `f(port: 9090)` | 命名参数不支持 |
| `trait` `impl` `async` `await` `spawn` `app` `use` `extern` | 能解析，未实现 |
| `import` | 完全无作用，内置函数本来就全局可用 |
| 数组传参、动态数组、闭包捕获、异常 `?` `!` | 未实现 |
| 非整数的 `match` | 值被转成 `int64_t` 再比较 |
| 泛型函数里调泛型函数 | 不会单态化，生成未定义函数 |

---

## 完整示例

```lume
@precompute(0, 40)
fn fib(n: int) -> int {
    if n < 2 { return n }
    return fib(n - 1) + fib(n - 2)
}

struct Point {
    x: float
    y: float

    fn len() -> float {
        return sqrt(x*x + y*y)
    }
}

fn main() {
    t1 := now()
    x := fib(40)
    t2 := now()
    say("fib(40) = {x}, 耗时 {t2 - t1} ns")

    p := Point(x: 3.0, y: 4.0)
    say("len = {p.len()}")
}
```

## 性能建议

1. 整数递归加 `@precompute`（编译期算完）或 `@cache`（运行时记忆化，O(1) 查表）。
2. 数据并行用 `par for`，元素级操作多核加速明显；求和直接写 `sum += a[i]`，编译器会自动 reduction。
3. 极致浮点性能加 `--fastmath`；本机跑的话加 `--native` 用上 AVX 等向量指令；
   想要极限再加 `--lto`。默认已经是 `-O3`。
4. 生成的函数都带 `static`，小函数内联交给 clang。
5. 数组参数自动带 `restrict`，不用自己操心别名 —— 这是 Lume 能对齐 Fortran 向量化的关键。
6. 写完可以看一眼生成的 `<文件名>.c`：确认你的热点函数有没有拿到
   `LUME_PURE`、循环前面有没有 `#pragma clang loop vectorize`。
