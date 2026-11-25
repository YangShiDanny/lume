//! Build helpers for manifest-driven Lume projects.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use crate::ffi::FfiAbiName;
use crate::manifest::{Manifest, ModuleDecl, ffi_c_type, lume_c_type};
use crate::parser::FuncDef;

#[derive(Debug, Clone)]
pub struct BuiltProject {
    pub manifest: Manifest,
    pub bitcodes: Vec<PathBuf>,
    /// C module path -> generated shared library path.
    pub shared_libraries: HashMap<PathBuf, PathBuf>,
}

/// Compiles C modules in the manifest to LLVM bitcode and, optionally, shared
/// libraries for the interpreter's dynamic FFI path.
pub fn build_project(path: impl AsRef<Path>, emit_shared: bool) -> Result<BuiltProject, String> {
    let manifest = Manifest::parse(path)?;
    let mut bitcodes = Vec::new();
    let mut shared_libraries = HashMap::new();

    for module in manifest.c_modules() {
        let bitcode = bitcode_path(&manifest.project_dir, module);
        run_command(
            Command::new("clang").args(module.cflags.iter()).args([
                "-emit-llvm",
                "-c",
                module.path.to_string_lossy().as_ref(),
                "-o",
                bitcode.to_string_lossy().as_ref(),
            ]),
            "clang bitcode generation",
        )?;
        validate_exported_symbols(&bitcode, &module.export)?;
        bitcodes.push(bitcode);

        if emit_shared {
            let library = shared_library_path(&manifest.project_dir, module);
            let mut command = Command::new("clang");
            command.args(module.cflags.iter()).arg("-shared");
            if cfg!(not(target_os = "windows")) {
                command.arg("-fPIC");
            }
            command.arg(&module.path).arg("-o").arg(&library);
            if cfg!(not(target_os = "windows")) {
                command.arg("-lm");
            }
            run_command(&mut command, "clang shared library generation")?;
            shared_libraries.insert(module.path.clone(), library);
        }
    }

    Ok(BuiltProject {
        manifest,
        bitcodes,
        shared_libraries,
    })
}

pub fn shared_library_for_symbol(
    project: &BuiltProject,
    symbol: &str,
    explicit: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        let resolved = project.manifest.project_dir.join(path);
        if !resolved.exists() {
            return Err(format!(
                "extern `{symbol}` library does not exist: {}",
                resolved.display()
            ));
        }
        return Ok(resolved);
    }
    for module in project.manifest.c_modules() {
        if module.export.iter().any(|name| name == symbol) {
            return project
                .shared_libraries
                .get(&module.path)
                .cloned()
                .ok_or_else(|| {
                    format!("C module for `{symbol}` was not built as a shared library")
                });
        }
    }
    Err(format!(
        "extern `{symbol}` has no library and no C module exports it"
    ))
}

pub fn generate_header(project: &BuiltProject, functions: &[FuncDef]) -> Result<String, String> {
    let mut exported: Vec<&FuncDef> = functions
        .iter()
        .filter(|function| {
            function.attrs.iter().any(|attr| attr == "ffi_export")
                && project
                    .manifest
                    .exports
                    .iter()
                    .any(|name| name == &function.name)
        })
        .collect();
    exported.sort_by(|a, b| a.name.cmp(&b.name));

    let guard_name = project
        .manifest
        .name
        .clone()
        .unwrap_or_else(|| "lume_module".to_string())
        .to_uppercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let guard = format!("LUME_GENERATED_{guard_name}_H");

    let mut output = String::new();
    output.push_str(&format!("#ifndef {guard}\n#define {guard}\n\n"));
    output.push_str("#include <stdint.h>\n\n");
    output.push_str("#ifdef __cplusplus\nextern \"C\" {\n#endif\n\n");

    for function in exported {
        let result = lume_c_type(function.ret_ty.as_deref())?;
        let mut params = Vec::new();
        for param in &function.params {
            params.push(format!(
                "{} {}",
                lume_c_type(param.ty.as_deref())?,
                param.name
            ));
        }
        output.push_str(&format!(
            "/* #[ffi_export] */ {result} {}({});\n",
            function.name,
            params.join(", ")
        ));
    }

    if !project.manifest.exports.is_empty() || !project.manifest.externs.is_empty() {
        output.push('\n');
    }
    let mut externs: Vec<_> = project.manifest.externs.iter().collect();
    externs.sort_by_key(|(name, _)| *name);
    for (name, declaration) in externs {
        let mut params: Vec<String> = declaration
            .signature
            .args
            .iter()
            .map(|ty| format!("{} arg", ffi_c_type(ty)))
            .collect();
        for (index, param) in params.iter_mut().enumerate() {
            *param = param.replace("arg", &format!("arg{index}"));
        }
        if declaration.signature.is_variadic() {
            params.push("...".into());
        }
        let abi_attribute = match declaration.signature.abi {
            FfiAbiName::Stdcall if cfg!(target_arch = "x86") && cfg!(windows) => "__stdcall ",
            FfiAbiName::Fastcall if cfg!(target_arch = "x86") && cfg!(windows) => "__fastcall ",
            _ => "",
        };
        output.push_str(&format!(
            "/* extern */ {}{} {}({});\n",
            abi_attribute,
            ffi_c_type(&declaration.signature.result),
            name,
            params.join(", ")
        ));
    }

    output.push_str("\n#ifdef __cplusplus\n}\n#endif\n\n");
    output.push_str(&format!("#endif /* {guard} */\n"));
    Ok(output)
}

pub fn generate_manifest_docs(manifest: &Manifest) -> String {
    let title = manifest.name.as_deref().unwrap_or("Lume project");
    let mut output = format!(
        "# {title}\n\nGenerated from `{}`.\n\n",
        manifest.path.display()
    );
    output.push_str("## Modules\n\n| Language | Path | Exports |\n| --- | --- | --- |\n");
    for module in &manifest.modules {
        let path = module
            .path
            .strip_prefix(&manifest.project_dir)
            .unwrap_or(&module.path)
            .to_string_lossy()
            .replace('|', "\\|");
        let exports = if module.export.is_empty() {
            "-".to_string()
        } else {
            module
                .export
                .iter()
                .map(|symbol| format!("`{symbol}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        output.push_str(&format!(
            "| `{}` | `{}` | {} |\n",
            module.lang, path, exports
        ));
    }

    if !manifest.exports.is_empty() {
        let mut exports = manifest.exports.clone();
        exports.sort();
        output.push_str("\n## Lume exports\n\n");
        for symbol in exports {
            output.push_str(&format!("- `{symbol}`\n"));
        }
    }

    if !manifest.externs.is_empty() {
        output.push_str(
            "\n## External functions\n\n| Symbol | Signature | Library |\n| --- | --- | --- |\n",
        );
        let mut externs: Vec<_> = manifest.externs.iter().collect();
        externs.sort_by_key(|(name, _)| *name);
        for (name, declaration) in externs {
            let library = declaration.library.as_deref().unwrap_or("linked C module");
            let signature = declaration.signature_text.replace('|', "\\|");
            output.push_str(&format!("| `{name}` | `{signature}` | `{library}` |\n"));
        }
    }

    output.push_str(
        "\n## Commands\n\n```sh\n\
         lume-bootstrap check manifest.toml\n\
         lume-bootstrap run manifest.toml\n\
         lume-bootstrap jit manifest.toml\n\
         lume-bootstrap header manifest.toml\n\
         ```\n",
    );
    output
}

fn bitcode_path(project_dir: &Path, module: &ModuleDecl) -> PathBuf {
    let stem = module
        .path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("module");
    project_dir.join(format!("{stem}.bc"))
}

fn shared_library_path(project_dir: &Path, module: &ModuleDecl) -> PathBuf {
    let stem = module
        .path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("module");
    let build_dir = project_dir.join("build");
    if let Err(_error) = std::fs::create_dir_all(&build_dir) {
        return build_dir;
    }
    #[cfg(target_os = "windows")]
    let name = format!("{stem}.dll");
    #[cfg(target_os = "macos")]
    let name = format!("lib{stem}.dylib");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let name = format!("lib{stem}.so");
    build_dir.join(name)
}

fn run_command(command: &mut Command, label: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("{label}: cannot start command: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{label}: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn validate_exported_symbols(bitcode: &Path, exports: &[String]) -> Result<(), String> {
    if exports.is_empty() {
        return Ok(());
    }
    let tool = find_llvm_tool("llvm-nm")
        .ok_or_else(|| "cannot find llvm-nm; install LLVM 23 tools".to_string())?;
    let output = Command::new(tool)
        .arg("--defined-only")
        .arg(bitcode)
        .output()
        .map_err(|error| format!("llvm-nm failed to start: {error}"))?;
    let listing = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        String::from_utf8_lossy(&output.stderr).to_string()
    };
    for symbol in exports {
        if !listing_declares_symbol(&listing, symbol) {
            return Err(format!(
                "{}: C module does not export `{symbol}`",
                bitcode.display()
            ));
        }
    }
    Ok(())
}

/// Darwin bitcode mangled from C prefixes global symbols with `_`. Accept
/// either spelling so the same manifest works on Linux, macOS and Windows.
fn listing_declares_symbol(listing: &str, symbol: &str) -> bool {
    let underscored = format!("_{symbol}");
    listing.lines().any(|line| {
        line.split_whitespace()
            .any(|field| field == symbol || field == underscored)
    })
}

fn find_llvm_tool(name: &str) -> Option<PathBuf> {
    if let Ok(bindir) = Command::new("llvm-config").arg("--bindir").output()
        && bindir.status.success()
    {
        let path = PathBuf::from(String::from_utf8_lossy(&bindir.stdout).trim())
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if path.exists() {
            return Some(path);
        }
    }
    for candidate in [format!("{name}-23"), name.to_string()] {
        let path = PathBuf::from(format!("{candidate}{}", std::env::consts::EXE_SUFFIX));
        if command_exists(&path) {
            return Some(path);
        }
    }
    None
}

fn command_exists(path: &Path) -> bool {
    if path.components().count() > 1 {
        return path.exists();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|directory| directory.join(path))
                .any(|candidate| candidate.exists())
        })
        .unwrap_or(false)
}

fn _source_is_newer(source: &Path, output: &Path) -> bool {
    match (std::fs::metadata(source), std::fs::metadata(output)) {
        (Ok(source), Ok(output)) => {
            source.modified().unwrap_or(SystemTime::UNIX_EPOCH)
                > output.modified().unwrap_or(SystemTime::UNIX_EPOCH)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_symbol_lookup_accepts_darwin_underscore() {
        assert!(listing_declares_symbol(
            "0000000000000000 T c_matmul\n",
            "c_matmul"
        ));
        assert!(listing_declares_symbol(
            "0000000000000000 T _c_matmul\n",
            "c_matmul"
        ));
        assert!(!listing_declares_symbol(
            "0000000000000000 T other\n",
            "c_matmul"
        ));
    }

    #[test]
    fn generates_docs_from_manifest_without_building_modules() {
        let manifest = Manifest::parse("../../examples/demo_c_interop/manifest.toml")
            .expect("demo manifest parses");
        let docs = generate_manifest_docs(&manifest);
        assert!(docs.contains("# demo_c_interop"));
        assert!(docs.contains("| `c` | `src/calc.c` | `c_matmul` |"));
        assert!(docs.contains("| `c_matmul` | `i64(i64, pointer)` | `linked C module` |"));
        assert!(docs.contains("lume-bootstrap header manifest.toml"));
    }
}
