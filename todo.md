# Lume TODO

> 最后更新：2026-10-17
> 状态：核心编译器可用，main.lume 30 项验证在 tcc 下全绿（[5] 除外：tcc 的 InterlockedIncrement 已知缺陷，WSL 正常），三平台 CI 通过
> 已合并原「未来优化方向.md」，合并后原文件可删

---

## 设计决策（已定，别改）

| 项目 | 决策 |
|---|---|
| 语言名 | Lume |
| 编译器名 | Lume |
| 文件后缀 | `.lume` |
| 变量声明 | `:=` 声明，`=` 赋值，不混用 |
| 逻辑运算 | `and` / `or` / `not`，不用 `&&` / `\|\|` / `!` |
| match | `=>`，不是 `->` |
| 函数返回 | `->` |
| 结构体构造 | 圆括号 `Point(x: 1.0)`，不是花括号 |
| 方法字段访问 | 裸字段名，冲突时才 `self.x` |
| 泛型调用 | 不写 `[T]`，类型自动推断 |
| 模块系统 | 文件名即模块名，自动扫描，零 `import` |
| std | 默认自动导入，`@no_std` 关闭 |
| 任务并行 | `par f(x)` + `await t`（A 方案，AI 友好） |
| 数据并行 | `par for`，自动 reduction |
| 数组语义 | **引用语义**（当前实现，`b := a` 共享内存）。`restrict` 已撤回；真正的值语义/所有权待做 |
| 线程安全标记 | 函数级 `@thread_safe` / `@not_thread_safe`，默认自动推断 |
| 循环依赖 | 禁止，编译期报错 |

---

## ✅ 已完成

### 编译器核心
- [x] Lexer（全部 token + 行列号）
- [x] Parser（递归下降 + Pratt 表达式）
- [x] 类型检查器（错误带 `help` 提示）
- [x] Codegen（生成 C）
- [x] CLI（`lume build` / `lume run`）
- [x] 模块拆分（`parser/`、`checker/`、`codegen/` 子目录）

### 语言特性
- [x] 字符串插值（AST 化，走真正的 parser）
- [x] 结构体 + 方法（指针传 self）
- [x] 数组 + `for x in list`
- [x] 泛型（单态化，多参数）
- [x] `match`（字面量、通配、变量绑定、结构体解构）
- [x] 管道 `|>`
- [x] `if` / `else` / `else if` / `if` 表达式（三元）
- [x] `while` / `for` / `break` / `continue`
- [x] 前向声明（互相递归、调用后面定义的函数）

### 性能特性（已有）
- [x] `@precompute` 编译期查表（有 libomp 并行填表，没有就串行；文件缓存）
- [x] `@cache` 运行时缓存（开放寻址哈希表，O(1)）
- [x] `par for` 自动并行（切分 + reduction + 串行阈值）
- [x] 泛型单态化（每类型一份代码，零开销）
- [x] 跨平台 runtime（Windows / Linux / macOS）
- [x] 默认 `-O3`、`--native`、`--lto`、`-fno-math-errno`
- [x] `LUME_RESTRICT`（数组无别名）
- [x] `LUME_PURE`（纯函数 `__attribute__((const))` + `purity.rs` 证明）
- [x] `#pragma clang loop vectorize` 提示（`for` / `par for` / `while`）
- [x] `par for` worker 提到文件顶层（跨平台可编译）
- [x] `lume_cpu_count()` 缓存

### 工具链
- [x] GitHub Actions 三平台 CI
- [x] 30 项验证测试（`main.lume`，tcc 实测全绿）
- [x] README.md + LUME.md
- [x] 标准库内置（`str_*` / `int_*` / `float_*` / `say` / `now`）

### 已修 bug
- [x] `par for` 闭区间重叠
- [x] `else {}` 块丢失
- [x] `str_*` 共享 buffer（8 槽位轮转）
- [x] `self.x` 生成非法 C
- [x] `malloc` 类型转换少 `*`
- [x] worker 循环没向量化（`_p->end` 拷局部）
- [x] `infer_type` 不知道函数返回类型
- [x] worker 引用局部变量 → 编译期报错
- [x] 嵌套 `par for` → 编译期报错
- [x] `f(self)` 传值（`(*_self)`）
- [x] `-=` / `*=` 归约（**已撤回，别顺手改**）
- [x] 默认参数不生效（`FuncSig.required/defaults` + 调用处展开）
- [x] 命名参数不支持（`StructLit` fallback + `reorder_named_args`）
- [x] 整数除零 UB（编译期字面量 0 报错 + 运行期 `lume_checked_div_i64`）
- [x] `INT64_MIN / -1` 溢出 UB（`lume_checked_div_i64` 内置检查）

---

## 🔥 已知 bug（待修）

- [x] `if` 表达式未实现（`x := if c { 1 } else { 2 }` 生成非法 C）—— 已修：单表达式分支走 C99 三元，else if 链嵌套
- [x] `malloc` 结果类型在部分 codegen 路径仍可能出错 —— 已修：最后一处是数组返回函数脱指针层脱了两层（str_split），改 strip_suffix 只脱一层
- [x] `match` 结构体解构恒为真 —— 已修：struct 编译期 `_tag` 字段 + C union 类型双关
- [x] `match` 值被强制转 `int64_t`（只能 match 整数/布尔）—— 已修：`_match_val` 按 infer_type 声明
- [x] 默认参数不生效 —— 已修：`FuncSig` 加 `required`/`defaults` 字段，checker 校验「默认值必须在尾部」+ 默认值类型，codegen 调用处展开默认值表达式（C 语义：只能引用调用处可见名字）
- [x] 命名参数不支持 —— 已修：`f(a: 1)` 与 `Point(x: 1)` 语法同形，parser 一律解析成 `StructLit`；checker/codegen 按名字区分（在 `structs` 表 → 结构体构造，否则 → 命名参数函数调用），共享 `ast::reorder_named_args` 重排
- [x] 整数除零是 UB（需加检查）—— 已修：编译期字面量 `0` 直接 checker 报错；运行期变量走 `lume_checked_div_i64`（`fprintf stderr + abort`）
- [x] `INT64_MIN / -1` 溢出是 UB —— 已修：`lume_checked_div_i64` 内置 `a == INT64_MIN && b == -1` 检查，abort；`lume_checked_mod_i64` 同条件返回 0（C99 下 UB，数学余数 0）
- [x] 数组不能跨函数传 —— 已修：隐式伴随长度参数 + 数组返回 malloc 拷贝
- [x] `nums.len()` 对数组参数返回不存在的 `nums_len` —— 已修：数组参数登记 array_len_exprs
- [x] 方法体里的 `map` / `filter` / `reduce` 生成非法 C —— 已修：`gen_stmt_with_self` 接上
  `gen_hof_let`，`gen_expr_with_self` 补 `reduce`；测试 `main.lume` [33]
- [x] **`a[i]` 越界静默读写内存** —— 已修：长度能编译期确定时插 `lume_idx_check`
  （`fprintf + abort`），长度和下标都是常量时直接报编译错；`@unchecked` / `--unchecked`
  可关。struct 字段数组和函数返回数组的长度还拿不到，仍不检查。测试 `tests_bounds.lume` +
  `tests/fail/index_oob_*.lume`
- [x] **测试没有断言** —— 已修：`scripts/run_tests.sh` + `tests/golden/*.out` 逐字节比对
  （只归一化耗时和 worker 顺序），`tests/fail/` 放必须报错的用例
- [x] **CI 不跑测试** —— 已修：`build.yml` 加 `test` job，`build` 依赖它
- [ ] 返回数组的调用不能出现在实参/表达式位置（`total(make3())`）—— 需要语句级临时提升
- [ ] `map`/`filter` 的源不能是 struct 字段数组
- [ ] 空 struct 构造 `E()` 被当成函数调用
- [ ] checker 的错误没有行号列号（`CheckError` 只有 msg/hint）—— 要大改，先记着
- [ ] codegen 的「暂不支持」以 `/* 注释 */` 降级，靠 clang 兜底报错且指向 .c —— 应改成
  `codegen_errors` 硬报错（`free` 误用已经是这么做的）
- [x] 浮点 `x * 2^n` 被强度削弱成整数移位（`2.5 * 8` 得 16）—— 已修：AST 没有类型信息，
  `optimize.rs` 不再对 `Expr::Ident` 做乘法强度削弱，交给知道类型的 clang
- [x] `LUME_OPT=safe` 无效 —— 已修：`cli.rs` 以前先无条件跑 Aggressive 再按 env 跑第二遍，
  现在只跑一遍
- [x] `@cache` 被 `par for` 调用时生成非法 C（worker 在原型之前）+ 无锁数据竞争 ——
  已修：args 结构体/worker 声明提前、worker 定义后置；被 par 调用到的 @cache 自动换带锁包装器
- [x] 空循环体/无输出程序也起日志 writer 线程 —— 已修：主函数占位符，生成完再看有没有
  真正的 `lume_logf`/`lume_log_write_literal`/`lume_log_flush`
- [x] 每个 `par for` 都全量 `lume_log_flush()` 等 I/O —— 已修：改成只提交（`lume_log_flush_thread`）
- [x] 强制 `vectorize(enable)` 发给含 `say` 的循环 → 每次构建两条 `-Wpass-failed` ——
  已修：先扫循环体，含未证明纯度的调用就不发 pragma
- [x] **数组返回值没有所有权/`free`** —— 已加显式所有权：`free(x)` 只接受编译器
  跟踪为 owned 的变量（函数返回的 `[..]`/局部数组、`map`/`filter`/管道结果）；
  字面量、参数、切片、返回入参别名的函数编译期拒绝。实测 20k × 1000 元素从
  155 MB 降到 15.8 MB。**自动释放/值语义仍是 TODO**
- [ ] **数组没有值语义** —— 文档过去写「值语义、无别名」，实际是引用；已改文档，
  真正的复制语义/所有权是 v0.3 的设计任务

---

## 📊 性能优化全景表

### 一、clang 不敢做（Lume 的独有优势）

- [ ] **#1 数组参数加 `restrict`** — **已撤回（2026-10-24）**：数组实际会别名（`shift(x, x)` 曾经算错），`restrict` 是 UB。要恢复必须先做值拷贝或全程序别名分析。
- [x] **#2 纯函数加 `__attribute__((const))`** — `purity.rs` 保守证明 → 消除重复调用
- [x] **#3 函数级 `@fastmath`** — 只对标注函数启用重结合、FMA contract、倒数优化；不放宽全局 NaN/Inf 语义
- [x] **#4 边界检查消除** — 编译期知道数组长度 → 快 10~30%
- [x] **#5 显式 `par for` 自动并行** — 常驻线程池 + 自动 reduction，8 核 400k 元素 2.4 倍
- [x] **#6 管道融合成单循环** — `xs |> map(f) |> map(g) |> filter(p)` 编译成**单个** for 循环，中间值全在栈上单个标量 `_pc` 里。内存 3N → 1 份输出，遍历 3 趟 → 1 趟。实现见 `src/codegen/pipe.rs`（`parse_chain` 拆链 + `type_plan` 校验签名 + `gen_fused_pipe_let` 发循环）。新增 `map`/`filter`/`reduce` 三个数组高阶内置：前两个产出两个值只能在 `let` 位置，`reduce` 返回标量可内联在表达式位置。测试 `tests_pipe.lume`（11 项）。
- [x] **#7 编译期执行纯函数** — `@precompute` 执行整个函数 → 运行时零开销
- [x] **#8 自动 reduction** — `par for` 里识别 `sum +=` → 并行求和 5~8 倍
- [ ] **#9 激进循环不变式外提** — `@fastmath` 下可以 → 减少重复计算
- [ ] **#10 跨函数常量传播** — 有整个 AST → 消除冗余调用
- [ ] **#11 浮点代数简化** — `@fastmath` 下 `x+0`/`x*1` 可简化 → 消除冗余
- [ ] **#12 FMA 融合** — `@fastmath` 下无条件做 → 一条指令，无中间舍入
- [ ] **#13 多维数组步长已知** — 编译期知道行主序、步长 → 缓存友好重排
- [ ] **#14 切片无别名** — 编译期知道不重叠 → 子数组向量化
- [x] **#15 `@cache` 哈希表** — 生成的缓存可优化 → O(n²) → O(n)

**已完成 8 / 15。**

### 二、clang 会做，但 Lume 可以更早/更激进

- [x] **#16 强度削弱 `x*2^n → x<<n`** — codegen 里无条件做
- [ ] **#17 取模 `x%2^n → x&(2^n-1)`** — 同上
- [ ] **#18 魔数除法** — codegen 里主动做，更早 → 除法快 10~40 倍
- [ ] **#19 循环感应变量削强度** — `a[i*stride]` → 指针递增（中）
- [ ] **#20 有符号除法修正** — 自己做时要注意舍入（小）
- [ ] **#21 常量折叠** — 更早，AST 层面（小）
- [ ] **#22 位运算优化 `x&0 → 0`** — 更早（小）
- [ ] **#23 浮点除法外提（LICM）** — `@fastmath` 下做
- [ ] **#24 乘常数移位化 `x*3 → x+(x<<1)`** — 嵌入式目标才值得（小）

### 三、白嫖 clang（不用自己做）

- [x] **#32 自动向量化（SIMD）** — clang 自动用 AVX/SSE
- [x] **#33 循环展开** — clang 自动
- [x] **#34 指令调度** — clang 自动
- [x] **#35 寄存器分配** — clang 自动
- [x] **#36 常量传播** — clang 自动
- [x] **#37 死代码消除** — clang 自动
- [x] **#38 内联** — clang 自动
- [x] **#39 尾调用优化** — clang 自动
- [x] **#40 链接时优化（LTO）** — `--lto` 加 `-flto`
- [x] **#53 循环向量化提示** — `#pragma clang loop vectorize(enable) interleave(enable)`
- [x] **#54 默认 `-O3` + `--native` + `--lto`** — 白嫖 clang 展开/向量化/AVX2
- [x] **#55 `-fno-math-errno`** — 让 `sqrt`/`pow` 真正无副作用，`LUME_PURE` 才站得住

### 四、研究级（暂不需要）

- [ ] **#41 矩阵乘法结合律重排** — `(A*B)*C` vs `A*(B*C)`（大）
- [ ] **#42 超越函数超优化** — Herbie 风格，找更精确的等价形式（极大）
- [ ] **#43 跨过程强度削减** — 整个调用图看常量除法（大）
- [ ] **#44 值敏感分析** — CAD 分析循环变量值（极大）
- [ ] **#45 循环分块 + 软件预取** — cache 友好（大）
- [ ] **#46 自动微分** — 数值计算（极大）
- [ ] **#47 Polyhedral 优化** — 循环变换的数学框架（极大）

### 五、未来换后端才需要

- [ ] **#48 自己生成 LLVM IR** — 跳过 C 前端，编译更快
- [ ] **#49 自己生成汇编** — 完全控制，最大工作量
- [ ] **#50 寄存器分配** — 如果不用 LLVM
- [ ] **#51 指令选择** — 如果不用 LLVM
- [ ] **#52 ABI 管理** — 如果不用 clang

---

## 📋 待做（按批次）

### 第一批：语言基础

- [x] **数组跨函数**（隐式长度伴随参数 + 数组返回 malloc + out-length 出参）
  - [x] `mod.rs` 加 `fn_signatures` / `array_params`
  - [x] `items.rs` 参数生成拆 `(T* data, int64_t len)`
  - [x] `expr.rs` 调用处追加长度
  - [x] `stmt.rs` `for` 支持数组参数遍历
  - [x] checker 放宽数组参数检查

- [x] **`if` 表达式**（C99 三元，else if 嵌套）

- [x] **`match` 结构体解构**（`_tag` + union，类型不对的臂 tag 兜底）

- [x] **字符串操作**（split / contains / replace / starts_with / ends_with）
  - [x] `str_split` / `str_contains` / `str_replace` / `str_starts_with` / `str_ends_with`

- [x] **整数除零检查**
  - [x] 编译期常量 0 报错（checker `Expr::Binary` 字面量 `Int(0)` 直接 error_hint）
  - [x] 运行时变量 0 检查（`lume_checked_div_i64` / `lume_checked_mod_i64`，fprintf + abort）

- [x] **`[v; N]` 非零重复初始化**
  - [x] 语句位置：生成声明 + 循环填充（跨平台，C 文件小）
  - [x] 表达式位置：展开成显式列表（小 N 才值得）
  - [x] `N` 很大时用 `memset` 优化单字节 0/1 模式

### 第二批：类型系统

- [x] **`T?` 可选值** + `?` 错误传播 —— 已完成：`Some(x)`/`None` 构造、`.has`/`.val`、`x!` 强制解包（None 则 abort）、`x?` 传播（提前 return None）；C 侧为命名 typedef `lume_opt_N {int8_t has; T val;}`（不用匿名 struct —— tcc 不认前向声明与定义的结构兼容）。tcc 实测全绿
- [x] **`T!` 错误值** —— 语义别名，并入上面的 `T?`（同一套 codegen）
- [x] **切片 `a[1..3]`**（parser 已支持，补 codegen）—— 已完成：`a[i..j]` / `a[i..=j]`（闭区间） / `a[..j]` / `a[..]` / `a[i..]`；零拷贝借用（改切片就是改原数组）；`let` 绑定、直接当实参、`for v in 切片`、嵌套切片、`.len()` 全支持；越界运行期报错 abort（`lume_slice_off`/`lume_slice_len`）。tcc 实测 15 项全绿
- [ ] **多返回值 / 元组**
- [ ] **泛型 struct**
- [x] **`i128` / `u128`** —— codegen 已输出 `__int128`/`unsigned __int128`，含一元负号/取反不截断
- [ ] **Debug 模式溢出检查**（`__builtin_add_overflow`）

### 第三批：模块系统 + 工具链

- [ ] **文件名即模块名**（路径 → 模块映射）
- [ ] **自动扫描加载**（`main.lume` 同级 + 子目录递归）
- [ ] **短名冲突检测**
- [ ] **`pub` 可见性**
- [ ] **循环依赖检测**
- [ ] **`@no_std`**
- [ ] **`@manual_load` / `@exclude`**
- [ ] **`@thread_safe` / `@not_thread_safe`**
- [ ] **CLI 统一命名为 `lumen`**
- [x] **VS Code 扩展语法高亮 + 文件图标主题**（`vscode-extension/`）
- [ ] **VS Code LSP**（诊断 / 补全 / 跳转）
- [ ] **`lumen fmt`**
- [ ] **`lumen test`**
- [ ] **`lumen doc`**

### 第四批：并发

- [ ] **`par f(x)` + `await t`**（`Task[T]` 类型 + codegen）
- [x] **异步日志与多线程 print**（线程本地缓冲 + FIFO 后台 writer；`flush()` 等待屏障，worker 退出只提交）
- [x] **`write` 内置（不换行）**
- [x] **线程池** — 常驻 N-1 worker + 主线程抢任务；按元素数只唤醒前几个；
  `LUME_PAR_THREADS` 可覆盖线程数；嵌套并行退化成串行。见 `runtime.rs` / `parallel.rs`
- [ ] **`par for` 支持嵌套控制流**（`while` / `for` / `match` 进 worker）
- [ ] **`par for` 支持 struct 字段数组**
- [ ] **`atomic_add` 内置**

### 第五批：性能优化（对应优化表）

**第二批：**

- [x] **#3 函数级 `@fastmath`**
- [x] **#6 管道融合**（数据管道核心）— 见上
- [ ] **#11 代数简化**（配合 `@fastmath`）
- [ ] **#16~18 强度削弱 / 魔数除法**（clang 已做，优先级低）

**第三批：**

- [ ] **#13 多维数组**（矩阵运算）
- [ ] **#14 切片无别名**（子数组向量化）
- [ ] **#10 跨函数常量传播**（消除冗余）
- [ ] **#12 FMA 融合**（浮点内核）
- [ ] **#9 激进 LICM**（循环不变式）

**换后端时：**

- [ ] **#16~24 强度削弱、魔数除法等**（自己控制，不依赖 clang）
- [ ] **#48~52 LLVM IR / 汇编后端**

**其他性能：**

- [x] **异步日志**（线程本地缓冲提交给后台 writer；`flush()` 显式等待，退出时排空并 join）
- [ ] **`@precompute` 支持 float 表**

### 第六批：生态（持续）

- [ ] **标准库扩展**（`std.io` / `std.collections` / `std.math`）
- [ ] **动态数组 `Vec`**（`{ data, len, cap }` + `push` / `pop`）
- [ ] **示例项目**（排序 / 矩阵乘法 / HTTP / BigInt）
- [ ] **包管理**（`lumen.toml` + 依赖下载）
- [ ] **官网 / 文档站**
- [ ] **包仓库**
- [ ] **C 库绑定**（`extern` 真正实现 + raylib / SDL2）
- [ ] **自举（用 Lume 写 Lume）**（长期目标）

---

## 🎯 BigInt（第一个用 Lume 写的程序）

### 前置补丁

- [x] **struct 支持定长数组字段**（`gen_struct_decl` 生成 `T name[N]`）
- [x] **`[0; N]` 重复初始化**（AST 加 `Expr::Repeat`）
- [x] **`write` 内置（不换行）**

### 单线程版本

- [ ] `big_from_int` / `big_zero`
- [ ] `big_cmp_abs` / `big_cmp`
- [ ] `big_add_abs` / `big_sub_abs` / `big_add` / `big_sub`
- [ ] `big_mul`
- [ ] `big_print`
- [ ] 测试：`2^100` 正确输出 `1267650600228229401496703205376`

### 并行版本（等两堵墙修完）

- [ ] `par for` 支持嵌套 `for`
- [ ] `par for` 支持 struct 字段数组
- [ ] `big_mul_par`（按输出位置 k 分块）
- [ ] 仅对 > 4096 limb 有收益

### 已知限制

- 值传递慢（2064 字节 struct 每次复制）→ 后续改成指针传
- 没有除法（长除法工作量大）
- 没有字符串解析（`big_from_str`）
- 1024 位上限（256 limb × 4 位）

---

## 🚫 明确不做（或长期不做）

- 继承（用 trait 替代，但 trait 还没做）
- GC（值语义 + 作用域释放）
- 闭包捕获（需要环境打包，工作量大）
- 异常（用 `T!` + `?` 替代）
- 宏系统（用泛型 + 装饰器替代）
- 裸指针（值语义已够）
- `no_std` 完整实现（需要重写 I/O、字符串、集合）

---

## 📝 开发流程约定

### 每加一个特性

1. 写 `main.lume` 的验证项（每行末尾印期望值）
2. 改编译器
3. 跑 `cargo run -- run main.lume`，全绿才算完成
4. 同步 `LUME.md`
5. 推送 GitHub（CI 四平台验证）

### 每修一个 bug

1. 先写能复现的测试
2. 用隔离探针定位（参考 `-=` 归约误判的调试方式）
3. 修完测试全绿
4. **不确定的不顺手改**（把「别顺手修它」写进代码注释）

### tcc / 非目标平台

**tcc 输出可以发现问题，但不能直接当作「Lume 错了」的证据，必须先隔离验证。**

### 提交前检查

- [ ] `cargo build` 零警告
- [ ] `main.lume` 全绿
- [ ] `LUME.md` 同步
- [ ] CI 四平台通过

---

## 🏁 里程碑

| 版本 | 目标 | 状态 |
|---|---|---|
| v0.1.0 | 核心编译器 + 20 项验证 | ✅ 已完成 语言暂时没有实际价值所有未发布 |
| v0.2.0 | 数组跨函数 + `if` 表达式 + `match` 解构 + 字符串操作 | ✅ 已完成 未发布 |
| v0.3.0 | `T?` / `T!` + 切片 + 元组 + 泛型 struct | 待做 |
| v0.4.0 | 模块系统 + 自动扫描 + VS Code 扩展 | 待做 |
| v0.5.0 | `par f()` + `await` + `lume_say` | 待做 |
| v1.0.0 | 包管理 + 文档站 + 示例项目 + 动态数组 | 待做 |
| v2.0.0 | 自举 | 长期 |

---

## 📎 附：两个核心优化的实现细节

### 纯函数证明（`src/codegen/purity.rs`）

`__attribute__((const))` 标错了会导致错误的 CSE（静默改结果），
比不优化严重得多，所以判据非常保守 —— **任何一点不确定就放弃**：

- 必须有返回值
- 只写自己的局部变量；写数组 / 写字段 / 写全局 → 放弃
- 只引用参数和函数内 `let` 出来的变量；碰全局 `const` → 放弃
- 只调用数学库白名单函数（`sqrt` `pow` `sin` `abs` `min` `max` …）
  或已经证明为纯的函数（不动点迭代）
- `say` / `par for` / 方法调用 / 结构体字段访问 → 放弃
- `@cache` / `@precompute` 的包装函数读写全局缓存表 → 放弃

配合 `-fno-math-errno`，`sqrt`/`pow` 不再写 errno，白名单才成立。

### `par for` reduction（`src/codegen/parallel.rs`）

识别 `v = v OP rhs`（OP ∈ `+` `-` `*`），并且要求：

- `v` 不是循环变量、循环体里没有被 `let` 重新声明
- `v` 在整个循环体里的出现次数**恰好**等于 `2 × 累加语句条数`
  —— 多一次就说明循环体还读了它，归约后语义会变，宁可不优化

满足后：每个线程一份私有累加器（加法/减法按 `+=` 收集、乘法按 `*=`），
结束后用 `+=` / `-=` / `*=` 合并回原变量。等价于 OpenMP 的 `reduction(+:v)`。

不满足时行为和以前一样（不加锁，多线程写同一变量会竞争）——
所以要写 `par for` 求和，就用 `sum += ...` 这种形式。

---

## 🔗 参考

- [Labor of Division](https://ridiculousfish.com/blog/posts/labor-of-division-episode-i.html) — 魔数除法
- [L3](https://github.com/undoio/l3) — 无锁日志，单线程 ~1ns
- [NanoLog](https://github.com/PlatformLab/NanoLog) — 编译期静态格式化，~80M logs/s
- [fastlog](https://github.com/ArashPartow/fastlog) — 线程本地缓冲异步日志
- [moodycamel::ConcurrentQueue](https://github.com/cameron314/concurrentqueue) — 工业级无锁队列
- [pthreadpool](https://github.com/Maratyszcza/pthreadpool) — 高性能线程池
- [Herbie](https://herbie.uwplse.org/) — 浮点超优化
- [egglog](https://github.com/egraphs-good/egglog) — 超优化框架