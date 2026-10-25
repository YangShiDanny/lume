# Lume 语言速查

给 AI 的 Lume 编程指南。读完这份就能写 Lume 代码。

> 版本：编译器 v0.2.0 · 本文档由 `src/` 源码逐模块反推生成（2026-10-16），与代码同步。  
> 标 ⚠ 的是**会静默出错**的地方，写码时必须避开。
>
> 本次已修复并同步到文档：插值改为 AST 化、函数前向声明、`else { }` 块生效、  
> `self.x` 可用、`|>` 可用、算术表达式类型推断、`@cache` 改 O(1) 哈希表、  
> 锁与预计算临时文件跨平台、`par for` 闭区间切块、字符串函数缓冲区轮转。
>
> 本轮（性能优化）：纯函数 `__attribute__((const))`、  
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
lume run   main.lume [选项] [-- 程序参数...]   # 编译并立即运行
lume libs  main.lume          # 列出库包、依赖关系、本次会加载哪些
```

选项：

- `--fastmath`：给 clang 加 `-ffast-math`（只影响 clang，编译器自身没做 fast-math 变换）
- `--native`：加 `-march=native`，按本机 CPU 生成向量指令（数值密集循环收益明显）
- `--lto`：加 `-flto`
- `-O2`：把默认的 `-O3` 降回 `-O21`

`run` 时写在 `.lume` 之后的参数会原样传给程序（`arg()` / `arg_count()` 取），  
`--` 之后则无条件透传：

```bash
lume run tool.lume in.txt out.txt     # arg(1)=in.txt  arg(2)=out.txt
lume run tool.lume -- --native        # 程序自己收到 --native
```

环境变量：

- `LUME_PATH`：额外的库目录（`:` 分隔，Windows 上 `;`）。见「写库：包的形态」。
- 默认库目录 `~/.lume/lib` 存在时会**自动**加进搜索路径。
- `LUME_LIB_TRACE=1`：编译时把加载了哪些库包打到 stderr（不影响编译）。
- `LUME_OPT=safe`：关掉激进优化。

本项目里用 `cargo run -- run main.lume`（`run.bat` 走 WSL）。

`main.lume` 是本项目的**优化验证套件**，33 项，每行末尾都印了期望值。WSL 实测（clang -O3）全对：

| 项    | 覆盖点                                  | 实测                       |
| ---- | ------------------------------------ | ------------------------ |
| [1]  | 纯函数 `const` → `sq(7)` 调两次被折叠         | 49 / 98                  |
| [2]  | 前向声明 + 互相递归                          | 27 / 1 / 0               |
| [3]  | 插值走 AST，浮点算术用 `%f`                   | 10.000000                |
| [4]  | `else { }` 块内容生效                     | else 块生效                 |
| [5]  | 字符串函数缓冲区轮转（8 槽位）                     | HELLO WORLD              |
| [6]  | `\|>` 管道                             | 81                       |
| [7]  | 泛型单态化                                | 7 / 4.250000             |
| [8]  | struct 方法、裸字段、`self.x`               | 5 / (6,8) / 10           |
| [8b] | `self` 按值传递（`f(self)` → `f(*_self)`） | 10.000000                |
| [9]  | 串行 vs `par for` vs 闭区间 reduction     | 55 / 55 / 55             |
| [9b] | `-=` / `*=` 归约（符号不能反）                | -55 / 3628800            |
| [9c] | 归约识别失败 → 退化成串行，结果仍对                  | 28                       |
| [9d] | **40000 个元素，超过串行阈值，强制走线程池+合并**       | 40000                    |
| [9e] | 循环体读只读外层标量（按值传进 worker）              | 3 / 6 / 9 / 12           |
| [10] | `par for` 元素级写回                      | 2 4 6 8 10               |
| [11] | float 数组参数 + 向量化提示                   | 2.5 / 4.5 / 6.5          |
| [12] | `@precompute` 编译期查表                  | fib(30)=832040，**84 ns** |
| [13] | `@cache` 哈希表记忆化                      | 128，首次 33 µs → 命中 138 ns |

想加新优化时，往 `main.lume` 里加一项、把期望值印在同行最省事。  
注意 `[9d]` 故意用 40000 个元素来超过 `LUME_PAR_SERIAL_THRESHOLD`（32768），  
这样线程池 + 合并那条路径才有人覆盖；改阈值时记得同步改这个数。

规则：

- 源文件扩展名必须是 **`.lume`**（不是 `.lm`）。传别的后缀只 warning，不报错。
- 产物：`<文件名>.c` 和 `<文件名>`（Windows 下 `<文件名>.exe`），都落在当前目录。
- **依赖 `clang` 在 PATH 里**。默认 `-O3`，并默认加 `-fno-math-errno`（让 `sqrt`/`pow` 成为真正的纯函数）。
- Linux 下链接 `-lm -lpthread`；Windows 下不加。
- `@precompute` 需要 clang；有 libomp 时并行填表，没有就自动串行填表。

编译器自身会做的优化（不用加开关）：

- 证明过的纯函数自动加 `__attribute__((const))` → 重复调用被消除
- 对可向量化的循环发 `#pragma clang loop vectorize(enable) interleave(enable)`；  
  含 `say` / 未证明纯度调用的循环不发
- `par for` 自动 reduction（见"并行"一节）
- `par for` 用常驻线程池：元素数 < `LUME_PAR_SERIAL_THRESHOLD`（32768）时  
  主线程串行跑完；否则按元素数只唤醒前几个 worker，`LUME_PAR_THREADS` 可覆盖线程数
- 数组参数**不加** `restrict`：数组是引用语义，可能别名，加了会算错
- 整数 `/` `%` 的除零检查只留给「除数在编译期看不出来」的情况；  
  常量除数（含 `const` 名）d ≠ 0 时检查是死代码，直接出 `(a / d)`。  
  这一步的实际收益是**让 clang 敢向量化**——带 `abort()` 的循环它不碰 SIMD
- 被除数能证明非负时，`x / 2^n` → `x >> n`、`x % 2^n` → `x & (2^n-1)`。  
  有符号负数不做这个替换（`-9/8 == -1` 但 `-9>>3 == -2`），clang 同样不敢，  
  Lume 靠 `for i in 0..n` 的循环变量推断出非负才敢
- `par for` 循环体里的只读外层标量按值传进 worker；写外层变量直接 `#error`

`par for` 的硬限制（违反会生成 `#error`，不会静默出错）：

- 不支持嵌套
- 循环体不能写外层变量（只有归约变量能跨线程累加）
- 只支持区间迭代器 `for i in 0..n`
- `par for` 自动 reduction（见"并行"一节）

---

## 词法

```lume
// 单行注释
/* 块注释 */
#! 和 #[ 开头的行会被整行跳过
```

### 字面量

| 写法                | 类型    | 说明                        |
| ----------------- | ----- | ------------------------- |
| `123` `1_000_000` | int   | 支持 `_` 分隔                 |
| `0xFF`            | int   | 十六进制                      |
| `0b1010`          | int   | 二进制                       |
| `3.14`            | float | 小数点后必须跟数字（`5.` 不合法）       |
| `"hi\n"`          | str   | 转义 `\n \t \r \\ \" \{ \}` |
| `'A'`             | rune  | 单字符                       |
| `#FF8800`         | int   | 必须是 6 位十六进制，生成 `0xFF8800` |
| `true` / `false`  | bool  |                           |
| `nothing`         | —     | 生成 C 的 `NULL`             |
| `{"a": 1}`        | map   | 键值对字面量，K/V 从第一对推         |

### 关键字

```
fn struct enum trait impl if else while for in return break continue match scope
import use pub const extern as and or not true false nothing
app par async await spawn it
```

保留但**未实现**的：`trait` `impl` `use` `async` `await` `spawn` `app`。  
（词法/语法能解析，codegen 阶段被丢弃或报错。）

`extern` 只支持 `extern fn`（见「extern：直接调 C」），`extern` 全局变量不支持。

### 类型关键字

`int i8 i16 i32 i64 u1 u2 u4 u8 u16 u32 u64 f32 f64 float bool str rune`

### 不能拿来当变量名的类型名

`Vec` 和 `Map`。它们不是关键字（词法上就是普通标识符），但类型位置和
表达式位置都**优先按容器类型**解析 —— `Vec[int]` 里的 `int` 会被当类型
而不是表达式。拿它们当变量名会得到莫名其妙的报错。

---

## 类型

| Lume                | 生成的 C                            | 备注                          |
| ------------------- | -------------------------------- | --------------------------- |
| `int` `i64`         | `int64_t`                        | 默认整数                        |
| `i8` `i16` `i32`    | `int8_t` `int16_t` `int32_t`     |                             |
| `u1` `u2` `u4` `u8` | **全部 `uint8_t`**                 | ⚠ u1/u2/u4 只是 u8 的别名，没有位宽语义 |
| `u16` `u32` `u64`   | `uint16_t` `uint32_t` `uint64_t` |                             |
| `f32`               | `float`                          |                             |
| `f64` `float`       | `double`                         | `float` 是 `double` 的别名      |
| `bool`              | `bool`                           |                             |
| `str`               | `const char*`                    |                             |
| `rune`              | `uint32_t`                       |                             |
| 结构体名                | 同名                               |                             |
| 枚举名                 | `int64_t` + `#define`             | 见「枚举」                        |
| `Vec[T]`            | `LumeVec*`                       | 元素类型不进 C，见「动态数组」            |
| `Map[K, V]`         | `LumeMap*`                       | K/V 不进 C，见「哈希表」              |
| `[T]`               | `T*`                             | 列表                          |
| `[T; N]`            | `T*`                             | ⚠ N 被解析但 codegen 忽略         |
| `T?` / `T!`         | `lume_opt_N` / `LumeOptStr`      | `{ int8_t has; T val; }`，两种后缀合并  |
| `T[U]`（其它）          | 未实现                              | 只有 `Vec[T]` / `Map[K, V]` 认       |

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

### 顶层常量

```lume
const M = 8             // 类型按值推断
const LIMIT: int = 100  // 也可以显式写类型
```

- 只能写在文件顶层（函数内用普通的 `:=`）。
- 生成 `const T NAME = ...;`，放在所有函数定义之前。
- 值是**字面量**（`8`、`-4`）时，编译器能把它当常量参与优化：  
  `x % M` 按 `x % 8` 处理，`x / M` 直接出 `(x / 8)` 而不带除零检查。  
  值是表达式（`const N = 1 + 2`）时照样能编译，只是拿不到这份优惠，  
  退回 C 编译器的常量传播。

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
- 算术和比较在**结构体**上可以重载（`a + b` → `op_add`），见
  [运算符重载](#运算符重载)。`and` / `or` / `|>` 不支持。

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

### 拼接

`str + str` 拼接，结果是一块**新分配**的内存：

```lume
a := "hello"
b := " world"
say(a + b)               // hello world
say(a + b + "!")         // 链式，左结合
say("len={str_len(a + b)}")

acc := ""
acc += "x"               // += 也走拼接
acc += "y"               // x 和 y 都在，不会互相覆盖
```

只允许 `str + str`。`"n = " + 1` 会**编译报错**，不做隐式转换 ——  
拼没拼上必须一眼看得出，要混类型就写插值 `"n = {n}"`。

拼接、`str_upper`、`str_slice` 这些派生结果都放在**字符串 arena** 里，  
和 C 的 `char*` 完全兼容（可以直接交给 `printf`），细节见  
[内置函数 → 字符串内存](#字符串内存arena)。

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

插值不只在 `say` 里有意义：

- **在 `say` 里**，插值直接摊成 printf 格式串，零分配。
- **在别的位置**（赋值、传参、参与拼接），插值会拼成一个新字符串，落在 arena 里：

```lume
n := 42
s := "n = {n}"              // "n = 42"
t := "{n} + {n} = {n * 2}"  // "42 + 42 = 84"
say("len={str_len(t)}")     // 13
```

（早期版本会把插值部分丢掉，`s := "a{x}"` 只得到 `"a"`，而且不报错。）

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

### `scope` 块：把块内的临时字符串还回去

字符串都从 arena 切（见「字符串内存：arena」），arena 默认只增不还。  
在循环里反复派生字符串就会一路涨。`scope` 给出一个局部回收点：

```lume
while i < n {
    scope {
        line := str_trim(str_upper(lines[i]))   // 这里有 2 次 arena 分配
        total = total + str_len(line)
    }                                            // 出块：这两块内存一起还回去
    i = i + 1
}
```

- **变量作用域**和普通块一样：块内声明的名字在外面看不到。
- **出块时**把块内派生的字符串一次性还给 arena，块**之前**派生的不受影响。
- 可以嵌套：内层只回收内层那一段。
- `break` / `continue` 会跳过块尾，编译器会在跳转前自动补一次回收。
- ⚠ `return` **不会**回收（故意泄漏一次）。返回值本身可能就是块内派生的  
  字符串，回收等于返回悬垂指针 —— 泄漏是安全的，悬垂不是。
- ⚠ `par for` 的循环体里**不能用** `scope`，编译期直接报错：多个 worker  
  共享同一个 arena，一个 worker 回退水位会把别人刚分配的一起回收。

拿不准内存有没有涨就量一下：`str_arena_bytes()` 返回当前 arena 已用字节数。

```lume
before := str_arena_bytes()
// ... 跑一堆派生 ...
say("这块代码用了 {str_arena_bytes() - before} 字节")
```

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
- `_` 通配。**被 match 的值是枚举时**有穷尽性检查：漏了变体编译期报错
  （写 `_` 兜底就不查）。其他类型不检查，都不匹配就静默跳过。
- ⚠ **值被强制转为 `int64_t` 再比较**：只能 match 整数/布尔/枚举，match 字符串或结构体不会按预期工作。
- ⚠ 结构体模式 `P(a, b)` 和元组模式 `(a, b)` 的条件恒为真（第一版没实现解构）。

枚举变体要写全名 `Tag.Number`（裸写 `Number` 是变量绑定，不是模式）：

```lume
match tok.kind {
    Tag.Number => say("数字"),
    Tag.Ident  => say("标识符"),
    _          => say("其它")
}
```

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
- **局部变量遮蔽字段**：方法体里 `w := 5` 之后再用 `w` 指的是那个局部变量，  
  不是字段。所以 `w := w + 1` 这种写法里的右侧 `w` 仍读字段（声明生效前的  
  那次引用），符合直觉。
- 方法里也可以正常用 `say`。
- 生成的 C：`typedef struct Point {...} Point;` + `static ... Point_len(Point* _self, ...)`。
- 接收者可以是变量、`self`、成员链、下标，也可以是函数返回值
  （`big(-5).sign()`）—— 没有地址的那种会先落一个临时变量。
- 没有继承、没有泛型 struct。

---

## 运算符重载

在结构体里定义 `op_*` 方法，`a + b` 就落到它上面。**没有额外语法糖**：
方法名是保留的，编译器看到左操作数是结构体就查这张表。

```lume
struct Money {
    cents: int

    fn op_add(o: Money) -> Money {
        return Money(cents: cents + o.cents)
    }

    fn op_str() -> str {
        return int_to_str(cents / 100) + "." + int_to_str(int_abs(cents % 100))
    }
}

a := Money(cents: 199)
b := Money(cents: 1)
say("{a + b}")          // 200 —— 先 op_add 再 op_str
```

| 写法 | 方法 | 签名 |
|---|---|---|
| `a + b` | `op_add` | `fn op_add(o: T) -> T` |
| `a - b` | `op_sub` | 同上 |
| `a * b` | `op_mul` | 同上 |
| `a / b` | `op_div` | 同上 |
| `a % b` | `op_mod` | 同上 |
| `-a` | `op_neg` | `fn op_neg() -> T`（**无参数**） |
| `a == b` | `op_eq` | `fn op_eq(o: T) -> bool` |
| `a != b` | `op_ne` | 同上 |
| `a < b` | `op_lt` | 同上 |
| `a > b` | `op_gt` | 同上 |
| `a <= b` | `op_le` | 同上 |
| `a >= b` | `op_ge` | 同上 |
| `"{a}"` / `say(a)` | `op_str` | `fn op_str() -> str`（**无参数**） |

规则：

- **`self` 是隐式的**，不算参数。二元方法的唯一参数就是右操作数。
- **比较只写 `op_eq` + `op_lt` 就够**：缺的自动组合 ——
  `a != b` → `!(a == b)`，`a > b` → `b < a`，`a <= b` → `!(b < a)`，
  `a >= b` → `!(a < b)`。写了精确名字就优先用它。
- **两边必须同类型**，不做隐式转换。
- `op_str` 是「可打印」的开关。没定义时 `say(x)` / `"{x}"` **编译期报错** ——
  结构体直接进 `printf` 的 varargs 读到的是第一个字段（UB），不能静默放过去。
- **`and` / `or` / `|>` 不能重载**。
- 左操作数不必是变量：`(a + b) + c`、`-a + b`、`f(x) * y` 都行，
  生成时先落临时变量再取地址。
- 方法体里可以写 `self + o`、`self.other_method()`。
- `a += b` 是 `a = a + b` 的语法糖，同样走 `op_add`。

C 侧生成 `S_op_add(&a, b)`（`self` 是指针）。运算符方法就是普通方法，
直接写 `a.op_add(b)` 效果一样。

完整可运行的例子：`examples/bigint/`（带符号任意精度整数，加减乘除、
比较、幂、`to_str` 全部通过运算符）。

---

## 枚举

C 风格、**无载荷**的枚举：每个变体就是一个整数常量。

```lume
enum Tag {
    Number          // 0
    Ident           // 1
    Plus            // 2
    Eof = 100       // 显式值
    Done            // 101（显式值之后继续 +1）
}
```

变体之间用换行或逗号分隔都行。用的时候带枚举名：

```lume
t := Tag.Ident
if t == Tag.Ident { say("标识符") }
say("{Tag.name(t)}")      // Ident —— 报错信息/调试用
say("{t}")                // 1    —— 当成整数打印

tok := Tok(kind: Tag.Number, line: 3)   // 可以当结构体字段/参数/返回类型
kinds: Vec[Tag] = [Tag.Number, Tag.Eof] // 也可以装进 Vec
```

- 类型标注写 `fn f(t: Tag)` / `kind: Tag`；
- `Tag.name(v)` 返回变体名字符串（越界值返回 `"?"`），**枚举唯一的方法**；
- `match` 上枚举会做穷尽性检查（见 [match](#match)）；
- 变体值必须唯一，重复了编译期报错；
- 枚举和整数是不同类型：`if t == 1` 编译期报错，得写 `Tag.X`。

C 侧生成 `typedef int64_t Tag;` 加每个变体一个 `#define Tag_Number 0LL`，
不用 C 的 `enum`（C enum 底层是 32 位 `int`，进 `printf` 的可变参数是 UB）。

> 带数据的枚举（`Num(int)` 这种 tagged union）**未实现**：语言还没有
> 载荷绑定和表达式形式的 `match`。等这两样落地再做。

---

## 多文件项目


**文件名即模块名，零 `import`。** 编译器会把你构建的那个文件所在目录
（含子目录）里的所有 `.lume` 自动当成同一个项目合并。

```lume
// main.lume —— 入口，必须有 fn main
fn main() {
    say("{triple(7)}")   // triple 在 mathx.lume 里，直接用
}
```

```lume
// mathx.lume —— 模块文件，没有 fn main
const SCALE = 3

fn triple(x: int) -> int {
    return x * SCALE
}
```

构建 `lume build main.lume`，`mathx.lume` 会被自动并入。跨文件可以随便互相调用，
前向声明是全局的，不存在「先定义后使用」的限制。

### 边界规则（重要）

自动扫描最怕把别人写的东西捡进来，所以有三条硬规则：

1. **带 `fn main` 的文件是可执行入口，不参与别人的合并。**
   所以 `main.lume` 和 `tests_pipe.lume` 放同一目录也互不干扰。
2. **子目录里只要有自己的入口文件，整个子目录算独立项目，跳过。**
   所以仓库根目录构建 `main.lume` 时，`tests/multi/` 不会被吞进去；
   反过来在 `tests/multi/` 里构建，也不会看到仓库根的文件。
3. **隐藏目录（`.git`、`.workbuddy`）、`target`、`node_modules` 不扫。**

另外：扫描到的非入口文件如果**解析失败**，只打 warning 并跳过，不让一个写到一半的
文件炸掉整个构建（入口文件自身解析失败仍然是硬错误）。

### 同名冲突

整个项目里顶层 `fn` / `struct` / `enum` / `const` 的名字必须唯一，重了直接报错并指出两个文件：

```
error: 函数 'helper' 重复定义
         /proj/a.lume 和 /proj/b.lume
  help: 同名顶层符号在整个项目里只能有一份，改个名字或删掉一份
```

**没有 `pub` 可见性**：合并后所有顶层符号互相可见。要隐藏就靠命名约定
（`_` 前缀之类），真正的模块边界等 `pub` 落地。

### 写库：包的形态

「库」没有特殊语法 —— **一个不含 `fn main` 的 `.lume` 就是库**。
但要能被多个项目复用、还要依赖别的库，就按**包**来放：

| 放法 | 包名 | 谁能用 |
|---|---|---|
| 和入口文件同目录（含子目录） | —— | 这个项目（算模块，不算包） |
| `~/.lume/lib/bigint/*.lume` | `bigint` | 本机所有项目 |
| `$LUME_PATH` 里某个目录下的一级子目录 | 目录名 | 所有指过去的项目 |

```bash
# 一次放好，任何项目里直接用，不需要 import
mkdir -p ~/.lume/lib/bigint
cp big.lume ~/.lume/lib/bigint/

# 或者放在别处，用环境变量指过去（`:` 分隔；Windows 上 `;`）
export LUME_PATH=/opt/lume-libs:/home/me/lume-lib
```

规则：

- **库根下的一级子目录 = 一个包**，包名就是目录名，包内所有 `.lume` 都属于它
  （子目录继续递归收）。
- 库根下直接散放的 `.lume` 也算一个包，包名取文件名 —— 旧布局照样能用。
- 包里带 `fn main` 的文件会被跳过（库目录放示例程序是常态）。
- 包之间、包和项目之间顶层重名照样报错（指出两个文件）。

#### 零 import 也能用

项目里不用写任何东西：你调了 `big_add`，编译器发现这个名字只被 `bigint`
包提供，就把整个包拉进来。和同目录模块的体验完全一致。

#### 库调库

包内部可以 `import 另一个包名` 声明依赖：

```lume
// ~/.lume/lib/bigint/big.lume
import strutil              // 依赖另一个库包

fn big_to_str(a: Big) -> str {
    return strutil_trim(...)  // 直接用对方的符号
}
```

- 每个包在**任一文件**里声明一次就够，整个包共享。
- 依赖不必放在同一个库根：`bigint` 在 `~/.lume/lib`、`strutil` 在 `/opt/lume-libs`
  也可以，只要两个根都在搜索路径里。
- 不写 import 也行 —— 包的方法体里引用了 `strutil_trim`，编译器一样顺着符号找过去。
  `import` 的真正作用是**多个包提供同名符号时指定用哪一个**。

#### 按需加载

只有被用到（或被显式 `import`）的包才参与构建。没被用到的库：

- 不参与编译，不拖慢构建；
- 不会和你的项目或别的库撞名 —— 两个库都有 `parse()`，你只用其中一个，不报歧义。

两个库都提供同一个名字**且真的用到了**才会报错，让你指定：

```
error: 库符号 'dup' 有歧义，同时由 2 个库提供：a / b
  help: 用 `import <库名>` 指定用哪个，或给其中一个函数改个带前缀的名字
```

#### 查依赖

```bash
lume libs main.lume
```

```
项目: main.lume
库搜索根:
  /home/me/.lume/lib

库包（共 3 个）:
  ✔ 加载 [目录] bigint  /home/me/.lume/lib/bigint  依赖: strutil
        提供: Big, big_add, big_mul
  ✔ 加载 [目录] strutil  /home/me/.lume/lib/strutil
        提供: strutil_trim
    未用 [目录] legacy  /home/me/.lume/lib/legacy
        提供: parse
```

只想看加载了哪些包、不想被别的东西干扰：`LUME_LIB_TRACE=1 lume run main.lume`。

#### 已知限制

- 加载判定是**符号级**的：包里导出的名字要能和你代码里出现的名字对上。
  只用到包里的常量或枚举变体也算（会加载）。
- 每次构建都把所有库包的顶层符号扫一遍，**没有缓存**。库特别多时会慢一点。
- 没有版本号、没有锁文件 —— 改 `~/.lume/lib/bigint` 就是全局生效。
- 项目里的 `import` 是「软提示」：找不到对应包时静默忽略（保持老代码兼容），
  库里的 `import` 是硬依赖，找不到直接报错。

**命名**：没有命名空间，合进来就是全局的。两种约定：

```lume
// ① 前缀式：所有顶层符号带库名前缀
fn big_add(a: Vec[int], b: Vec[int]) -> Vec[int] { ... }

// ② 结构体式：名字天然的挂在类型下（推荐，读起来像 a.add(b)）
struct Big {
    limbs: Vec[int]
    fn add(o: Big) -> Big { ... }
    fn to_str() -> str { ... }
}
```

完整例子见 `examples/bigint/`（`big.lume` 是库，`main.lume` 是调用方）：

```bash
lume run examples/bigint/main.lume
```

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
- 生成 C 栈数组（`int64_t nums[] = {...}`），**固定长度**；`[v; n]` 是重复初始化（`n` 必须 > 0）。
- 数组**可以**跨函数传：编译期自动补一个 `{name}_len` 伴随参数，函数里的 `a.len()` 也能用。
- 数组是**引用**语义：`b := a` 只复制指针，改 `b` 就是改 `a`；函数参数也不复制。
  所以不要假设两个参数不重叠（`shift(x, x)` 是合法的，且会互相影响）。
- `a[i]` 有**运行期边界检查**：越界直接 `fprintf + abort`。
  长度和下标都能编译期算出来时（`a[5]`、长度 3）直接报编译错误，不生成 C。
  热点内核想省掉这个分支：单个函数写 `@unchecked`，或整份文件 `lume build --unchecked`。
- 切片 `a[i..j]` / `a[i..=j]` / `a[..j]` / `a[i..]` / `a[..]` 是**零拷贝借用**：
  改切片就是改原数组，边界同样检查，`s.len()` 是切片长度。
- 需要**长度可变**的容器用 `Vec[T]`，见下一节。

---

## 动态数组 `Vec[T]`

数组的长度在编译期就定了。词法器、语法树要的是「边扫边追加」的容器，
所以有一个独立的类型：

```lume
struct Tok {
    kind: str
    line: int
}

fn main() {
    v: Vec[int] = []          // 空
    v.push(10)                // 追加
    v.push(20)
    say("{v.len()}")          // 2

    v[1] = 99                 // 下标写入（有边界检查）
    say("{v[0]} {v[1]}")      // 10 99
    say("{v.last()} {v.pop()}")   // 99 99

    toks: Vec[Tok] = [Tok(kind: "ident", line: 1)]   // 带初值
    toks.push(Tok(kind: "plus", line: 1))

    for t in toks {           // 遍历
        say("{t.kind}")
    }
}
```

方法：

| 方法 | 返回 | 说明 |
|---|---|---|
| `v.len()` / `v.len` | int | 元素个数 |
| `v.is_empty()` | bool | `len == 0` |
| `v.push(x)` | void | 追加，容量不够时按 2 倍扩容 |
| `v.pop()` | T | 弹出最后一个；**空时运行期 abort** |
| `v.last()` | T | 看最后一个；**空时运行期 abort** |
| `v.clear()` | void | `len = 0`，**保留**已分配的容量 |
| `v.reserve(n)` | void | 预分配至少 n 个元素的位置 |
| `v.free()` | void | 释放底层 buffer 和句柄本身 |

规则：

- **必须写类型**：`v: Vec[int] = []`。空字面量推不出元素类型。
  `v := make_vec()` 这类可以省，元素类型从函数返回类型推。
- 元素类型只影响编译期（决定 `v[i]` 按什么类型读、`push` 的实参校验），
  生成的 C 里**只有一个** `LumeVec` 结构体（`data` / `len` / `cap` / `esz`），
  元素大小存在 `esz` 里，push 时 `memcpy` —— 和 `qsort` 同一个套路，
  不随元素类型膨胀代码。
- **引用**语义，和固定数组一致：`w := v` 之后 push 到 `w` 就是 push 到 `v`；
  传进函数也不复制。想要独立副本就再建一个逐个 push。
- 底层 buffer 是 `malloc/realloc` 的**堆**内存，**不进 arena**：
  arena 只增不减，反复 push 长数组会吃满。需要回收就 `v.free()`
  （之后句柄不能再用）；进程退出时交给 OS 也不算错。
- `v[i]` 带运行期边界检查（越界 `fprintf + abort`，报错里带变量名）。
  和其它数组一样，`@unchecked` 或 `--unchecked` 可以关掉。
- `par for` 里**只能读**：`push/pop/clear/reserve/free` 会被类型检查拒绝
  （多个 worker 并发 `realloc` 是数据竞争）。读 `v.len()` / `v[i]` 可以。
- Vec 字段可以放在结构体里（`kids: Vec[Node]`），字面量里写 `kids: []`
  或 `kids: [a, b]` 都行。
- 遍历时不要 push —— `realloc` 会把 buffer 搬走，正在读的指针立刻悬垂。

---

## 哈希表 `Map[K, V]`

自举编译器要符号表，所以除了 `Vec[T]` 还有一个哈希表：

```lume
fn main() {
    m := {"banana": 2, "apple": 5}      // 字面量，K/V 从第一对推
    m.set("cherry", 1)                  // 插入/覆盖
    say("{m.len()}")                    // 3

    a := m.get("apple")                 // 返回 int?（可空）
    if a.has {
        say("{a.val}")                  // 5
    }
    say("{m.has("durian")}")            // false：不建条目

    m.del("apple")
    say("{m.len()}")                    // 2

    for k in m.keys() {                 // 遍历键（顺序由哈希决定）
        say("{k} = {m.get(k).val}")
    }
}
```

方法：

| 方法 | 返回 | 说明 |
|---|---|---|
| `m.len()` | int | 条目数 |
| `m.is_empty()` | bool | `len == 0` |
| `m.get(k)` | `V?` | 查表；没命中 `has == 0`，`val` 零初始化 |
| `m.set(k, v)` | void | 插入或覆盖 |
| `m.has(k)` | bool | 只查，不建条目 |
| `m.del(k)` | bool | 删除；不存在返回 false |
| `m.keys()` | `Vec[K]` | 键的快照 |
| `m.values()` | `Vec[V]` | 值的快照 |
| `m.clear()` | void | 清空，保留桶数组 |
| `m.free()` | void | 释放全部内存（含 str 键的副本） |

规则：

- **空的 `{}` 要写明类型**：`m: Map[str, int] = {}`——`{}` 自身推不出 K/V。
  不写就直接用 `Map[str, int]()`，它自带类型。
- **没有 `m[k]` 下标**：取值得写 `m.get(k).val`。因为 `get` 返回的是 `V?`，
  直接下标就得处理「键不存在时返回什么」，而下标又要能当左值 —— 两者凑
  在一起会生成说不清语义的 C。
- **键只支持 `int` / `str` / `bool` / `rune` / 枚举**。其它类型（尤其是
  `Vec` / `Map` / 数组）按字节比较没有意义，编译期直接拒绝。结构体键能用，
  但里面有指针字段的话 `memcmp` 比的是地址。
- `str` 键会被 **dup 一份**存进表里（runtime 负责 free）；其它键按字节
  memcpy。所以 `m.set(k, v)` 之后改原来的字符串不会影响表里的内容。
- 和 `Vec` 一样是**引用**语义，传进函数改的是同一张表。
- C 侧只有一个 `LumeMap` 句柄（`state`/`keys`/`vals`/`cap`/`len`/`ksz`/`vsz`），
  键值大小存在句柄里，不随 K/V 组合膨胀代码。开地址法 + 墓碑，
  负载超过 3/4 就翻倍 rehash。
- `keys()` / `values()` 返回的是**新 Vec**（快照），改动它不影响表；
  顺序是桶顺序，**不保证插入顺序**，要稳定输出就自己排。
- `par for` 里**只能读**：`set/del/clear/free` 会被类型检查拒绝
  （并发改表是数据竞争）。`get` / `has` / `len` 可以 —— 读共享表是安全的。
- `say(m)` 和 `"{m}"` 会报编译错（句柄指针打出来是地址）。要打就
  遍历 `.keys()`。

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

### @unchecked

```lume
@unchecked
fn dot(a: [float], b: [float], n: int) -> float {
    s := 0.0
    for i in 0..n {
        s = s + a[i] * b[i]     // 不插 lume_idx_check
    }
    return s
}
```

- 关掉**这个函数内**的 `a[i]` 运行期边界检查。越界会退回静默读写内存（和 C 一样）。
- 只该用在两种情况下：下标已经手工证明在界内；或者边界分支挡住了向量化。
  一般情况下 clang 能把检查消掉（`for i in 0..a.len()` 这种它自己就能证明），先用
  `-Rpass=loop-vectorize` 看清楚再决定，别一上来就加。
- 整份文件想关：`lume build foo.lume --unchecked`。

---

## extern：直接调 C

`extern fn` 只声明签名、不写函数体，编译出来的 C 就是一个原型，
链接时去 libc / 系统库 / 你 `-l` 进来的库里找符号。

```lume
extern fn strlen(s: str) -> int
extern fn strcmp(a: str, b: str) -> i32
extern fn atof(s: str) -> f64
extern fn getpid() -> i32
extern fn printf(fmt: str, ...) -> i32

@link("m")
extern fn tgamma(x: f64) -> f64

fn main() {
    s := "hello"
    say("len = {strlen(s)}   pid > 0 = {getpid() > 0}")
}
```

生成：

```c
extern int64_t lume_ext_strlen(const char* s) __asm__("strlen");
extern int32_t lume_ext_strcmp(const char* a, const char* b) __asm__("strcmp");
extern int32_t lume_ext_printf(const char* fmt, ...) __asm__("printf");
```

### 为什么要 `lume_ext_` 前缀 + `__asm__`

直接用原名声明会和系统头文件撞类型：`<stdio.h>` 里是
`int puts(const char*)`，Lume 的 `int` 是 `int64_t`，两个声明同作用域
就是 conflicting types。加前缀后两个标识符互不相识，`__asm__("puts")`
把符号名指回去，链接期照常解析。所以 `puts` / `printf` / `fopen`
这些都能安全声明，不用管头文件。

### 宽度必须自己对齐

编译器**不做**任何 ABI 检查，写错就是静默算错：

| C 那边的类型 | Lume 里写 |
|---|---|
| `int`、`pid_t`、`unsigned int` | `i32` |
| `long`、`long long`、`size_t`、`ssize_t`、指针 | `int` |
| `double` | `f64` |
| `float` | `f32` |
| `char*`、`const char*` | `str` |
| `void*` | 没有对应类型（拿 `int` 顶着，别解引用） |

- 写成 `-> int` 而真实返回 `int` 时，负数会变成大正数（`int` 的返回值
  只写 eax，高位被清零）——**所以 `int` 一律写 `i32`**。
- 指针类型没有专门语法。要传 `NULL` 就传 `0`（`int64_t` 的 0 在寄存器
  里就是全零，和空指针同表示）。

### 变参 `...`

```lume
extern fn printf(fmt: str, ...) -> i32
```

- `...` 之前声明的参数仍然**必须逐个传**，并且做类型检查；`...` 之后
  多传的只求值、不检查。
- varargs 的提升规则是 C 的：`float` → `double`，小于 `int` 的整数 → `int`。
  所以 `%lld` 必须配一个 `int` 变量，直接传字面量 `42` 会退化成 C 的
  `int`，`%lld` 读 64 位就是垃圾。要传就 `x := 42` 再 `printf("%lld", x)`。

### `@link("名字")`

给 clang 加 `-l<名字>`（`@link("m")` → `-lm`）。写在任意一个 `extern fn`
上即可，整份程序共享，重复的自动去重。`-lm` / `-lpthread` 默认已经带了。

### 已知限制

- 不能声明 extern **全局变量**（只支持 `fn`）。
- 不能取 C 函数指针（没有函数指针类型），所以 `qsort` / `signal` /
  `atexit` 这类要回调的接不进来。
- 不能把 Lume 函数导出去给 C 调（`main` 除外）。
- Windows 上 `__asm__` 标签是 GCC/Clang 扩展，MSVC 不认；本项目只用 clang。

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
- 自动按 `cpu_count` 切块、最多 64 个任务块，引用的外部数组打包进 worker 结构体。
  线程池常驻，主线程也参与抢任务。
- worker 函数生成在文件顶层，不嵌在函数里 —— clang / gcc / MSVC 都能编过。
- 循环体支持：赋值、`let`、`if`（可嵌套）；其他语句生成 `/* par body: 暂不支持 */`。
- `0..N` 与 `0..=N` 切块都正确，区间按 `[start, stop)` 均分给各线程，不会重复也不会漏。
- ⚠ 循环体里不要 `say`（多线程 printf 交错）。
- 循环体里的只读外层变量按值传进 worker；**写外层变量会直接 `#error`**
  （多线程写同一个变量是数据竞争，宁可编译不过也不要错得悄无声息）。
- **不支持嵌套** `par for`，会 `#error`。
- 元素数 < `LUME_PAR_SERIAL_THRESHOLD`（32768）时自动串行跑，不进线程池。
- 嵌套并行（`par for` 里再 `par for`，含经函数间接调用）运行期退化成串行。

**自动 reduction**：识别 `v += rhs` / `v -= rhs` / `v *= rhs`，每个线程用私有累加器，
结束后合并回 `v`，等价于 OpenMP 的 `reduction(+:v)`。触发条件很严格：

- `v` 不是循环变量，循环体里没有被 `let` 重新声明
- `v` 在整个循环体里**只出现在这些累加式里**（出现次数必须正好是 `2 × 累加语句数`）。
  循环体里再多读一次 `v`，编译器就放弃归约
- 同一个 `v` 上不能混用不同运算符（合并顺序会影响浮点结果）

⚠ 不满足条件时**不归约也不加锁**，多线程写同一个变量就是数据竞争，结果随线程数变。
要并行求和就老老实实写 `total += a[i]`。

⚠ 减法的私有累加器是**按加法收集、合并时统一减**的。看着别扭，但这是对的：
如果累加时就用 `-=`、合并时又 `-=`，负号会被减两次，`-55` 会变成 `+55`
（用隔离探针验证过，别"顺手修"它）。

---

## 内置函数

直接可用，**不需要 import**：

| 函数 | 返回 | 说明 |
|---|---|---|
| `say(...)` | void | 打印，支持插值 |
| `write(...)` | void | 同 `say` 但不换行 |
| `eprint(...)` | void | 同 `say`，输出到 **stderr**（同步写） |
| `ewrite(...)` | void | 同 `write`，输出到 **stderr** |
| `free(x)` | void | 释放 owned 数组（函数/`map`/`filter`/管道新建的） |
| `now()` | int | 当前纳秒时间戳（单调时钟） |
| `read_file(path)` | `str?` | 读整个文件；失败返回 `None` |
| `write_file(path, s)` | bool | 覆盖写；失败返回 `false` |
| `append_file(path, s)` | bool | 追加写；失败返回 `false` |
| `file_exists(path)` | bool | **普通文件**存在（目录返回 `false`） |
| `remove_file(path)` | bool | 删除；失败返回 `false` |
| `arg_count()` | int | 命令行参数个数（含程序名） |
| `arg(i)` | str | 第 `i` 个参数；越界返回 `""` |
| `exit(code)` | void | 立即退出进程（先 flush 输出） |
| `str_len(s)` `str_to_int(s)` `str_to_float(s)` | int / float | 长度、解析 |
| `str_upper(s)` `str_lower(s)` `str_trim(s)` | str | 大小写、去首尾空白 |
| `str_replace(s, old, new)` | str | 全部替换 |
| `str_slice(s, start, end)` | str | 取 `[start, end)`，越界自动夹边 |
| `str_index_of(s, sub)` | int | 子串首次位置，没有则 `-1` |
| `str_repeat(s, n)` | str | 重复 `n` 次 |
| `str_split(s, sep)` | [str] | 按 `sep` 切分（段数不限，`sep` 为空则按字符切） |
| `str_contains(s, sub)` `str_starts_with` `str_ends_with` | bool | |
| `int_to_str(x)` `float_to_str(x)` | str | 数字 → 字符串 |
| `str_arena_reset()` | void | 释放所有派生字符串（见下） |
| `str_arena_bytes()` | int | arena 当前已用字节数，用来量内存 |
| `int_abs(x)` `int_min(a,b)` `int_max(a,b)` | int | |
| `float_sqrt(x)` / `sqrt(x)` | float | 两种名字都行 |
| `float_abs` `float_pow`/`pow` | float | |
| `float_sin`/`sin` `float_cos`/`cos` `float_tan`/`tan` | float | |
| `float_floor`/`floor` `float_ceil`/`ceil` `float_round`/`round` | float | |

### 文件 IO、命令行与 stderr

自举的地基：编译器要先把一个 `.lume` 文件整个吃进来，再按 `argv` 决定干什么。

```lume
fn main() {
    src := read_file("main.lume")
    if !src.has {
        eprint("error: 打不开 main.lume")
        exit(1)
    }
    say("读到 {str_len(src.val)} 字节")
}
```

几条必须记住的规矩：

- **`read_file` 失败返回 `None`，不是 `""`。** 空文件是合法内容，
  用空串表示失败就再也分不清「读到了空文件」和「打不开」。
- **读到的内容是普通 `str`，从 arena 切**，和 `str_upper` / 拼接同类：
  不用也不该 `free`，生命周期跟 arena 走（`scope` 能回收）。
- **只能读普通文件。** 实现是 `fseek` + `ftell` 取长度再 `fread`，
  管道和终端不可 seek，会返回 `None` —— 宁可失败也不返回半截数据。
- **`write_file` 是整体覆盖**，不是流式追加。要追加用 `append_file`。
- **`file_exists` 只认普通文件**（`S_ISREG`），目录返回 `false`，
  免得写了 `read_file(some_dir)` 才发现读不出来。
- **`arg(0)` 是程序自己的路径**，不保证是绝对路径；越界和负数返回 `""`。
- **`exit(n)` 会先排空输出**。全程序的 `say` 走异步队列，
  不 flush 直接 `exit` 会丢掉最后几行。
- **`eprint` / `ewrite` 是同步写**，且写之前会先 flush stdout 队列 ——
  否则同一段程序里 `say` 的内容会跑到 `eprint` 后面去，读起来像时序错了。
  错误信息必须立刻出现（下一行可能就是 `exit`），所以这里不走进队列。
- **可空值不能直接 `say` 或插值**：`T?` 在 C 里是 `{ has, val }` 结构体，
  塞进 varargs 会把 `has` 当指针读。编译期直接拒绝，先 `.has` / `.val`。

### 数组所有权与 `free`

- `let r = f()` 且 `f` 的 `return` 是 `[..]` / `[v; n]` / 局部数组 → `r` 是
  **owned**，用 `free(r)` 释放。
- `map` / `filter` / `|>` 管道的结果同样是 owned。
- 字面量、数组参数、切片、`return a`（返回入参）得到的数组**不是** owned，
  对它们调用 `free` 会在编译期报错。
- `b := a` 是别名：`a` owned 时 `b` 也共享同一块内存；`free` 其中一个后，
  其余别名一并失效（和 C 手动内存管理一样）。
- `par for` 循环体里不能 `free`（多个 worker 会重复释放）。
- `free(x)` 只能作为语句；写进插值或 `let` 会报类型错误。
- 忘记 `free` 仍会泄漏；自动释放/值语义还没做。

其他 C 标准函数**没有 extern 通路**，用不了。文件 IO、`argv`、`exit`、`stderr`
是编译器内置的特例（见上面的函数表），不经过 `extern`。

### 字符串内存：arena

`str` 就是 C 的 `const char*`。派生字符串（拼接、`str_upper`、`str_slice`……）
全部从一块**只增不还**的 arena 里切，`str_arena_reset()` 一次性释放全部。

- **任意多个字符串可以同时存活**。过去是 8 个静态槽轮转，连续调 9 次就开始
  互相覆盖 —— `str_upper(a)` 的结果可能被 `str_upper(b)` 冲掉，而且不报错。
  现在没有这个限制。
- **没有 4095 字符上限**。过去超了静默截断。
- **每次派生都是新内存**，`s + t` 不和任何已有字符串别名，改不了别人。

代价是内存只增不减：循环里反复拼接会持续占用，直到有人回收。
两种回收方式：

1. **`scope { ... }` 块**（首选，局部回收）：出块时把块内派生的字符串一次还回去，
   块之前的字符串不受影响。可以嵌套，`break` / `continue` 会在跳转前补回收。
   详见「控制流 → `scope` 块」。
2. **`str_arena_reset()`**（全局回收）：清空整个 arena。之后**所有** `str` 都变成
   悬垂指针（包括变量里存着的那些），所以只在一批临时字符串确实用完时调 ——
   比如解析完一个源文件之后。

`str_arena_bytes()` 返回当前已用字节数，用来确认某段代码到底吃了多少内存。

字面量（`"abc"`）是静态数据，不进 arena，`reset` 碰不到它们。
`par for` 的 worker 线程共用同一个 arena，内部有锁保护（但 `scope` 在
`par for` 里被禁止，见上文）。

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

**不检查**：泛型函数体、数组越界（`Vec` 与已知长度的固定数组除外，那两个
是运行期检查）。

---

## 已知限制速查

| 别写 | 原因 |
|---|---|
| `trait` `impl` `async` `await` `spawn` `app` `use` `extern` | 能解析，未实现 |
| `import 不存在的名字` | 项目里当软提示静默忽略；**库里**是硬依赖，找不到直接报错 |
| 闭包捕获 | 未实现 |
| 带数据的枚举 `Num(int)` | 未实现：变体只能是无载荷的整数常量 |
| `if t == 0`（枚举和整数混比） | 类型不同，编译期报错；写 `Tag.X` |
| 枚举变体的值重复 | 编译期报错（C 里 `switch` 会出现 duplicate case） |
| `Vec[T]` 不写元素类型（`v := []`） | 推不出元素类型，编译期报错 |
| `par for` 里 `push` / `pop` / `clear` / `reserve` / `free` | 编译期报错：多 worker 并发 `realloc` 是数据竞争 |
| 遍历 `Vec` 的同时 push | `realloc` 会搬走 buffer，正在用的元素指针悬垂 |
| `v.free()` 之后再用 `v` | 手动所有权，编译器不跟踪 use-after-free |
| `free(字面量/数组参数/切片)` | 不是新分配的堆数组，编译期报错 |
| `free` 后再用同一变量 | 手动所有权：free 后变量失效，编译器不跟踪 use-after-free |
| 别名重复 `free` | 不检查（`b := a` 后 free 两次会 double free） |
| 嵌套块里 free 同名变量 | owned 跟踪是函数级/按名字，外层同名变量会误报 |
| `total(make3())` | 返回数组的调用不能直接作实参，先 `let r = make3()` |
| `map(arr, f)`（`arr` 是 struct 字段） | 字段数组作为 HOF 源还不支持 |
| `E()`（空 struct） | 会被当成函数调用；struct 至少写一个字段 |
| 泛型函数里调泛型函数 | 不会单态化，生成未定义函数 |
| `s.arr[i]`（struct 字段数组）、`f()[i]`（函数返回的数组） | 长度拿不到，**不做**边界检查（仍然是静默越界） |
| `str_arena_reset()` 之后继续用旧的 `str` | 全部变悬垂指针，编译器不跟踪 |
| 循环体里反复拼接字符串却不用 `scope` | arena 只增不减，要自己划定回收范围 |
| `scope` 里 `return 一个字符串` | 不回收（故意泄漏一次），否则返回悬垂指针 |
| `par for` 循环体里用 `scope` | 编译期报错：多 worker 共享 arena，回退会踩坏别人的字符串 |
| `"n=" + 1`（str 和非 str 相加） | 不做隐式转换，编译期报错；用插值 `"n={n}"` |
| `read_file(目录)` / `read_file(管道)` | 只读普通文件：`fseek` 取长度失败就返回 `None` |
| `say(可空值)` / `"{可空值}"` | `T?` 是 `{ has, val }` 结构体，进 varargs 是 UB，编译期拒绝 |
| `say(struct)` / `"{struct}"`（没定义 `op_str`） | 同上：结构体进 varargs 是 UB，编译期拒绝，提示加 `op_str` |
| 结构体上 `a + b`（没定义 `op_add`） | 编译期报错，提示要写哪个 `op_*` |
| 重载 `and` / `or` / `|>` | 这三个不走运算符重载 |
| `let x := None` | 推断成 `int?`；要别的类型得写 `let x: str? = None` |
| `m[k]`（Map 下标） | 不支持：`get` 返回 `V?`，下标又要能当左值，凑不出清晰语义。写 `m.get(k).val` |
| `m := {}`（空字面量） | 推不出 K/V。写 `m: Map[str, int] = {}` 或 `Map[str, int]()` |
| `Map[Vec[int], int]` 之类指针键 | 按字节比较没有意义，编译期拒绝 |
| `say(m)` / `"{m}"` | 句柄指针打出来是地址，编译期拒绝；遍历 `.keys()` |
| `par for` 里 `set` / `del` / `clear` / `free` | 编译期报错：并发改表是数据竞争（读可以） |
| 指望 `keys()` 按插入顺序排列 | 顺序是桶顺序（哈希决定），要稳定就自己排 |
| `x := 1` 之后又 `x := 2` | `:=` 一律新建变量，重名是 C 的 redefinition 错误；改值用 `x = 2` |
| 变量名叫 `it` / `u8` / `scope` 之类 | 它们是关键字/类型关键字，报的是「期待表达式」，不好联想 |
| 拿 `write_file` 当流式写 | 每次调用都是一次完整的 open/write/close，内容全在内存里 |
| `extern fn` 里 `int` / `i32` 写混 | 编译器不查 ABI：`int` 返回 `int` 会被读成零扩展的大正数 |
| 想 `extern` 一个回调 / 函数指针 | 没有函数指针类型，`qsort` / `signal` 接不进来 |

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
5. 数组参数**没有** `restrict`：数组是引用语义、可能别名。想让 clang 更放心地
   向量化，就避免把同一个数组同时传给一个函数的多个参数。
6. 写完可以看一眼生成的 `<文件名>.c`：确认你的热点函数有没有拿到
   `LUME_PURE`、循环前面有没有 `#pragma clang loop vectorize`。
