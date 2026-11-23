# Lume Visual Studio Code 扩展

此扩展为 `.lume` 文件提供语法高亮、两套配色主题、LSP 补全与悬停、编译器诊断、
签名帮助、函数符号，以及运行/检查命令。

## 功能

### 配色

扩展自带两套主题，在「Preferences: Color Theme」中选择：

- **Lume Night**：深色，黑底配薄荷绿/紫/橙强调色，接近 LumeOS 网页风格。
- **Lume Day**：浅色，白底配同色系强调色。

语法按类别细分：控制关键字、逻辑运算符、函数定义/调用、内置函数、类型、常量、
数字、字符串、转义、变量、注解分隔符与运算符，各主题分别着色。即使用其它主题，
TextMate 作用域也采用通用命名，能继承对应的配色。

扩展图标和 `.lume` 文件图标使用 LumeOS 网页的四角星标志。运行
「Preferences: File Icon Theme」并选择 **Lume Icons**，即可在资源管理器里看到 `.lume`
文件的专属图标。

### 补全与辅助

- 自动补全：LSP 服务提供关键字（含 `fn` / `if` / `while` 片段）、类型、内置函数
  （带签名）和当前文件函数。
- 签名帮助：输入 `(` 或 `,` 时显示内置函数和自定义函数的参数签名。
- 悬停：关键字、类型、内置函数和自定义函数的说明。
- 符号大纲：`fn` 定义出现在 Explorer 的 Outline 和 Go to Symbol 中。
- 代码片段：`lumemain`、`fn`、`ifelse`、`ifexpr`、`while`、`match`、`let`、
  `print`、`log` 等。
- 编辑体验：`end` / `else` 自动反缩进，`fn` / `if` / `while` / `match` 自动缩进与折叠，
  括号/引号自动配对。

### LSP 诊断

LSP 服务直接复用 Lume parser 和 type checker，在打开和编辑文档时报告语法与类型
错误。服务通过 stdio 运行 `lume-bootstrap --lsp`，不需要单独安装 Node.js 语言服务。
如果服务器未能启动，扩展会显示错误消息。

## 构建和安装

```bash
cd editor/vscode‑lume
npm ci
npm run compile
npm run package
```

在 VS Code 中运行「Extensions: Install from VSIX...」，选择生成的
`lume-language-0.1.0.vsix` 文件。

## 设置

- `lume.bootstrapPath`：`lume-bootstrap` 的完整路径，覆盖自动探测；同一个二进制
  同时提供 LSP、运行和检查命令。
- `lume.replCommand`：REPL 命令，默认 `lume`。

## 命令

- `Lume: Run Current File`：运行当前打开的 `.lume` 文件。
- `Lume: Check Current File`：只检查当前文件，不执行程序。
- `Lume: Open REPL`：在集成终端打开 Lume REPL。

扩展调用 `lume-bootstrap` CLI。运行/检查命令默认查找工作区下的
`target/release/lume-bootstrap`（Windows 为 `.exe`）；也可设置 `LUME_BOOTSTRAP`
环境变量或 `lume.bootstrapPath` 配置指定路径。REPL 命令依赖 PATH 中可找到 `lume`。
