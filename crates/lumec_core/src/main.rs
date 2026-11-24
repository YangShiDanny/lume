use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use lumec_core::LumeCore;
use lumec_core::evaluator::{Evaluator, ExternBinding, Value};
use lumec_core::ffi::ForeignLibrary;
use lumec_core::manifest::ExternDecl;
use lumec_core::project::{self, BuiltProject};

struct JitExtern {
    name: String,
    signature: lumec_core::ffi::FfiSignature,
    library: Option<PathBuf>,
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  lume-bootstrap [run] <file.lume>");
    eprintln!("  lume-bootstrap check <file.lume>");
    eprintln!("  lume-bootstrap jit <file.lume>");
    eprintln!("  lume-bootstrap llvm-ir <file.lume>");
    eprintln!("  lume-bootstrap <run|check|jit|llvm-ir|header> <manifest.toml>");
    eprintln!("  lume-bootstrap docs <manifest.toml> [output.md]");
    eprintln!("  lume-bootstrap bootstrap <file.lume>");
    eprintln!("  lume-bootstrap bootstrap-ir <file.lume>");
    eprintln!("  lume-bootstrap --lsp");
}

fn print_value(value: Value) {
    match value {
        Value::Int(value) => println!("{value}"),
        Value::UInt(value) => println!("{value}"),
        Value::Float(value) => println!("{value}"),
        Value::Bool(value) => println!("{value}"),
        Value::String(value) => println!("{value}"),
        Value::Array(values) => println!("{values:?}"),
        Value::Struct { name, .. } => println!("<{name}>"),
        Value::Closure { .. } => println!("<closure>"),
        Value::FfiLibrary(_) => println!("<ffi library>"),
        Value::FfiSymbol(_) => println!("<ffi symbol>"),
        Value::FfiCallback(_) => println!("<ffi callback>"),
        Value::FfiBuffer(buffer) => println!("<ffi buffer {} bytes>", buffer.size()),
        Value::FfiAggregate(value) => {
            println!("<ffi aggregate {:?} {} bytes>", value.ty, value.bytes.len())
        }
        Value::FfiPointer(pointer) => println!("ptr({pointer:p})"),
        Value::GcBuffer(buffer) => println!("<gc buffer {} bytes>", buffer.len()),
        Value::Unit => {}
    }
}

fn core_function_exists(functions: &[lumec_core::parser::FuncDef], name: &str) -> bool {
    functions.iter().any(|function| function.name == name)
}

fn run_llvm(
    functions: &[lumec_core::parser::FuncDef],
    emit_ir: bool,
    bitcodes: &[PathBuf],
    externs: &[JitExtern],
    prefer_entry: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut core = LumeCore::new();
    for extern_decl in externs {
        if let Some(library) = &extern_decl.library {
            core.register_dynamic_extern(
                extern_decl.name.clone(),
                library,
                extern_decl.signature.clone(),
            )?;
        } else {
            core.register_extern_signature(
                extern_decl.name.clone(),
                extern_decl.signature.clone(),
            )?;
        }
    }
    for bitcode in bitcodes {
        core.load_bitcode(&std::fs::read(bitcode)?)?;
    }
    let compiled = lumec_core::codegen::compile_program(functions)?;
    for function in compiled {
        core.define_function(function)?;
    }
    if emit_ir {
        print!("{}", core.llvm_ir()?);
        return Ok(());
    }
    let entry = if prefer_entry && core_function_exists(functions, "entry") {
        "entry"
    } else if core_function_exists(functions, "main") {
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

fn load_extern_bindings(
    built: &BuiltProject,
) -> Result<HashMap<String, ExternBinding>, Box<dyn std::error::Error>> {
    let mut bindings = HashMap::new();
    for (name, declaration) in &built.manifest.externs {
        let library_path =
            project::shared_library_for_symbol(built, name, declaration.library.as_deref())?;
        let library = unsafe { ForeignLibrary::open(&library_path) }
            .map_err(|error| -> Box<dyn std::error::Error> { Box::new(error) })?;
        let symbol = Rc::new(library.symbol(name)?);
        bindings.insert(
            name.clone(),
            ExternBinding {
                signature: declaration.signature.clone(),
                symbol,
            },
        );
    }
    Ok(bindings)
}

fn jit_externs(built: &BuiltProject) -> Vec<JitExtern> {
    built
        .manifest
        .externs
        .iter()
        .map(|(name, declaration): (&String, &ExternDecl)| JitExtern {
            name: name.clone(),
            signature: declaration.signature.clone(),
            library: declaration
                .library
                .as_deref()
                .map(|path| built.manifest.project_dir.join(path)),
        })
        .collect()
}

fn call_entry(
    funcs: Vec<lumec_core::parser::FuncDef>,
    externs: HashMap<String, ExternBinding>,
) -> Result<(), String> {
    let evaluator = Evaluator::with_externs(funcs, externs);
    let entry = if evaluator.has_function("main") {
        "main"
    } else if evaluator.has_function("entry") {
        "entry"
    } else if evaluator.has_function("boot") {
        "boot"
    } else {
        return Err("no main, entry, or boot function".into());
    };
    match evaluator.call(entry, Vec::new()) {
        Ok(value) => {
            print_value(value);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn run_manifest(command: &str, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let emit_shared = command == "run";
    let built = project::build_project(path, emit_shared)?;
    let module = built
        .manifest
        .lume_modules()
        .next()
        .ok_or("manifest has no Lume module")?;
    let resolved = module.path.to_string_lossy().to_string();
    let funcs = lumec_core::modules::load_source_graph(&module.path)?;
    let extern_signatures = built
        .manifest
        .externs
        .iter()
        .map(|(name, declaration)| (name.clone(), declaration.signature.clone()))
        .collect();
    if let Err(error) =
        lumec_core::typecheck::check_program_with_externs(&funcs, &extern_signatures)
    {
        return Err(format!("{resolved}: {error}").into());
    }

    match command {
        "header" => {
            print!("{}", project::generate_header(&built, &funcs)?);
            Ok(())
        }
        "jit" | "llvm-ir" => run_llvm(
            &funcs,
            command == "llvm-ir",
            &built.bitcodes,
            &jit_externs(&built),
            built.manifest.build_mode.as_deref() == Some("jit"),
        ),
        "check" => {
            println!("{resolved}: OK ({} functions)", funcs.len());
            Ok(())
        }
        "run" => {
            let externs = load_extern_bindings(&built)?;
            call_entry(funcs, externs)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })
        }
        other => Err(format!("unsupported manifest command `{other}`").into()),
    }
}

fn run_file(command: &str, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let funcs = lumec_core::modules::load_source_graph(path)?;
    if let Err(error) = lumec_core::typecheck::check_program(&funcs) {
        return Err(format!("{}: {error}", path.display()).into());
    }
    if command == "jit" || command == "llvm-ir" {
        return run_llvm(&funcs, command == "llvm-ir", &[], &[], false);
    }
    if command == "check" {
        println!("{}: OK ({} functions)", path.display(), funcs.len());
        return Ok(());
    }
    call_entry(funcs, HashMap::new())
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--lsp") {
        lumec_core::lsp::serve_stdio()?;
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("bootstrap") {
        let ([_, source_path] | [_, source_path, _]) = args.as_slice() else {
            print_usage();
            return Ok(());
        };
        let source = std::fs::read_to_string(source_path)?;
        let functions = lumec_core::bootstrap::compile_source(&source)?;
        let has_main = functions.iter().any(|function| function.name == "main");
        let has_entry = functions.iter().any(|function| function.name == "entry");
        let mut core = LumeCore::new();
        for function in functions {
            core.define_function(function)?;
        }
        let entry = if has_main {
            "main"
        } else if has_entry {
            "entry"
        } else {
            "boot"
        };
        println!("{}", core.call(entry, &[])?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("bootstrap-ir") {
        let ([_, source_path] | [_, source_path, _]) = args.as_slice() else {
            print_usage();
            return Ok(());
        };
        let source = std::fs::read_to_string(source_path)?;
        let functions = lumec_core::bootstrap::compile_source(&source)?;
        let mut core = LumeCore::new();
        for function in functions {
            core.define_function(function)?;
        }
        print!("{}", core.llvm_ir()?);
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("docs") {
        let ([_, manifest_path] | [_, manifest_path, _]) = args.as_slice() else {
            print_usage();
            return Ok(());
        };
        let manifest = lumec_core::manifest::Manifest::parse(manifest_path)?;
        let docs = project::generate_manifest_docs(&manifest);
        if let Some(output_path) = args.get(2) {
            std::fs::write(output_path, docs)?;
            println!("Wrote {}", Path::new(output_path).display());
        } else {
            print!("{docs}");
        }
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
        let source = LumeCore::new().read_file(bootstrap_path)?;
        let source = String::from_utf8_lossy(&source);
        if source.is_empty() {
            return Err("lib/bootstrap.lume is empty".into());
        }
        println!("loaded {bootstrap_path}");
        println!("Lume bootstrap world ready");
        return Ok(());
    }

    let (command, path) = match args.as_slice() {
        [path] if !path.starts_with('-') => ("run", Path::new(path)),
        [command, path] => (command.as_str(), Path::new(path)),
        _ => {
            print_usage();
            return Ok(());
        }
    };
    if path.extension().and_then(|value| value.to_str()) == Some("toml") {
        run_manifest(command, path)
    } else {
        run_file(command, path)
    }
}
