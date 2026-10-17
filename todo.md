# Lume TODO

> 最后更新：2026-10-17
> 状态：核心编译器可用，20 项验证测试全绿，三平台 CI 通过

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

### 性能特性
- [x] `@cache`（开放寻址哈希表，O(1)）
- [x] `@precompute`（编译期查表 + OpenMP + 文件缓存）
- [x] `par for`（自动切块 + reduction + 串行阈值）
- [x] `LUME_RESTRICT`（数组无别名）
- [x] `LUME_PURE`（纯函数 `__attribute__((const))`）
- [x] `#pragma clang loop vectorize` 提示
- [x] 默认 `-O3`，`--native` / `--lto` / `--fastmath`
- [x] `lume_cpu_count()` 缓存

### 工具链
- [x] 跨平台 runtime（Windows / Linux / macOS）
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

## 📋 待做（按批次）

### 第一批：语言基础（1 周）

- [ ] **数组跨函数**（设计已完成）
  - [ ] `mod.rs` 加 `fn_signatures` / `array_params`
  - [ ] `items.rs` 参数生成拆 `(T* data, int64_t len)`
  - [ ] `expr.rs` 调用处追加长度
  - [ ] `stmt.rs` `for` 支持数组参数遍历
  - [ ] checker 放宽数组参数检查

- [ ] **`if` 表达式**
  - [ ] codegen 生成 C 的三元或 if-else 表达式

- [ ] **`match` 结构体解构**
  - [ ] 需要类型标签（enum tag）或 C union

- [ ] **字符串操作**
  - [ ] `str_split` / `str_contains` / `str_replace` / `str_starts_with` / `str_ends_with`

- [ ] **整数除零检查**
  - [ ] 编译期常量 0 报错
  - [ ] 运行时变量 0 检查（`lume_div_zero()`）

### 第二批：类型系统（2 周）

- [ ] **`T?` 可选值**
  - [ ] AST / checker / codegen 全链路
  - [ ] `?` 错误传播

- [ ] **`T!` 错误值**
  - [ ] 同上

- [ ] **切片 `a[1..3]`**
  - [ ] codegen 支持（parser 已支持）

- [ ] **多返回值 / 元组**
  - [ ] `fn divmod(a, b) -> (int, int)`
  - [ ] C 里用 struct 包装

- [ ] **泛型 struct**
  - [ ] `struct Box[T] { value: T }`
  - [ ] 单态化扩展到 struct

- [ ] **`i128` / `u128`**
  - [ ] 类型映射 `__int128`
  - [ ] 打印函数 `lume_print_i128`

- [ ] **Debug 模式溢出检查**
  - [ ] `__builtin_add_overflow` 系列
  - [ ] `--debug` 开关

### 第三批：模块系统 + 工具链（1 个月）

- [ ] **文件名即模块名**
  - [ ] 路径 → 模块名映射（`math.lume` → `math`，`gui/window.lume` → `gui.window`）

- [ ] **自动扫描加载**
  - [ ] 从 `main.lume` 同级+子目录递归扫描
  - [ ] 排除 `target/` / `.lume_cache/` / `.git/` / `node_modules/` / `_` 前缀

- [ ] **短名冲突检测**
  - [ ] 两个 `math` → 报错，要求完整路径

- [ ] **`pub` 可见性**
  - [ ] 非 `pub` 私有，跨模块不可见

- [ ] **循环依赖检测**
  - [ ] 图上找环，编译期报错

- [ ] **`@no_std`**
  - [ ] 文件级关闭 std

- [ ] **`@manual_load` / `@exclude`**
  - [ ] 关掉自动扫描 / 排除指定目录

- [ ] **`@thread_safe` / `@not_thread_safe`**
  - [ ] 函数级标记
  - [ ] 自动推断（纯函数 → thread_safe）
  - [ ] `par for` 里只能调 `@thread_safe`

- [ ] **CLI 统一命名为 `lumen`**
  - [ ] `Cargo.toml` 改 name
  - [ ] 文档 / 用法提示全部同步

- [ ] **VS Code 扩展**
  - [ ] 语法高亮（TextMate）
  - [ ] `language-configuration.json`
  - [ ] 代码片段
  - [ ] LSP（补全 / 跳转 / 诊断）

- [ ] **`lumen fmt`**
  - [ ] 格式化工具

- [ ] **`lumen test`**
  - [ ] 测试框架

- [ ] **`lumen doc`**
  - [ ] 从注释生成文档

### 第四批：并发（1 周）

- [ ] **`par f(x)` + `await t`**
  - [ ] `Task[T]` 类型
  - [ ] parser 支持 `par` 前缀
  - [ ] codegen 生成 `_task_int64` / `_task_double` / struct
  - [ ] `await` 展开成 `join` + 读结果

- [ ] **`lume_say` 全局锁**
  - [ ] `par for` 里 `say` 不交错
  - [ ] `say.rs` 的 `printf` 改成 `lume_say`

- [ ] **`write` 内置（不换行）**
  - [ ] 用于 BigInt 连续输出

- [ ] **线程池**
  - [ ] 替代裸 `pthread_create`
  - [ ] 参考 `pthreadpool`

- [ ] **`par for` 支持嵌套控制流**
  - [ ] `while` / `for` / `match` 进 worker

- [ ] **`par for` 支持 struct 字段数组**
  - [ ] `collect_used_arrays` 扩展到 struct 字段

- [ ] **`atomic_add` 内置**
  - [ ] 显式原子操作

### 第五批：性能优化（持续）

- [ ] **`@fastmath` 装饰器**
  - [ ] 函数级 `-ffast-math` 效果

- [ ] **多维数组**
  - [ ] `[f32; M; N]` 或 `[f32; M][N]`
  - [ ] 行主序 + 编译期已知步长
  - [ ] 缓存友好重排

- [ ] **管道融合**
  - [ ] `data |> filter |> map |> sum` 合成单循环

- [ ] **无锁日志**
  - [ ] 参考 L3 / fastlog
  - [ ] 线程本地缓冲 + 后台写入
  - [ ] 目标：`say` 1~10 ns

- [ ] **魔数除法**
  - [ ] 自己的 codegen 实现（不依赖 clang）
  - [ ] 为换 LLVM 后端铺路

- [ ] **强度削弱**
  - [ ] `x * 2^n → x << n`
  - [ ] `x % 2^n → x & (2^n-1)`

- [ ] **循环感应变量削强度**
  - [ ] `a[i * stride]` → 指针递增

- [ ] **FMA 融合**
  - [ ] `a * b + c` → `fma(a, b, c)`

- [ ] **`@precompute` 支持 float 表**
  - [ ] 打印用 `%f`，解析用 `parse::<f64>`

### 第六批：生态（持续）

- [ ] **标准库扩展**
  - [ ] `std.io`（文件读写）
  - [ ] `std.collections`（动态数组、哈希表）
  - [ ] `std.math`（更多数学函数）

- [ ] **动态数组 `Vec`**
  - [ ] `{ data, len, cap }` 结构
  - [ ] `push` / `pop` / `reserve`

- [ ] **示例项目**
  - [ ] 排序（快排、归并）
  - [ ] 矩阵乘法（对标 Fortran）
  - [ ] HTTP 服务器
  - [ ] BigInt

- [ ] **包管理**
  - [ ] `lumen.toml`
  - [ ] 依赖下载
  - [ ] 版本解析

- [ ] **官网 / 文档站**
  - [ ] 用户上手教程
  - [ ] API 参考

- [ ] **包仓库**
  - [ ] 类似 crates.io / npm

- [ ] **C 库绑定**
  - [ ] `extern` 真正实现
  - [ ] raylib / SDL2 绑定（GUI）

- [ ] **自举（用 Lume 写 Lume）**
  - [ ] 需要动态数组、字符串、文件 I/O 全部就绪
  - [ ] 长期目标

---

## 🎯 BigInt（第一个用 Lume 写的程序）

### 前置补丁（约 1 小时）

- [ ] **struct 支持定长数组字段**
  - [ ] `gen_struct_decl` 生成 `T name[N]`，不是 `T* name`

- [ ] **`[0; N]` 重复初始化**
  - [ ] AST 加 `Expr::Repeat`
  - [ ] parser 解析 `[value; count]`
  - [ ] codegen 生成 `{0}`

- [ ] **`write` 内置（不换行）**
  - [ ] runtime 加 `lume_write`
  - [ ] checker 加 `write` 到内置表

### 单线程版本

- [ ] `big_from_int` / `big_zero`
- [ ] `big_cmp_abs` / `big_cmp`
- [ ] `big_add_abs` / `big_sub_abs`
- [ ] `big_add` / `big_sub`
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

## 🔗 参考

- [Labor of Division](https://ridiculousfish.com/blog/posts/labor-of-division-episode-i.html) — 魔数除法
- [L3](https://github.com/undoio/l3) — 无锁日志，单线程 ~1ns
- [NanoLog](https://github.com/PlatformLab/NanoLog) — 编译期静态格式化，~80M logs/s
- [fastlog](https://github.com/ArashPartow/fastlog) — 线程本地缓冲异步日志
- [moodycamel::ConcurrentQueue](https://github.com/cameron314/concurrentqueue) — 工业级无锁队列
- [pthreadpool](https://github.com/Maratyszcza/pthreadpool) — 高性能线程池
- [Herbie](https://herbie.uwplse.org/) — 浮点超优化
- [egglog](https://github.com/egraphs-good/egglog) — 超优化框架