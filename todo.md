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
- [x] 动态数组 `Vec[T]`（句柄 + 元素类型只在编译器侧，见自举第 5 条）
- [x] 枚举 + `match` 穷尽性检查（自举第 6 条）
- [x] `scope` 块（arena 水位回退，自举第 2 条）
- [x] 运算符重载 `op_*`（见下文「运算符重载」）
- [x] `extern fn` FFI + `@link`（自举第 8 条，可直接调 libc）

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
- [x] **`const` 声明完全不可用** —— 已修：checker 的 `collect_items` 只登记了
  函数和结构体，从没登记过 `Item::Const`，所以任何 `x % M` 都报「未定义的变量」，
  代码生成里的 `gen_const` 形同虚设。现在 checker 登记（名字 → 类型 + 字面量值），
  codegen 建常量表；顺带把「零常量除数」提前到编译期报错。测试 `tests_divmod.lume`
  + `tests/fail/const_div_zero.lume`
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

> 2026-10-25 复查结论：这一节原本标着「clang 已做，优先级低」，真动手才发现
> **值钱的不是「把 clang 会做的事提前做一遍」，而是「给 clang 一个它拿不到的前提」**。
> 大部分子项 clang 做得比我们好，硬做等于负优化 —— 下面逐条标了判断。

- [x] **#16 强度削弱 `x/2^n → x>>n`** — 只在**可证非负**时做（`is_nonneg`）。
  有符号负数做移位和做除法语义不同（`-9/8 == -1` 而 `-9>>3 == -2`），
  clang 不敢换，Lume 证明了就敢。（AST 层的旧版本早已撤回：那里没有类型，
  会把 `2.5 * 8` 算成 16。）
- [x] **#17 取模 `x%2^n → x&(2^n-1)`** — 同上，前提是 `is_nonneg`。
- [x] **#18 魔数除法** — **改做成「常量除数安全化」**。魔数乘法本身 clang 比我们强，
  但前提是它能看见「除数非零」—— 我们生成的 `lume_checked_div_i64` 里带 `abort()`，
  这个分支一直碍事（尤其阻碍 SIMD）。现在常量除数 d≠0 直接出 `(a / d)`，
  clang 的魔数除法和向量化都解锁了。**这才是「更早」的正确用法。**
- [x] **#20 有符号除法修正** — 不需要单独实现：`is_nonneg` 成功才走移位，
  失败就保留 `lume_sdiv_pow2`（自带符号修正），两条路各自正确。
- [x] **#21 常量折叠** — 升级成「常量真的能用」：`const` 以前 checker 根本不登记，
  写 `x % M` 直接报「未定义的变量」，等于死特性。现在 checker 登记类型、
  codegen 建字面量常量表，`x % M` 当 `x % 8` 处理；连零常量除零都能在编译期
  拦下（`x / ZERO` 以前要跑到运行期才 abort）。
- [ ] **#19 循环感应变量削强度** — **不做**。clang 的 IVSR 比手写强，改了是负优化。
- [ ] **#22 位运算优化 `x&0 → 0`** — **不适用**。语言没有位运算符
  （`&` `|` `^` `<<` `>>` 都没进 lexer），这类表达式用户写不出来。
- [ ] **#23 浮点除法外提（LICM）** — **不做**。`@fastmath` 已带 `-ffast-math`，
  clang 自己的 LICM 会外提，再提一遍是重复劳动。
- [ ] **#24 乘常数移位化 `x*3 → x+(x<<1)`** — **不做**。对 clang 后端是负优化：
  它知道目标机 LEA / IMUL 的延迟，比我们拍脑袋强。

**本节该做的已做完（#16~#18、#20、#21）。剩下 4 项是「做了更慢」，已标注理由。**

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

- [x] **字符串拼接 + arena 派生**（自举咽喉，2026-10-25）
  - [x] `str + str` → `lume_str_concat`（runtime 里 arena bump 分配）
  - [x] `+=` 自动可用（parser 早就展开成 `s = s + x`）
  - [x] 只允许 `str + str`；`"n=" + 1` 编译期报错 + hint 指向插值
  - [x] 所有 `str_*` 从 8 个静态轮转槽改成 arena：**解除 8 槽限制**
  - [x] 去掉 4095 字节截断：**解除长度上限**
  - [x] 新函数：`str_slice` `str_index_of` `str_repeat` `int_to_str` `float_to_str` `str_arena_reset`
  - [x] 非 `say` 位置的插值现在真拼（以前 `s := "a{x}"` 静默得到 `"a"`）
  - [x] 内置函数表收敛到 `src/builtins.rs`（原来 checker/codegen/say 各抄一份，漏一处就生成不存在的 C 调用）
  - [x] 修 `say("{str_contains(..)}")` 的 varargs UB（bool 用 `%lld` 读 8 字节）
  - [x] 测试：`tests_string.lume` 14 组 + `fail/str_concat_mismatch` + `fail/internal_builtin`

  **arena 已知限制**（下一步要解决）：
  - 内存只增不减，只有 `str_arena_reset()` 整体回收 —— 循环里反复拼接会持续占用
  - 没有 `mark` / `release` 的嵌套作用域，只能整体 reset
  - `reset` 之后所有 `str` 变悬垂指针，**编译期无检查**
  - 解法是语言级的 `scope` / `frame` 块，属 v0.4+ 设计任务

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

- [x] **文件名即模块名**（2026-10-25）—— 没有路径到名字的映射，因为根本没有
  import：入口文件所在目录（含子目录）里的 `.lume` 全部自动并入
- [x] **自动扫描加载**（入口同级 + 子目录递归）—— `cli.rs` 的 `collect_lume_files`
- [x] **短名冲突检测** —— 顶层 `fn` / `struct` / `const` 同名直接报错并指出两个文件
- [ ] **`pub` 可见性** —— 现在合并后所有顶层符号互相可见，模块边界靠命名约定
- [x] **循环依赖检测** —— **不适用**：没有 import 就没有依赖图，不可能成环
- [ ] **`@no_std`**
- [ ] **`@manual_load` / `@exclude`** —— 自动扫描的逃生口，需要能手动排除文件
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
- [x] **#16~18 强度削弱 / 常量除数安全化**（2026-10-25 完成，见上面第二节）

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

### 自举（用 Lume 写 Lume）—— 按依赖顺序

目标：`lume.lume` 能编译自己。这些缺口**互相咬**，顺序不能乱。

| # | 缺口 | 状态 |
|---|---|---|
| 1 | 字符串拼接 + arena 派生 | ✅ 已完成（2026-10-25） |
| 2 | 字符串生命周期管理（`scope` 块） | ✅ 已完成（2026-10-25） |
| 3 | 文件读写 | ✅ 已完成（2026-10-25） |
| 4 | 命令行参数 + `exit` | ✅ 已完成（2026-10-25） |
| 5 | 动态数组 `Vec`（token 流长度不可预知） | ✅ 已完成（2026-10-25） |
| 6 | 枚举（`TokenKind` 几十种，现在只能用 int 常量硬凑） | ✅ 已完成（2026-10-25） |
| 7 | map / 集合（符号表要哈希表） | ⬜ |
| 8 | `extern` FFI（真正去调 clang） | ✅ 已完成（2026-10-25） |

**为什么第 2 条排在 IO 前面**：字符串现在没有所有权语义，
`read_file` 返回的指针可能在后续 `str_trim` 之后失效。先做 IO 必然返工。

**库包与依赖**（2026-10-25，为自举铺路）：真正开写编译器时要把
lexer / parser / checker / codegen 拆成几个库，而这些库互相依赖。原来的
「LUME_PATH 下所有 .lume 无条件合并」撑不住：不用的库也参与编译、两个库
同名就炸、跨库根的依赖没法表达。现在改成：

- 库根下的**一级子目录 = 一个包**（包名 = 目录名），根下散放的 `.lume`
  也算一个包（包名 = 文件名）。包内文件递归收，带 `fn main` 的当示例跳过。
- 包内可写 `import 另一个包名` 声明依赖，整个包共享，不要求同一个库根。
- **按符号需求自动加载**：项目引用到的顶层名字 → 唯一提供它的包 → 该包的
  引用 → … 迭代到不动点。项目里仍然零 import。
- 没用到的库完全不参与构建，所以两个库都有 `parse()` 也不会冲突；真用到
  两个才报歧义，用 `import` 指定。

查验入口：`lume libs main.lume`（依赖图 + 谁会被加载）、
`LUME_LIB_TRACE=1`（只打加载列表）。
验收：`tests/libdeps`（三层：项目 → calc → sutra，含跨包 struct 字段和
方法体跨库调用；legacy 包故意撞名以证明未用库不参与）、
`tests/fail_lib/ambiguous`、`tests/fail_lib/missing_dep`。

`scope` 的落地形态（2026-10-25）：记 arena 水位 → 出块回退。
`break` / `continue` 自动补回收，`return` 故意不回收（返回值可能就是块内
派生的字符串），`par for` 里禁止使用（多 worker 共享 arena）。
验收入口：`tests_scope.lume`（17 项，含 `str_arena_bytes()` 的字节数断言）。

第 3、4 条的落地形态（2026-10-25）：

- `read_file(path) -> str?`，失败 `None`，内容进 arena（不用 free）。
  空文件是合法内容 → 所以不能用 `""` 表示失败。
  只读普通文件（`fseek`/`ftell` 取值），管道/终端返回 `None`。
- `write_file` / `append_file` / `remove_file` 返回 `bool`；
  `file_exists` 只认普通文件（`S_ISREG`），目录返回 `false`。
- `str?` 的 C 类型是**固定名字** `LumeOptStr`（`typedef struct { int8_t has;
  const char* val; }`），不走 `lume_opt_N` 惰性编号 —— runtime 里的
  `lume_file_read()` 必须能直接返回它，而那时编译器还没分配过 tag。
  `opt_tag("const char*")` 永远返回这个名字，所以和 `Some("x")` / `None`
  / `!` / `?` / struct 字段完全互通。
- `arg_count()` / `arg(i)`：main 签名统一成
  `int main(int _lume_argc, char** _lume_argv)`，开头一行 `lume_set_args(...)`。
  用户给 `main` 写了参数时按用户的来（那种写法拿不到 C 的 argv）。
- `exit(n)` 先 `lume_log_flush()` 再退 —— 异步日志不排空会丢最后几行。
- `eprint` / `ewrite`：同一套格式串逻辑，出口换 stderr 并同步写；
  写之前先 flush stdout 队列，保证和 `say` 的相对顺序正确。
- checker 新增两条拒绝：可空值不能直接插值、不能直接进 `say`/`write`/
  `eprint`/`ewrite`（`{ has, val }` 结构体进 varargs 是 UB）。
- 顺手修掉一个老 bug：**`Expr::Str` 生成 C 字面量时只转义了 `\` 和 `"`**，
  源码里的 `"a\nb"` 到 codegen 已是真实换行字节，原样写进 C 会把一行切成
  两行（clang 报 missing terminating `"`）。现在统一走 `c_str_lit`。
- 验收入口：`tests_file.lume`（22 项，含 2 MB 跨 chunk 读取、
  `par for` 里读文件、argv、stderr 顺序、`exit(0)` 不执行后续语句）

第 5 条的落地形态（2026-10-25）：

- 类型拼写 `Vec[T]`，语法上不加新分支 —— 后缀方括号已经产出
  `Type::Generic(Named("Vec"), [T])`，`ast::vec_elem_type` 是唯一识别入口。
- **C 侧只有一个** `LumeVec { char* data; int64_t len; int64_t cap; int64_t esz; }`，
  元素类型不进 C 类型：push 时 `memcpy(esz)`，取元素时调用点按元素类型
  强转 —— 和 `qsort` 同一个套路，代码量不随元素类型膨胀。
  所以 `Vec[T]` 的 C 类型恒为 `LumeVec*`，元素类型只能记在编译器一侧：
  `vec_elems`（变量）/ `vec_field_elems`（结构体字段）/ `fn_vec_elems`
  / `method_vec_elems`。这四张表由 `inferred_vec_elem` 统一查询。
- 方法不进 `builtins::TABLE`：push 要按元素类型取地址、pop 要按元素类型
  强转，全依赖接收者类型，只能分派。入口是 `gen_vec_method` /
  `gen_vec_index`，`gen_expr` / `gen_expr_with_self` / `par` 三处共用
  （上次字符串拼接只改两处、漏了 par）。
- `push` 收元素**地址**，实参常常是字面量（没地址）→ 先落进临时变量。
  临时变量在表达式生成期间 `push_line`，输出是线性拼接的，所以天然排在
  当前语句之前（`v.push(f(x))` 也能用）。
- 底层 buffer 是 `malloc/realloc`，**不进 arena**（arena 只增不减，
  反复 push 长数组会吃满）。需要回收时 `v.free()`。
- `par for` 里只能读：`push/pop/clear/reserve/free` 在 checker 直接拒
  （多 worker 并发 realloc 是数据竞争）。读 `len()` / `v[i]` 支持 ——
  worker 的 args 结构体里存 `LumeVec*`（Vec 自带 len，没有 `_len` 伴生字段）。
- 顺手修两个老 bug：
  ① `collect_arrays_in_expr` / `collect_idents_in_expr` **都不处理 `FmtStr`**，
  于是 `par for { say("{a.len()}") }` 不会把 a 带进 worker，生成未声明符号；
  ② `gen_for` 生成循环体时丢了方法字段表，循环体里裸写字段名会生成裸 C 标识符
  （现在从 `cur_field_names` 兜回来）。
- 验收：`tests_vec.lume`（13 组，含 1000 次 push 跨多轮扩容、嵌套
  `Vec[Vec[int]]`、结构体里的 Vec 字段 + 方法、函数收/返 Vec、别名语义、
  `for x in v`）；负向 `fail/vec_par_push`、`fail/vec_push_type`、
  `fail/vec_bad_method`。
  + `tests/fail/exit_code.lume`（非零退出码且日志不丢）
  + `tests/fail/optional_interp.lume` / `optional_print.lume`。

**第 5~8 条的顺序有弹性**，但都必须在「写第一行编译器代码」之前就位 ——
否则写一半发现 token 存不下，得回头重构整个前端。

第 6 条的落地形态（2026-10-25）：

- 语法 `enum Name { A  B = 10  C }`：变体之间换行或逗号都行，显式值之后
  没写的继续 +1。**无载荷**（`A(int)` 那种 tagged union 没做）。
- C 侧**不用 `enum`**：C enum 底层是 32 位 `int`，塞进 `printf` 的 varargs
  （按 `%lld` 读）是 UB，赋值/返回也到处要强转。生成的是
  `typedef int64_t Tag;` + 每个变体一条 `#define Tag_A ((int64_t)0LL)`
  + `static inline const char* lume_enum_name_Tag(Tag)`（`Tag.name(v)` 用）。
  `#define` 而不是 `static const`：`case` 标签要求整数常量表达式。
- `Tag.A` 走 `Expr::Field(Ident("Tag"), "A")`。三处生成器（`gen_expr` /
  `gen_expr_with_self` / `par`->兜底）都要**在查变量表之前**认出
  「左边是类型名而不是变量」，否则会得到 `Tag.A` 这种 C 里不存在的东西。
  `infer_type` 的 Field / Call 分支同理（`Tag.name(t)` 必须是 `const char*`，
  不然 `"x" + Tag.name(t)` 会生成指针加法）。
- **match 穷尽性检查**（checker `check_enum_match`）：值静态类型是枚举且
  没有 `_` 兜底时，漏变体 = 编译期报错；另外查「变体名不存在」、
  「写的是别的枚举的变体」、「同一个变体匹配两次」。
- 变体值重复在 checker 拒（C 的 `switch` 会 duplicate case）。
- 验收：`tests_enum.lume`（13 组，含显式值续号、`Tag.name()`、结构体字段、
  函数收/返、`Vec[Tag]`、方法体里改字段、循环里 match、`par for` 里读枚举）；
  负向 `fail/enum_match_missing`、`fail/enum_bad_variant`、
  `fail/enum_dup_value`、`fail/enum_bad_method`。

第 8 条的落地形态（2026-10-25）：

- 语法 `extern fn name(params) -> T`，无函数体；参数表末尾可以写 `...` 变参。
  只支持 `fn`，**不支持 extern 全局变量**。
- **C 标识符加 `lume_ext_` 前缀，符号名用 `__asm__("原名")` 指回**：
  ```c
  extern int32_t lume_ext_strcmp(const char* a, const char* b) __asm__("strcmp");
  ```
  直接用原名会和 `<stdio.h>` / `<string.h>` 的声明撞类型
  （它们的 `int` 对 Lume 的 `int64_t` 是 conflicting types），加前缀后
  两个标识符互不相识，链接期仍解析到真符号。调用点靠
  `Codegen::c_fn_name` 换名（`gen_expr` / `par` 的 Ident 分支各一处）。
- 宽度靠用户对齐：C 的 `int` 写 `i32`，`long`/`size_t`/指针写 `int`。
  写错不报错 —— 真实返回 `int` 的函数若声明成 `int`，负数会变成零扩展的
  大正数（返回值只写 eax）。已在 LUME.md 里用表格说明。
- 数组参数**不补**伴随长度参数（`fn_array_params` 对 extern 全 false）——
  C 侧签名里没有那个东西。
- checker 三条新约束：extern 不能有函数体、不能是泛型；`...` 只能用在
  extern 上。这三条**故意放在 checker 而不是 parser**：解析期报错会让整个
  文件被判「解析失败」，同目录其它文件全被带着打警告（tests/fail 里加一个
  坏文件就把 13 个 golden 全改了）。
- 变参调用只检查 `...` 之前那些参数，多传的只求值。
- `@link("m")` 装饰器 → clang 额外加 `-lm`（`Codegen::link_libs` →
  `compile_c` 的参数）。
- 不做回调：没有函数指针类型，`qsort` / `signal` / `atexit` 接不进来。
- 验收：`tests_ffi.lume`（13 项：strlen/strcmp/atoi/atof/labs/getpid/
  sqrt(@link)/printf 变参/方法里调 extern/结果参与运算与条件）；
  负向 `fail/extern_body`、`fail/extern_variadic_plain`、`fail/extern_few_args`。

### 库机制（2026-10-25，为「自己写库」铺路）

自举要能把编译器拆成多个库文件；一开始只有「同目录自动合并」，库无法复用。

- `$LUME_PATH`（`:` / `;` 分隔）+ 默认 `~/.lume/lib`（存在才算）作为
  **共享库目录**。里面的 `.lume` 一样零 import 直接可用。
- 库目录里**带 `fn main` 的文件跳过** —— 库目录放示例程序是常态，不能因为
  一个示例就把整个目录判成独立项目连带排掉别的库。
- canonicalize 去重：同一个文件从「项目扫描」和「库路径」两条路走进来只算一次。
- 验收入口：`tests/libpath/`（入口 `main.lume` + 藏在 `.lib/` 隐藏目录里的
  `bigx.lume`；隐藏目录自动扫描不看，所以这条用例只有 `LUME_PATH` 生效才过）。
- 顺手修的两个坑：
  ① `Stmt::Expr` 生成 `(*(int64_t*)(...));` 这种纯取值语句时 clang 报
  `-Wunused-value` —— 而 `v.pop()` 当语句用是最常见的写法。现在统一
  `(void)(...)`。
  ② 按「AST 看起来纯」发的 `#pragma clang loop vectorize` 常被 clang 拒
  （代码生成会插入 `lume_vec_at` 等调用），每条回一句 `loop not vectorized`。
  加 `-Wno-pass-failed` 关掉这类诊断（pragma 只是建议）。
- `examples/bigint/`：可运行的库样板（`big.lume` + `main.lume`，30! / 2^100
  / 21 位乘法都对）。

---

## 🎯 BigInt（第一个用 Lume 写的库）

实现位置：`examples/bigint/`（`big.lume` + `main.lume`）。它跟着测试脚本一起跑
（用例名 `example_bigint`），所以文档里说的能力和示例不会分家。

### 已完成

- [x] 表示：`struct Big { neg: bool, d: Vec[int] }` —— 1e9 进制、**高位在前**、
      无前导零、`-0` 归一成 `0`
- [x] 构造：`big(int)` / `big_str("...")`（支持 `+` / `-` 前缀）
- [x] 加 / 减 / 乘（O(n²) 竖式），带符号，进位链正确
- [x] 除 / 取模：教科书竖式 + **每支二分试商**。不写 Knuth D 的归一化估商——
      那个快，但估错一次就是静默算错；二分慢 30 倍常数，但显然正确。
      商余向零截断，和 C 的整数除法一致
- [x] 比较：只写 `op_eq` / `op_lt`，`!=` `>` `<=` `>=` 由运算符重载自动组合
- [x] `op_str`：插值 `"{a}"` 和 `say(a)` 直接可用
- [x] 快速幂 `pow`、`divmod`、`gcd`、`fib`、`fact`、`sign` / `abs` / `cmp`
- [x] `to_int()` 超出范围**饱和**，不回绕
- [x] 与 node `BigInt` 逐位对齐：30! / 2^100 / fib(100) / 21 位乘法 /
      `p = q*d + r` / `7 / -2 = -3 余 1`

### 已知限制

- `big_str` 不校验非法字符（`atoll` 语义：遇到非数字就停在那）
- Vec 没有自动回收：长循环里反复运算会累积内存，长跑要手动 `b.free()`
- 除法常数大：每位 30 次二分，约 O(30·n·m)，明显慢于加乘
- 没有并行版本（`Vec` 在 `par for` 里不能 push）

---

## 🎯 运算符重载（2026-10-25）

- [x] `src/ops.rs`：运算符 → 方法名的唯一真相（checker / codegen 共用一份解析）
- [x] `+ - * / %` → `op_add` … 一元 `-` → `op_neg`
- [x] `== != < > <= >=` → `op_eq` / `op_lt`（缺的自动换位 / 取反组合）
- [x] `op_str`：插值与 `say` 自动转字符串；没定义就编译期拒绝
      （结构体进 printf varargs 是 UB）
- [x] 左操作数没有地址时（`(a+b)+c`、`f(x)*y`）自动落临时变量
- [x] 方法体内 `self + o`、`self.other()`；接收者可以是任意左值/返回值
- [x] 签名在**定义处**校验（参数个数、`op_str` 返回类型），错误指向写错的 fn
- [x] 验收：`tests_opover.lume`（18 项）+ `fail/op_missing` + `fail/op_str_missing`

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
| v0.4.0 | 模块系统 + 自动扫描 + VS Code 扩展 | 🔶 模块系统已完成，LSP 待做 |
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