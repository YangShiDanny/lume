# LumeC

Lume v0.1 自举工程。唯一 Rust/Cargo 组件是 `crates/lumec_core`；
编译器、构建工具和标准库由 `lib/**/*.lume` 提供。

```bash
cargo build --workspace
cargo run -p lumec_core --bin lume-bootstrap
```
