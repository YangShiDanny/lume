pub mod buffer;
pub mod builtin;
pub mod clock;
pub mod codegen;
pub mod evaluator;
pub mod ffi;
pub mod gc;
pub mod ir;
mod jit;
pub mod lsp;
pub mod manifest;
pub mod modules;
pub mod parser;
pub mod project;
#[cfg(feature = "system-sleef")]
pub(crate) mod sleef;
pub mod typecheck;
pub mod typeobj;

pub use jit::{BuiltinFn, CoreError, LumeCore};
