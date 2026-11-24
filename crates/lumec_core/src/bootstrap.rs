//! Stage-1 bootstrap helpers.
//!
//! These helpers run the Lume-written lexer/parser/lowerer with the stage-0
//! interpreter, parse the textual IR it emits and hand that IR to the Rust JIT
//! backend. This is not a fully self-hosting compiler yet: stage-0 still owns
//! the parser used to load the compiler modules and the LLVM backend.

use crate::evaluator::{Evaluator, Value};
use crate::ir::IrFunc;
use crate::ir_text;
use crate::modules;
use crate::typecheck;
use std::path::{Path, PathBuf};

#[cfg(test)]
const COMPILE_SOURCE: &str = "fn stage1_demo() -> int\nret 42\n";

fn candidate_frontends() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(directory) = std::env::var_os("LUME_LIB_DIR") {
        paths.push(Path::new(&directory).join("bootstrap.lume"));
    }
    paths.push(Path::new("lib/bootstrap.lume").to_path_buf());
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        paths.push(Path::new(&manifest_dir).join("../../lib/bootstrap.lume"));
    }
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(parent) = current_exe.parent()
    {
        paths.push(parent.join("../lib/bootstrap.lume"));
        paths.push(parent.join("lib/bootstrap.lume"));
    }
    paths
}

pub fn frontend_path() -> Result<PathBuf, String> {
    candidate_frontends()
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "cannot find lib/bootstrap.lume; set LUME_LIB_DIR".into())
}

fn load_frontend() -> Result<Evaluator, String> {
    let path = frontend_path()?;
    let functions = modules::load_source_graph(&path)?;
    typecheck::check_program(&functions)?;
    Ok(Evaluator::new(functions))
}

pub fn compile_source(source: &str) -> Result<Vec<IrFunc>, String> {
    let evaluator = load_frontend()?;
    let ir = match evaluator.call("compile", vec![Value::String(source.to_string())])? {
        Value::String(text) => text,
        other => return Err(format!("compile must return a string, got {other:?}")),
    };
    if let Some(message) = ir.strip_prefix("error: ") {
        return Err(message.to_string());
    }
    ir_text::parse_program(&ir).map_err(|error| error.to_string())
}

pub fn bootstrap_world() -> Result<String, String> {
    let evaluator = load_frontend()?;
    let source = "fn main() -> int\nret 42\n";
    match evaluator.call("compile", vec![Value::String(source.into())])? {
        Value::String(text) => Ok(text),
        other => Err(format!("compile must return a string, got {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LumeCore;

    #[test]
    fn lume_frontend_lowers_a_function_to_valid_ir() {
        let functions = compile_source(COMPILE_SOURCE).unwrap();
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, "stage1_demo");
        functions[0].validate().unwrap();
    }

    #[test]
    fn stage1_ir_executes_through_the_rust_jit_backend() {
        let functions = compile_source(COMPILE_SOURCE).unwrap();
        let mut core = LumeCore::new();
        for function in functions {
            core.define_function(function).unwrap();
        }
        assert_eq!(core.call("stage1_demo", &[]).unwrap(), 42);
    }

    #[test]
    fn lume_frontend_lowers_if_else_through_jit() {
        let source = "\
fn pick(flag:int) -> int
    if flag > 0
        ret 1
    else
        ret 2
    end

fn main() -> int
ret pick(0)
";
        let functions = compile_source(source).unwrap();
        let mut core = LumeCore::new();
        for function in functions {
            core.define_function(function).unwrap();
        }
        assert_eq!(core.call("main", &[]).unwrap(), 2);
    }

    #[test]
    fn bootstrap_world_compiles_embedded_source() {
        let value = bootstrap_world().unwrap();
        assert!(value.contains("func main 0"), "{value}");
        assert!(value.contains("const 0 42"), "{value}");
    }

    #[test]
    fn lume_frontend_lowers_nested_control_flow() {
        let source = "\
fn sum_to(limit:int) -> int
    total:int = 0
    value:int = 1
    while value <= limit
        total := total + value
        value := value + 1
    end
ret total

fn main() -> int
ret sum_to(10)
";
        let functions = compile_source(source).unwrap();
        let mut core = LumeCore::new();
        for function in functions {
            core.define_function(function).unwrap();
        }
        assert_eq!(core.call("main", &[]).unwrap(), 55);
    }
}
