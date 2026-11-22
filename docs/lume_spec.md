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

当前解释器不做完整静态类型检查；类型标注用于保留源码意图，运行值由
解释器管理。

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

## 当前限制

当前 `lume-bootstrap` 的解释器/解析器尚未实现完整语言规范，主要限制包括：

- `if`、`else if` 和 `while` 支持语句形式；`if` 和 `while` 也支持赋值右侧的代码块表达式。`while` 表达式返回最后一次迭代中代码块的值；如果循环一次也未执行，则返回 `0`。
- 数组支持字面量、整数下标读取，以及对已声明数组变量的下标赋值。下标必须是非负整数且位于数组范围内；暂不支持切片和嵌套下标赋值。
- 暂不支持结构体字面量、闭包、泛型、方法调用和模式匹配。
- 不做静态类型检查、模块导入或完整错误恢复。
- 当前 `run` 通过解释器执行；完整 Lume 前端到 LLVM IR 的自举编译链仍在开发。

发现与实现不符时，请以可运行代码和测试为准，并通过项目 issue/PR 反馈。
