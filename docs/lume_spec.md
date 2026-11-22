# Lume v0.1 规范

语言规范以工作区根目录的 `todo.md` 为准。

Rust core只提供 IR、LLVM JIT、GC、bitcode 和最小宿主能力；
Lume 源文件实现 lexer、parser、AST lowering、构建和标准库。
