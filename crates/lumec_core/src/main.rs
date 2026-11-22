use lumec_core::LumeCore;
use lumec_core::evaluator::{Evaluator, Value};
use lumec_core::parser::parse_source;

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  lume-bootstrap [run] <file.lume>");
    eprintln!("  lume-bootstrap check <file.lume>");
    eprintln!("  lume-bootstrap --lsp");
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
            if let Some(rest) = line.strip_prefix("path = \"") {
                if let Some(found) = rest.strip_suffix("\"") {
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
        }
        let Some(lume_source) = lume_source else {
            eprintln!("{}: manifest has no Lume module", path);
            std::process::exit(1);
        };
        let resolved = lume_source.to_str().unwrap_or_default();
        let source = core.read_file(resolved)?;
        let source = String::from_utf8_lossy(&source);
        let funcs = match parse_source(&source) {
            Ok(funcs) => funcs,
            Err(error) => {
                eprintln!("{}: {}", resolved, error);
                std::process::exit(1);
            }
        };
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
            Ok(Value::Float(value)) => println!("{}", value),
            Ok(Value::Bool(value)) => println!("{}", value),
            Ok(Value::String(value)) => println!("{}", value),
            Ok(Value::Array(values)) => println!("{:?}", values),
            Ok(Value::Unit) => {}
            Err(error) => {
                eprintln!("{}: {}", resolved, error);
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let source = core.read_file(path)?;
    let source = String::from_utf8_lossy(&source);
    let funcs = match parse_source(&source) {
        Ok(funcs) => funcs,
        Err(error) => {
            eprintln!("{}: {}", path, error);
            std::process::exit(1);
        }
    };
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
        Ok(Value::Float(value)) => println!("{}", value),
        Ok(Value::Bool(value)) => println!("{}", value),
        Ok(Value::String(value)) => println!("{}", value),
        Ok(Value::Array(values)) => println!("{:?}", values),
        Ok(Value::Unit) => {}
        Err(error) => {
            eprintln!("{}: {}", path, error);
            std::process::exit(1);
        }
    }
    Ok(())
}
