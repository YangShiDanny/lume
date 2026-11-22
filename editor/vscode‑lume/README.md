# Lume Visual Studio Code 扩展

此扩展为 `.lume` 文件提供语法高亮、`#` 单行注释配置、关键字和函数补全、
悬停提示、函数符号、基础诊断，以及运行/检查命令。

## 构建和安装

在仓库根目录执行：

```bash
cd editor/vscode‑lume
npm ci
npm run compile
npm run package
```

在 VS Code 中运行「Extensions: Install from VSIX...」，选择生成的
`lume-language-0.1.0.vsix` 文件。

## 命令

- `Lume: Run Current File`：运行当前打开的 `.lume` 文件。
- `Lume: Check Current File`：只检查当前文件，不执行程序。

扩展调用 `lume-bootstrap` CLI。请先在项目根目录运行 `cargo build -p lumec_core`，
并在该项目工作区中使用扩展。默认查找工作区下的
`target/debug/lume-bootstrap`；也可设置 `LUME_BOOTSTRAP` 环境变量指定二进制
的完整路径。