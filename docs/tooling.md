# Lume 开发辅助工具

## Language Server Protocol

`lume-bootstrap --lsp` 通过标准输入/输出运行 LSP 服务，VS Code 扩展会在打开
`.lume` 文件时自动启动它。也可以用 `cargo run -p lumec_core -- --lsp` 手动启动
以供其他 LSP 客户端连接；标准输出只用于 JSON-RPC 消息。

服务当前提供：

- 语法补全：关键字、类型、内置函数和当前文件中的函数。
- Hover：关键字、类型、内置函数签名和本文件函数签名。
- 实时诊断：复用 Lume parser 和 type checker，在打开或编辑文档后发布错误。

编辑器扩展还提供函数签名帮助和文档符号。LSP 目前检查当前打开的文本内容，
尚未跨导入模块构建完整的工作区索引。

## 从 manifest 生成文档

`docs` 子命令直接读取 manifest，不会编译项目模块：

```bash
# 将生成的 Markdown 输出到标准输出
cargo run -p lumec_core -- docs examples/demo_c_interop/manifest.toml

# 直接写入指定文件
cargo run -p lumec_core -- docs examples/demo_c_interop/manifest.toml docs/demo_c_interop.md
```

文档列出 manifest 中的模块路径、语言、导出符号、Lume 导出和 C 外部函数签名，
并附上项目的 check/run/jit/header 命令。更新 manifest 后重新运行命令即可同步文档。

## 自动检查示例

从仓库根目录执行：

```bash
cargo build -p lumec_core
bash scripts/test-examples.sh target/debug/lume-bootstrap
```

脚本检查代表性 `.lume` 文件和混合 C/Lume manifest，并断言解释执行/JIT 示例的
标准输出；同时验证 LLVM IR 和 manifest 文档生成命令。GitHub Actions Release 构建
后复用同一脚本，以避免本地与 CI 维护两份示例清单。
