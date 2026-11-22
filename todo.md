# Lume v0.1 精简设计稿（自举导向，轻量化改版）

## 实现进度（自举落地）

- [x] 建立单一 Rust/Core 工作区与 `lume-bootstrap` 二进制
- [x] 实现 IR、JIT、GC 根栈和类型基础
- [x] 实现外部 LLVM bitcode 加载、解析和链接
- [x] 提供文件读写、外部进程和 GC 根操作的核心接口
- [x] 建立 `lib/syntax` 的 Lume 自举源码骨架
- [x] 建立 `lib/build` 的 Lume 构建源码骨架
- [x] 建立 `lib/std` 与内置包装源码骨架
- [x] 建立基础示例、文档和工作区配置
- [x] 补齐词法、语法、表达式求值和循环执行
- [x] 支持 `check/run` 与 manifest 入口
- [x] 开发 VS Code 语法高亮、诊断、补全、悬停和符号

> 
> 核心修改点：
> 
> 
> 1. ✅ **`ret` 直接终止当前块，不需要对应的 `end`**；函数、if‑else、while，遇到 `ret` 直接退出块。
> 2. ✅ 只有**没有写 ret 正常掉落的块才需要 `end`**；`ret` 等价于隐式块结束。
> 3. ✅ 支持**块表达式：`if / while` 可以产出值，直接赋值给变量**（替代笨重 lambda，块本身就是值来源）。
> 4. ✅ 保留：海象`:=`、类型标注`x:int=10`、`and or not is`。
> 5. ✅ 砍掉冗余描述，只保留实际要用的语法、AST、和 lumec_core IR 映射。
> 6. 块规则：遇到`ret`立刻跳出当前作用域；没有 ret，则必须写`end`闭合块。

## 关键字

```
fn ret if else while and or not is true false end
```

## 符号

```
() [] , : = := + - * / % < <= > >= == !=
```

- `:` 类型标注；`=`普通赋值；`:=`海象赋值（表达式，有返回值）

## 注释

`# 单行注释`，无块注释

## 字面量

- 整数：`10` `-123`
- 字符串：`"abc"`
- 布尔：`true` `false`

## 运算符优先级（高→低）

1. 一元：`not`、负号`-`
2. `* / %`
3. `+ -`
4. `< <= > >= == != is`
5. `and`
6. `or`
7. `:=`（海象，最低）

---

# 语法规则

## 1. 函数定义

> 
> 规则：
> 
> 
> - 函数体内部，如果执行到`ret`，**直接结束函数，不需要写 end 在函数尾部**。
> - 如果函数全程没有任何`ret`掉落执行到末尾，必须写`end`闭合函数。

```
# 方案A：ret直接终止函数，末尾不需要end
fn add(a:int, b:int) -> int
    c:int = a + b
ret c

# 方案B：无ret，必须写end
fn nop()
    x:int=10
end
```

示例求和：

```
fn sum_to(n:int) -> int
    s:int=0
    i:int=1
    while i <= n
        s := s + i
        i := i + 1
    end
ret s
```

## 2. 变量声明 & 赋值

```
# 声明+类型+初始化（必须带初始化）
x:int=10
y:i16=20

# 普通赋值（语句，无返回）
x = 100

# 海象赋值（表达式，有返回；只给已存在变量，不能写类型）
x := 200
```

海象经典场景：

```
buf:string=""
if buf:=read_line()
    print(buf)
ret 1
end
```

## 3. if‑else

> 
> 两条路径：
> 
> 
> 1. 分支内部写`ret` → 该分支直接结束，**这个分支不用 end**
> 2. 分支没有 ret，正常向下执行 → 必须用`end`关闭 if 块
> 3. if 是**块表达式，可以产出值直接赋值给变量**（替代 lambda 的核心特性）

### 案例 1：分支用 ret 提前返回

```
fn max(a:int, b:int) -> int
    if a > b
        ret a
    else
        ret b
    # 全部分支都ret了，不需要写end
```

### 案例 2：if 作为表达式，产出值赋值变量（重点，代替 lambda）

> 
> 没有 ret，所以 if 块末尾必须写`end`

```
fn max2(a:int,b:int)->int
    res:int = if a>b
        a + 10
    else
        b + 20
    end
ret res
```

> 
> ⚡这里：`if ... end`整体是一个表达式，计算产出一个值直接赋给`res`，**不需要单独定义小函数 /lambda**。

支持`else if`：

```
sign:int = if x>0
    1
else if x<0
    -1
else
    0
end
```

## 4. while 循环

> 
> 1. 循环体内遇到`ret`直接跳出整个函数；
> 2. 如果没有 ret，循环块必须以`end`结束；
> 3. while 同样是块表达式，产出值（循环最后执行表达式结果）可赋值。

```
fn demo()
    i:int=0
    while i<5
        print(i)
        i:=i+1
    end
ret 0
```

while 作为表达式赋值：

```
val:int = while buf:=next()
    buf
end
```

## 5. ret 语义

```
ret          # 返回单元占位值（0）
ret a + b    # 返回表达式，立刻终止当前作用域（if/while/function）
```

> 
> 重点：
> 
> 
> - 在`fn`内部执行`ret`：直接退出函数。
> - 在`if`块内部执行`ret`：不是只退出 if，**直接退出外层函数**。
> - `ret`只做跳出，**不充当普通表达式，只能做语句**。

> 
> 如果你想要 “局部块产出值而不 return 函数”，就用 `if ... end` / `while ... end` 块表达式赋值变量，不要用 ret。
> ret = 函数返回；块表达式 = 块产出值给局部变量，二者分离。

## 6. 数组、结构体字面量（自举编译器需要）

```
[]
[1,2,3]
Token{kind:"ident", text:"fn", start:0, end:2}
arr[0]
```

## 完整示例合集

```
# ret直接结束函数，无end
fn add(a:int,b:int)->int
    c:int = a + b
ret c

# if全部分支ret，不需要end
fn max(a:int,b:int)->int
    if a > b
        ret a
    else
        ret b

# if作为块表达式赋值，无ret，必须end
fn compute(a:int)->int
    res:int = if a > 10
        a * 2
    else
        a + 5
    end
ret res

# while循环
fn sum_to(n:int)->int
    s:int=0
    i:int=1
    while i <= n
        s := s + i
        i := i + 1
    end
ret s

# 海象
fn read_demo()
    line:string=""
    if line:=get_line()
        print(line)
        ret 1
    end
ret 0

fn main()
    print(add(10,20))
ret 0
```

---

# 块规则总结（牢记，parser 核心逻辑）

1. `ret`执行：**立刻终止所在函数，不再继续解析本函数后续语句；ret 所在分支不需要 end。**
2. 块（fn /if/else /while）如果**没有任何 ret、程序会向下继续流动**，必须以`end`关闭块。
3. `if ... end`、`while ... end`是**块表达式**，可以放在等号右边，产出一个值给变量。这就是你要的 “类似 lambda 但更好”：不需要定义匿名函数，直接把一段逻辑块当作表达式使用。
4. `end`只负责闭合**没有被 ret 截断的块**。

> 
> 对比 lambda：
> lambda：`f = || { ...; return x }` 要定义匿名函数再调用。
> Lume 块表达式：`v = if cond { ... } else { ... } end`，直接运行块，拿到结果赋值，无函数调用开销，直接 lowering 到 IR phi 节点。

---

# AST 精简定义

```
File
└── FuncDef { name, params:[Param], ret_ty?, body:[Stmt] }

Param { name, ty? }

Stmt
├─ VarDecl { name, ty?, init:Expr }      # x:int=10
├─ Assign { name, expr:Expr }            # x = expr
├─ Ret { expr?:Expr }                    # ret / ret expr
├─ ExprStmt { expr:Expr }                # 函数调用语句

Expr
├─ IntLit / StrLit / BoolLit
├─ Ident {name}
├─ Binary {op, lhs, rhs}                # + - * / and or is == ...
├─ Unary {op:not / neg, inner}
├─ Call {callee, args}
├─ Index {base, index}
├─ ArrayLit {elements}
├─ StructLit {type_name, fields}
├─ Walrus {name, expr}                   # x:=expr 海象赋值表达式
├─ IfExpr {cond, then_body:[Stmt], else_body:[Stmt]}    # if块表达式
└─ WhileExpr {cond, body:[Stmt]}                        # while块表达式
```

## 到 lumec_core IR 的映射要点

1. `ret e` → `IrOp::Return(ValueId)`，直接函数返回。
2. `IfExpr` / `WhileExpr` 块表达式：内部语句执行，最终产出值走`Phi`节点得到 ValueId，可直接用于赋值。
3. `and / or` 翻译成短路控制流 IR，不是位运算。
4. `not` 翻译成条件比较。
5. `is` 调用 builtin 做指针身份比较。
6. 海象`Walrus`：计算表达式，写入局部槽，返回 ValueId，可用于 if/while 条件。

## 引导阶段暂时不实现

- 闭包、for 循环、泛型、方法调用`obj.foo()`、模式匹配、块注释。
- 类型检查（只解析 AST，类型字符串保留，lower 到 IR 直接丢弃）。

## 边界重点

> 
> - ret 只管**函数返回**，不是块局部返回；if 里面写 ret 直接退出整个函数。
> - 需要 “局部块产出值” 就用`if ... end` / `while ... end`块表达式赋值变量，不使用 ret。
> - end 只用来闭合没有被 ret 截断的块。

# Lume v0.1 设计稿 扩展：多语言混合编译，统一输出 LLVM

> 
> 核心目标：

1. Lume 自身语法（前面精简版）
2. **多语言混编：Lume / C / 后续其他语言，同一个项目，统一编译产出 LLVM IR/bitcode**
3. 全部最终下沉到 LLVM，由 lumec_core JIT 执行；也可 AOT 输出 obj / 可执行文件
4. 库可以由多种语言共同编写，对外一套统一 ABI；**不做跨语言 GC 自动穿透，边界靠显式 FFI 接口隔离**
5. 保留之前全部规则：`ret`直接退出函数、块表达式替代 lambda、`:=`海象、`x:int=10`类型标注、`and or not is`

> 
> 关键认知：
> 不是让 Lume 解析别的语言源码；而是**项目模块可以指定源码语言，各自的前端把源码翻译成统一 LLVM IR /bitcode，全部喂给 LLVM 链接层**。
> lumec_core 底层本来就是 LLVM，天然可以加载外部 LLVM‑bitcode 模块。

## 整体架构分层

```
用户项目（lume项目）
├─ src/
│  ├─ main.lume       # Lume源码，Lume前端 → LLVM‑IR
│  ├─ math.c          # C源码 → clang编译为 *.bc (LLVM bitcode)
│  └─ helper.rs       # 未来：Rust源码 → rust‑llvm‑bitcode
├─ manifest.toml      # 项目清单：声明每个模块的语言、依赖、ABI导出、符号可见性
│
# 构建链路
[各个语言前端] → 输出 LLVM IR / Bitcode (.bc)
        ↓
lume‑build（构建工具）：把所有bc合并、链接、优化
        ↓
两种输出模式
  1) JIT模式：喂给 lumec_core 直接运行（内存加载bitcode）
  2) AOT模式：编译成native目标文件/可执行程序

# 运行时边界
- Lume侧：自己的IR、GC、Tagged‑Value、Lume类型系统
- 外部语言(C等)：**纯C ABI i64 / 指针**，不感知Lume GC；只能通过FFI边界交互
```

> 
> ⚠️重要约束：
> 外部 C / 其他语言**不能直接操作 Lume GC 堆对象内部**；只能传：整数、原始指针。
> 如果要传递 GC 对象：传递不透明指针`*mut u8`给外部；外部不能读写对象字段；回传到 Lume 侧再重新包装成`Value`。
> GC 扫描不会扫描 C 语言栈，C 调用 Lume 的时候必须手动把 Lume 对象压入 GC 根集合，防止被回收。

## 1. manifest.toml 项目清单格式（多语言库描述文件）

> 
> 每个库 / 项目根目录，`manifest.toml`，lume‑build 读取，识别各个源码模块语言，控制导出符号、ABI。

```
[project]
name = "demo_lib"
version = "0.1.0"
# 编译目标模式：jit | aot
build_mode = "jit"

[[module]]
path = "main.lume"
lang = "lume"
# 是否对外导出符号给其他模块调用
export = ["entry", "calc_sum"]

[[module]]
path = "math.c"
lang = "c"
# clang参数，生成llvm bitcode
cflags = ["-O1"]
export = ["c_matmul"]

# 未来可扩展 lang = "rust" / "zig"
# [[module]]
# path = "helper.rs"
# lang = "rust"

# FFI绑定声明：Lume侧的签名，绑定外部语言符号
[ffi.extern]
# Lume里面可以 call("c_matmul", [a,b])
c_matmul = { params = ["i64", "*u8"], ret = "i64" }

# 导出Lume函数，给C调用（C ABI）
[ffi.export]
calc_sum = { params = ["i64"], ret = "i64" }
```

字段说明：

- `lang`: `lume` / `c`；后续扩展`zig`等；
- `export`: 本模块哪些函数符号对外可见；私有函数不导出，内部链接；
- `[ffi.extern]`：**Lume 侧声明外部符号签名**，Lume IR 可以`Call("c_matmul", args)`调用 C 写的函数；
- `[ffi.export]`：把 Lume 函数暴露为 C‑ABI 符号，C 代码可以调用 Lume 写的函数。

## 2. 各语言产出物

表格

| 源码 | 前端工具 | 输出产物 |
| --- | --- | --- |
| `.lume` | Lume 引导前端 / 自举编译器 | LLVM IR 文本 或者 `.bc` bitcode |
| `.c` | clang | `clang -emit‑llvm‑bitcode math.c -o math.bc` |

> 
> lume‑build 做的工作：
> 
> 
> 1. 遍历 manifest 所有 module，按对应语言编译全部输出`.bc`
> 2. 调用 LLVM 工具链：llvm‑link 把全部 bitcode 合并为一个整体 module
> 3. 做内部链接、优化、符号裁剪
> 4. JIT 模式：把合并后的 bitcode 加载进 lumec_core 的 ExecutionEngine 直接运行
> 5. AOT 模式：llc 编译为目标文件，链接得到可执行 / 静态库

### lumec_core 新增能力（很小改动，复用现有 LLVM 设施）

给`LumeCore`增加接口：**加载外部 LLVM bitcode 文件 / 字节流**

```
impl LumeCore {
    /// 加载外部LLVM bitcode，和现有的JIT module合并
    pub fn load_bitcode(&mut self, bytes: &[u8]) -> Result<(), CoreError> {
        // 1. 只有未编译状态可以加载bitcode
        self.ensure_not_compiled("load bitcode")?;
        // 2. 解析bitcode，导入到内部module
        // 3. 外部符号可以被IrFunc的Call指令直接调用
    }
}
```

> 
> 这样：C 编译出来的`.bc`，直接喂给 lumec_core，Lume 的 IR 的`Call(name, args)`就可以调用 C 函数；C 也可以调用导出的 Lume C‑ABI 函数。

## 3. FFI 调用规则（统一跨语言 ABI）

> 
> **跨语言边界统一使用 C ABI，只允许基础类型：i64、u64、指针`*u8`。**
> Lume 内部丰富类型（GC 对象、Tagged Value、字符串）**不能直接跨 FFI 边界传递**。

### 场景 A：Lume 调用 C 函数

1. 在`manifest.toml`的`[ffi.extern]`声明 C 函数签名；
2. Lume IR 直接`Call("c_matmul", [arg0, arg1])`；
3. 参数只能传 i64 或者 GC 对象转出来的原始指针；
4. 如果传 GC 对象指针：调用 builtin 把该对象压入 GC 根，防止执行 C 代码过程被 GC 回收。

示例，Lume 代码调用 C：

```
fn lume_run_c() -> int
    buf:pointer = alloc_buf(1024)
    # 调用C实现的c_matmul，传入整数+裸指针
ret c_matmul(1024, buf)
end
```

### 场景 B：C 调用 Lume 的函数

1. `manifest.toml` `[ffi.export]`声明要导出的 Lume 函数；
2. Lume 编译器把该函数生成**标准 C ABI**的 LLVM 函数；
3. C 代码可以声明原型直接调用；
4. Lume 函数被 C 调用进入时：builtin 自动把 C 传入的指针标记为 GC 临时根。

C 侧头文件伪示例（lume‑build 可以自动生成）

```
int64_t calc_sum(int64_t n);
```

## 4. Lume 语法层面 FFI 扩展（极少新增语法）

不需要大量新语法，**全部在 manifest.toml 做声明**，Lume 代码只做两件事：

1. 调用外部符号，直接普通 call：`c_matmul(a,b)`
2. 如果要导出 Lume 函数给外部，函数上加注解（属性）

```
#[ffi_export]
fn calc_sum(n:int) -> int
    s:int=0
    i:int=1
    while i <= n
        s:= s + i
        i:= i +1
    end
ret s
```

> 
> `#[ffi_export]`属性：告诉编译器，此函数生成 C‑ABI，符号对外导出；对应 manifest 的`ffi.export`。

> 
> 外部导入不需要语法，全部写在 manifest；Lume 直接 call 符号名即可。

## 5. 之前 Lume 语法不变（完整继承）

> 
> 下面全部保留，不受多语言功能影响

- `fn ...` / `ret`直接退出函数，无 ret 才需要`end`
- 块表达式 `res:int = if cond ... end`（替代 lambda）
- 海象 `:=`、普通赋值`=`
- 类型标注 `x:int=10`
- `and or not is`
- 数组、结构体字面量

## 6. AST 微小扩展

只新增函数属性标记，其余 AST 完全不变：

```
FuncDef {
    name,
    params,
    ret_ty?,
    body,
    attrs: Vec<Attr>   // 新增：#[ffi_export]
}

Attr::FfiExport
```

## 7. 自举关系

1. **Rust 写的 lumec_core 底座**：LLVM JIT、加载 bitcode、GC、Lume‑IR 执行。
2. `lume‑build`构建工具：初期 Rust 写（一次性引导工具）；未来**可以用 Lume 自举重写整个构建工具**。
3. 多语言模块：C/clang 负责把 C 代码转 bc；不需要 Lume 去理解 C 语法。

> 
> 关键点：Lume 不用写 C 解析器，利用 clang 做 C 的前端，大家共同输出 LLVM bitcode，在 LLVM 层汇合。

## 8. 边界限制（必须明确，避免幻想）

1. ❌ 不支持自动跨语言 GC：C 代码内部保存 Lume GC 对象，**必须手动调用 builtin 压入 GC 根集合**，否则对象会被回收悬空。
2. ❌ 不支持直接把 Lume `bigint` /string/array 传给 C；只能传裸指针；C 只能读原始内存字节，不能识别 Lume 对象头。
3. ✅ 支持：整数、裸指针跨语言传递。
4. ✅ 全部模块统一编译到 LLVM IR /bitcode，可 JIT 可 AOT。
5. ✅ 库可以混合`.lume` + `.c` 一起编译为一个 JIT 模块或者一个 AOT 库。

## 9. 开发落地步骤（按顺序）

1. 在`LumeCore`增加`load_bitcode(bytes)`接口，读取外部 llvm bitcode，和本 module 合并。（改动很小，复用 inkwell/llvm 接口）
2. 设计`manifest.toml`解析（Rust 写引导版 lume‑build）
3. 实现逻辑：调用 clang 把`.c`编译成`.bc`，llvm‑link 合并所有 bitcode。
4. Lume 语法增加函数属性`#[ffi_export]`，lower 到 IR 时标记该函数生成 C‑ABI。
5. 实现 GC 根相关 builtin：`gc_push_root(ptr)` / `gc_pop_root()`，给 FFI 边界使用。
6. 测试用例：
   - 写一个简单`.c`实现函数；manifest 声明 extern；Lume 代码 call 调用 C 函数；JIT 运行拿到返回值。
   - Lume 函数标记`#[ffi_export]`，C 调用 Lume 函数。

## 10. 未来扩展方向（现在不做）

- 支持更多语言：zig（输出 bitcode）；
- 自动生成 C 头文件；
- 更高级 FFI：结构体布局对齐声明；
- 自举 lume‑build：用 Lume 重写整个构建工具，不再依赖 Rust 写的构建器。

## 极简示例项目文件一览

```
demo_lib/
├─ manifest.toml
├─ src/
│  ├─ main.lume
│  └─ math.c
```

`src/math.c`

```
int64_t c_matmul(int64_t n, uint8_t* buf)
{
    return n * 2;
}
```

`src/main.lume`

```
#[ffi_export]
fn calc_sum(n:int) -> int
    s:int=0
    i:int=1
    while i <= n
        s := s + i
        i := i + 1
    end
ret s

fn entry() -> int
    buf:pointer = alloc_buf(128)
ret c_matmul(128, buf)
end
```

`manifest.toml`

```
[project]
name = "demo_lib"
version = "0.1.0"
build_mode = "jit"

[[module]]
path="src/main.lume"
lang="lume"
export=["entry", "calc_sum"]

[[module]]
path="src/math.c"
lang="c"
cflags=["-O1"]
export=["c_matmul"]

[ffi.extern]
c_matmul = {params=["i64","*u8"], ret="i64"}

[ffi.export]
calc_sum = {params=["i64"], ret="i64"}
```

运行流程：

1. `lume‑build build` → 编译所有模块合并为一个 bitcode
2. lumec_core 加载合并后的 bitcode
3. `core.call("entry", &[])`，entry 里面 Lume 调用 C 写的`c_matmul`。

> 
> 至此，你的库就可以 Lume 与 C 混合开发，全部下沉 LLVM，JIT 运行或者 AOT 编译本地库。

# 修正架构：**只 lumec_core 使用 Cargo/Rust，其余全部自举，外部不搞一堆 Rust crate**

> 
> 核心思想：
> 
> 
> - `/home/myth/LumeC` 工作区
> - **只有 `lumec_core` 这一处 Rust 代码，用 Cargo 编译出唯一二进制：`lume-bootstrap`**
> - `lume‑bootstrap` 是**极小的一次性引导程序**：仅负责加载并运行用 Lume 语言写的全部上层工具（构建器、parser、编译器）
> - ✅ 不存在 `lumec_bootstrap`、`lumec_build` 这些额外 Rust crate；**parser、manifest 解析、bitcode 合并全部用 `.lume` 自举代码实现**
> - ✅ 除 core 之外，所有逻辑（语法解析、构建工具、标准库）都是 Lume 源码，不靠 Cargo 管理。
> - Cargo 仅仅用来编译 `lumec_core`，产出 `lume-bootstrap`，做完这件事 Cargo 的使命就结束。

```
/home/myth/LumeC
├── Cargo.toml                 # 仅用于构建 lumec_core，workspace只有一个成员
├── .gitignore
├── README.md
├── crates/
│   └── lumec_core/            # 🦀 【唯一Rust代码】底层底座 IR/JIT/GC/bitcode加载
│       ├── Cargo.toml
│       └── src/
│           ├── builtin.rs
│           ├── dispatch.rs
│           ├── gc.rs
│           ├── ir.rs
│           ├── jit.rs
│           ├── lib.rs
│           └── main.rs        # main → 编译输出 ./lume‑bootstrap 二进制
│
├── lib/                       # 🟢【全部Lume自举代码，*.lume，完全不用Cargo】
│   ├── bootstrap.lume          # 入口：lume标准世界的引导入口
│   ├── syntax/                # Lume实现：lexer、parser、AST、lower→IR
│   │   ├── lexer.lume
│   │   ├── parser.lume
│   │   ├── ast.lume
│   │   └── lower.lume
│   ├── build/                 # Lume实现构建工具（原来lumec_build全部搬到这里）
│   │   ├── manifest.lume      # 解析 manifest.toml
│   │   ├── cc.lume            # 调用clang，生成bc
│   │   └── link_bc.lume       # 调用llvm‑link，合并bitcode
│   ├── std/                   # Lume标准库 array / string / math / io
│   │   ├── array.lume
│   │   ├── string.lume
│   │   └── math.lume
│   └── builtins.lume          # Lume侧内置包装，FFI根管理等
│
├── examples/                  # 🟢 示例项目（用户项目，*.lume / .c / manifest.toml）
│   ├── demo_basic/
│   │   ├── manifest.toml
│   │   └── src/main.lume
│   └── demo_c_interop/
│       ├── manifest.toml
│       └── src/
│           ├── main.lume
│           └── calc.c
│
└── docs/
    └── lume_spec.md           # 语言设计规范文档
```

## 文件角色详解

### 1. `crates/lumec_core`（🦀 Rust + Cargo，仅此一处）

- 包含：IR 定义、IR 验证、LLVM‑JIT、GC 堆、Tagged‑Value、`LumeCore` runtime
- **新增 Rust 侧最小 builtin 原语集合**，只给自举世界用：
  1. 文件 IO：读文件、写文件
  2. 执行外部命令（调用 clang /llvm‑link，给 build.lume 使用）
  3. bitcode 加载接口：`load_bitcode()`
  4. GC 根压入弹出
  5. 把一段 Lume IR 数据喂进 JIT 的 builtin

> 
> ⚠️ Rust 层**完全没有 Lume 语法解析、没有 parser、没有 manifest 解析**。

- `lumec_core/src/main.rs` 编译输出二进制：`lume‑bootstrap`。

**根目录 `/home/myth/LumeC/Cargo.toml`**

```
[workspace]
members = ["crates/lumec_core"]
resolver = "2"
```

`crates/lumec_core/Cargo.toml`

```
[package]
name = "lumec_core"
version = "0.1.0"
edition = "2021"

[dependencies]
inkwell = { version = "0.4.0", features = ["llvm18‑0"] }
self_cell = "1.0.2"

[[bin]]
name = "lume‑bootstrap"
path = "src/main.rs"
```

### 2. `crates/lumec_core/src/main.rs`（lume‑bootstrap 二进制）

> 
> **Rust 这边 main 逻辑极其简短，只做一件事：**
> 
> 
> 1. 初始化 `LumeCore`
> 2. 读取磁盘上 `lib/bootstrap.lume`（Lume 写的引导程序源码文本）
> 3. ⚠️ 关键点：Rust 这边**没有 parser**，这里是鸡生蛋缺口：
>    - 阶段 0（最开始开发）：Rust 里面**硬编码手写一小段最小 IR**，这个 IR 的功能：调用 Rust 内置 IO builtin，把`lib/syntax/*.lume`源码读取进来。
>    - 这小段手写 IR 是唯一的火柴；火柴点燃之后，剩下全部逻辑跑在 Lume 上。

伪代码示意 main.rs

```
// lumec_core/src/main.rs
fn main()->Result<(),Box<dyn Error>>{
    let mut core = LumeCore::new();

    // ===== 【唯一火柴：手写最小IR，仅此一点点硬编码】=====
    // 这个手写IR的功能：调用rust‑builtin读取lib/bootstrap.lume字符串
    let bootstrap_ir = hand_written_minimal_ir();
    core.define_function(bootstrap_ir)?;

    // 运行bootstrap入口，从此进入Lume自举世界
    core.call("bootstrap", &[])?;
    Ok(())
}
```

> 
> 这一小段手写 IR 是整个系统唯一 “火种”；**火种之后：lexer、parser、build 工具全部是 Lume 代码**。
> 没有 Rust 写的 lexer/parser，没有额外 Rust crate。

### 3. `lib/bootstrap.lume` （🟢 Lume，自举世界入口）

> 
> 火种 IR 跑起来之后，就执行这个 lume 代码：

1. 读取 `lib/syntax/lexer.lume`、`parser.lume`
2. 调用 Lume 写的 lexer+parser，把这些源码编译成 IR
3. 注册进 JIT runtime
4. 之后就拥有完整 Lume 编译器能力，可以编译任意`.lume`文件
5. 再加载 `lib/build/build.lume`：得到完整项目构建工具

> 
> 从此以后：
> 解析 manifest.toml、调用 clang 编译 C 文件、调用 llvm‑link 合并 bitcode，**全部是 Lume 代码在运行**。

### 4. examples 用户示例

用户写项目，只需要写 `manifest.toml` + `.lume` + `.c`，**不需要碰任何 Cargo**。

### 运行流程（完整引导链）

```
# 第一步：开发者仅一次，用cargo编译底层底座
cd /home/myth/LumeC
cargo build

# 产出唯一二进制
./target/debug/lume‑bootstrap

# 执行流程：
# 1. Rust: LumeCore初始化，运行【火种手写IR】
# 2. 火种IR调用rust‑builtin IO，读取 lib/bootstrap.lume 文本
# 3. 执行Lume写的lexer+parser，编译整个lib/*下面全部lume库
# 4. 现在完整Lume编译器、build工具就绪
# 5. 可以处理用户项目：
#     lume‑bootstrap run examples/demo_c_interop/manifest.toml
#        → lume‑build（lume实现）解析manifest
#        → 调用clang编译 .c → bc
#        → Lume自身编译器编译 *.lume → IR → bc
#        → llvm‑link 合并bitcode
#        → lumec_core JIT运行
```

## 关键约束总结

1. 🦀 **Rust/Cargo 只存在于 `crates/lumec_core`**，只编译出 `lume‑bootstrap`。
2. ✅ `lib/`、`examples/` 全部 Lume 源码，**完全不经过 Cargo**。
3. ✅ lexer、parser、AST lowering、manifest 解析、bitcode 构建工具，全部是 Lume 语言写，跑在 JIT 上。
4. ✅ Rust 层只暴露最原始 builtin 原语：IO、spawn 外部进程、load_bitcode、GC 根操作；**不实现任何 Lume 语法逻辑**。
5. ✅ 那一小段手写 IR 是唯一 “火种”，不需要 Rust 写完整 parser。
6. ❌ 没有 `lumec_bootstrap`、`lumec_build` Rust crate，全部自举。

> 
> 这个就是严格意义的自举：Rust 只做极小 VM 底座；编译器、构建工具都是宿主语言本身写出来的。

## .gitignore

```
target/
**/*.bc
**/*.ll
*.o
*.so
*.out
```