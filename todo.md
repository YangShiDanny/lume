# Lume TODO

> 最后更新：2026-10-17
> 状态：核心编译器可用，20 项验证测试全绿，三平台 CI 通过
> 已合并原「未来优化方向.md」，合并后原文件可删

---

## 设计决策（已定，别改）

| 项目 | 决策 |
|---|---|
| 语言名 | Lume |
| 编译器名 | Lumen（当前代码里还叫 `lume`，待统一） |
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
| 数组语义 | 值类型，无别名，自动加 `restrict` |
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
- [x] `match`（字面量、通配、变量绑定）
- [x] 管道 `|>`
- [x] `if` / `else` / `else if`
- [x] `while` / `for` / `break` / `continue`
- [x] 前向声明（互相递归、调用后面定义的函数）

### 性能特性（已有）
- [x] `@precompute` 编译期查表（+ OpenMP + 文件缓存）
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
- [x] 20 项验证测试（`main.lume`）
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

---

## 🔥 已知 bug（待修）

- [ ] `if` 表达式未实现（`x := if c { 1 } else { 2 }` 生成非法 C）
- [ ] `malloc` 结果类型在部分 codegen 路径仍可能出错
- [ ] `match` 结构体解构恒为真
- [ ] `match` 值被强制转 `int64_t`（只能 match 整数/布尔）
- [ ] 默认参数不生效
- [ ] 命名参数不支持
- [ ] 整数除零是 UB（需加检查）
- [ ] `INT64_MIN / -1` 溢出是 UB
- [ ] 数组不能跨函数传
- [ ] `nums.len()` 对数组参数返回不存在的 `nums_len`

---

## 📊 性能优化全景表

### 一、clang 不敢做（Lume 的独有优势）

- [x] **#1 数组参数加 `restrict`** — 无裸指针，值语义保证无别名 → 向量化 2~8 倍
- [x] **#2 纯函数加 `__attribute__((const))`** — `purity.rs` 保守证明 → 消除重复调用
- [ ] **#3 函数级 `@fastmath`** — 只对指定函数开 `-ffast-math` 效果 → 浮点重排、FMA、倒数
- [x] **#4 边界检查消除** — 编译期知道数组长度 → 快 10~30%
- [x] **#5 无别名下的自动并行** — `par for` 是显式契约 → 多核 5~8 倍
- [ ] **#6 管道融合成单循环** — `|>` 语义明确 → 内存 3N → N
- [x] **#7 编译期执行纯函数** — `@precompute` 执行整个函数 → 运行时零开销
- [x] **#8 自动 reduction** — `par for` 里识别 `sum +=` → 并行求和 5~8 倍
- [ ] **#9 激进循环不变式外提** — `@fastmath` 下可以 → 减少重复计算
- [ ] **#10 跨函数常量传播** — 有整个 AST → 消除冗余调用
- [ ] **#11 浮点代数简化** — `@fastmath` 下 `x+0`/`x*1` 可简化 → 消除冗余
- [ ] **#12 FMA 融合** — `@fastmath` 下无条件做 → 一条指令，无中间舍入
- [ ] **#13 多维数组步长已知** — 编译期知道行主序、步长 → 缓存友好重排
- [ ] **#14 切片无别名** — 编译期知道不重叠 → 子数组向量化
- [x] **#15 `@cache` 哈希表** — 生成的缓存可优化 → O(n²) → O(n)

**已完成 6 / 15。**

### 二、clang 会做，但 Lume 可以更早/更激进

- [ ] **#16 强度削弱 `x*2^n → x<<n`** — codegen 里无条件做
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

- [ ] **数组跨函数**（设计已完成）
  - [ ] `mod.rs` 加 `fn_signatures` / `array_params`
  - [ ] `items.rs` 参数生成拆 `(T* data, int64_t len)`
  - [ ] `expr.rs` 调用处追加长度
  - [ ] `stmt.rs` `for` 支持数组参数遍历
  - [ ] checker 放宽数组参数检查

- [ ] **`if` 表达式**（优化 #3 的前置）

- [ ] **`match` 结构体解构**（需要类型标签）

- [ ] **字符串操作**
  - [ ] `str_split` / `str_contains` / `str_replace` / `str_starts_with` / `str_ends_with`

- [ ] **整数除零检查**
  - [ ] 编译期常量 0 报错
  - [ ] 运行时变量 0 检查（`lume_div_zero()`）

### 第二批：类型系统

- [ ] **`T?` 可选值** + `?` 错误传播
- [ ] **`T!` 错误值**
- [ ] **切片 `a[1..3]`**（parser 已支持，补 codegen）
- [ ] **多返回值 / 元组**
- [ ] **泛型 struct**
- [ ] **`i128` / `u128`**
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
- [ ] **VS Code 扩展**（语法高亮 + LSP）
- [ ] **`lumen fmt`**
- [ ] **`lumen test`**
- [ ] **`lumen doc`**

### 第四批：并发

- [ ] **`par f(x)` + `await t`**（`Task[T]` 类型 + codegen）
- [ ] **`lume_say` 全局锁**（多线程 print 不交错）
- [ ] **`write` 内置（不换行）**
- [ ] **线程池**（替代裸 `pthread_create`）
- [ ] **`par for` 支持嵌套控制流**（`while` / `for` / `match` 进 worker）
- [ ] **`par for` 支持 struct 字段数组**
- [ ] **`atomic_add` 内置**

### 第五批：性能优化（对应优化表）

**第二批：**

- [ ] **#3 函数级 `@fastmath`**
- [ ] **#6 管道融合**（数据管道核心）
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

- [ ] **无锁日志**（参考 L3 / fastlog，`say` 1~10 ns）
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

- [ ] **struct 支持定长数组字段**（`gen_struct_decl` 生成 `T name[N]`）
- [ ] **`[0; N]` 重复初始化**（AST 加 `Expr::Repeat`）
- [ ] **`write` 内置（不换行）**

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
5. 推送 GitHub（CI 三平台验证）

### 每修一个 bug

1. 先写能复现的测试
2. 用隔离探针定位（参考 `-=` 归约误判的调试方式）
3. 修完测试全绿
4. **不确定的不顺手改**（把「别顺手修它」写进代码注释）

### tcc / 非目标平台

**tcc 输出可以发现问题，但不能直接当作「Lume 错了」的证据，必须先隔离验证。**

### 提交前检查

- [ ] `cargo build` 零警告
- [ ] `main.lume` 20 项全绿
- [ ] `LUME.md` 同步
- [ ] CI 三平台通过

---

## 🏁 里程碑

| 版本 | 目标 | 状态 |
|---|---|---|
| v0.1.0 | 核心编译器 + 20 项验证 | ✅ 已完成 |
| v0.2.0 | 数组跨函数 + `if` 表达式 + `match` 解构 + 字符串操作 | 待做 |
| v0.3.0 | `T?` / `T!` + 切片 + 元组 + 泛型 struct | 待做 |
| v0.4.0 | 模块系统 + 自动扫描 + VS Code 扩展 | 待做 |
| v0.5.0 | `par f()` + `await` + 线程池 + `lume_say` | 待做 |
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