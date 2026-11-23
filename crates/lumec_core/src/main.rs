use lumec_core::LumeCore;
use lumec_core::evaluator::{Evaluator, Value};

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  lume-bootstrap [run] <file.lume>");
    eprintln!("  lume-bootstrap check <file.lume>");
    eprintln!("  lume-bootstrap jit <file.lume>");
    eprintln!("  lume-bootstrap llvm-ir <file.lume>");
    eprintln!("  lume-bootstrap --lsp");
}

fn run_llvm(
    functions: &[lumec_core::parser::FuncDef],
    emit_ir: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let compiled = lumec_core::codegen::compile_program(functions)?;
    let mut core = LumeCore::new();
    for function in compiled {
        core.define_function(function)?;
    }
    if emit_ir {
        print!("{}", core.llvm_ir()?);
        return Ok(());
    }
    let entry = if core_function_exists(functions, "main") {
        "main"
    } else if core_function_exists(functions, "entry") {
        "entry"
    } else if core_function_exists(functions, "boot") {
        "boot"
    } else {
        return Err("no main, entry, or boot function".into());
    };
    println!("{}", core.call(entry, &[])?);
    Ok(())
}

fn core_function_exists(functions: &[lumec_core::parser::FuncDef], name: &str) -> bool {
    functions.iter().any(|function| function.name == name)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let core = LumeCore::new();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--lsp") {
        print_usage();
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--help")
        || args.first().map(String::as_str) == Some("-h")
    {
        print_usage();
        return Ok(());
    }
    if args.is_empty() {
        let bootstrap_path = "lib/bootstrap.lume";
        let bootstrap_source = core.read_file(bootstrap_path)?;
        let source = String::from_utf8_lossy(&bootstrap_source);
        if source.is_empty() {
            return Err("lib/bootstrap.lume is empty".into());
        }
        println!("loaded {}", bootstrap_path);
        println!("Lume bootstrap world ready");
        return Ok(());
    }

    let (command, path) = match args.as_slice() {
        [path] if !path.starts_with('-') => ("run", path.as_str()),
        [command, path] => (command.as_str(), path.as_str()),
        _ => {
            print_usage();
            return Ok(());
        }
    };
    if path.ends_with(".toml") {
        let manifest = core.read_file(path)?;
        let manifest = String::from_utf8_lossy(&manifest);
        let project_dir = std::path::Path::new(path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut lume_source = None;
        for line in manifest.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("path = \"")
                && let Some(found) = rest.strip_suffix("\"")
            {
                let module_path = project_dir.join(found);
                if found.ends_with(".lume") {
                    lume_source = Some(module_path);
                } else if found.ends_with(".c") {
                    let output = project_dir.join(format!(
                        "{}.bc",
                        module_path
                            .file_stem()
                            .and_then(|v| v.to_str())
                            .unwrap_or("module")
                    ));
                    let status = std::process::Command::new("clang")
                        .args(["-emit-llvm", "-c"])
                        .arg(&module_path)
                        .arg("-o")
                        .arg(&output)
                        .status();
                    match status {
                        Ok(status) if status.success() => {}
                        Ok(status) => {
                            eprintln!("{}: clang failed with {}", path, status);
                            std::process::exit(1);
                        }
                        Err(error) => {
                            eprintln!("{}: clang: {}", path, error);
                            std::process::exit(1);
                        }
                    }
                }
            }
        }
        let Some(lume_source) = lume_source else {
            eprintln!("{}: manifest has no Lume module", path);
            std::process::exit(1);
        };
        let resolved = lume_source.to_str().unwrap_or_default();
        let funcs = match lumec_core::modules::load_source_graph(&lume_source) {
            Ok(funcs) => funcs,
            Err(error) => {
                eprintln!("{}", error);
                std::process::exit(1);
            }
        };
        if let Err(error) = lumec_core::typecheck::check_program(&funcs) {
            eprintln!("{}: {}", resolved, error);
            std::process::exit(1);
        }
        if command == "jit" || command == "llvm-ir" {
            run_llvm(&funcs, command == "llvm-ir")?;
            return Ok(());
        }
        if command == "check" {
            println!("{}: OK ({} functions)", resolved, funcs.len());
            return Ok(());
        }
        let evaluator = Evaluator::new(funcs);
        let entry = if evaluator.has_function("main") {
            "main"
        } else if evaluator.has_function("entry") {
            "entry"
        } else {
            eprintln!("{}: no main or entry function", resolved);
            std::process::exit(1);
        };
        match evaluator.call(entry, vec![]) {
            Ok(Value::Int(value)) => println!("{}", value),
            Ok(Value::UInt(value)) => println!("{}", value),
            Ok(Value::Float(value)) => println!("{}", value),
            Ok(Value::Bool(value)) => println!("{}", value),
            Ok(Value::String(value)) => println!("{}", value),
            Ok(Value::Array(values)) => println!("{:?}", values),
            Ok(Value::Struct { name, .. }) => println!("<{name}>"),
            Ok(Value::Closure { .. }) => println!("<closure>"),
            Ok(Value::FfiLibrary(_)) => println!("<ffi library>"),
            Ok(Value::FfiSymbol(_)) => println!("<ffi symbol>"),
            Ok(Value::FfiCallback(_)) => println!("<ffi callback>"),
            Ok(Value::FfiBuffer(buffer)) => println!("<ffi buffer {} bytes>", buffer.size()),
            Ok(Value::FfiPointer(pointer)) => println!("ptr({pointer:p})"),
            Ok(Value::GcBuffer(buffer)) => println!("<gc buffer {} bytes>", buffer.len()),
            Ok(Value::Unit) => {}
            Err(error) => {
                eprintln!("{}: {}", resolved, error);
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let funcs = match lumec_core::modules::load_source_graph(path) {
        Ok(funcs) => funcs,
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    };
    if let Err(error) = lumec_core::typecheck::check_program(&funcs) {
        eprintln!("{}: {}", path, error);
        std::process::exit(1);
    }
    if command == "jit" || command == "llvm-ir" {
        run_llvm(&funcs, command == "llvm-ir")?;
        return Ok(());
    }
    if command == "check" {
        println!("{}: OK ({} functions)", path, funcs.len());
        return Ok(());
    }
    let evaluator = Evaluator::new(funcs);
    let entry = if evaluator.has_function("main") {
        "main"
    } else if evaluator.has_function("entry") {
        "entry"
    } else if evaluator.has_function("boot") {
        "boot"
    } else {
        eprintln!("{}: no main, entry, or boot function", path);
        std::process::exit(1);
    };
    match evaluator.call(entry, vec![]) {
        Ok(Value::Int(value)) => println!("{}", value),
        Ok(Value::UInt(value)) => println!("{}", value),
        Ok(Value::Float(value)) => println!("{}", value),
        Ok(Value::Bool(value)) => println!("{}", value),
        Ok(Value::String(value)) => println!("{}", value),
        Ok(Value::Array(values)) => println!("{:?}", values),
        Ok(Value::Struct { name, .. }) => println!("<{name}>"),
        Ok(Value::Closure { .. }) => println!("<closure>"),
        Ok(Value::FfiLibrary(_)) => println!("<ffi library>"),
        Ok(Value::FfiSymbol(_)) => println!("<ffi symbol>"),
        Ok(Value::FfiCallback(_)) => println!("<ffi callback>"),
        Ok(Value::FfiBuffer(buffer)) => println!("<ffi buffer {} bytes>", buffer.size()),
        Ok(Value::FfiPointer(pointer)) => println!("ptr({pointer:p})"),
        Ok(Value::GcBuffer(buffer)) => println!("<gc buffer {} bytes>", buffer.len()),
        Ok(Value::Unit) => {}
        Err(error) => {
            eprintln!("{}: {}", path, error);
            std::process::exit(1);
        }
    }
    Ok(())
}
