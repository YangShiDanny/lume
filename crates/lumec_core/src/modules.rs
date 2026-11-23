use crate::parser::{FuncDef, parse_imports, parse_source};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn load_source_graph(entry: impl AsRef<Path>) -> Result<Vec<FuncDef>, String> {
    let mut visited = HashSet::new();
    let mut functions = Vec::new();
    load_module(entry.as_ref(), &mut visited, &mut functions)?;
    Ok(functions)
}

fn load_module(
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    functions: &mut Vec<FuncDef>,
) -> Result<(), String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve module `{}`: {error}", path.display()))?;
    if !visited.insert(path.clone()) {
        return Ok(());
    }

    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read module `{}`: {error}", path.display()))?;
    let imports = parse_imports(&source).map_err(|error| format!("{}: {error}", path.display()))?;
    for import in imports {
        let imported = resolve_import(&path, &import);
        load_module(&imported, visited, functions)?;
    }

    let parsed = parse_source(&source).map_err(|error| format!("{}: {error}", path.display()))?;
    functions.extend(parsed);
    Ok(())
}

fn resolve_import(importer: &Path, import: &str) -> PathBuf {
    let requested = Path::new(import);
    let requested = if requested.extension().is_some() {
        requested.to_path_buf()
    } else {
        requested.with_extension("lume")
    };
    importer
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(requested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn loads_relative_imports_once_and_detects_missing_modules() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("lume-modules-{unique}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("main.lume"),
            "import \"math\"\nimport \"math.lume\"\nfn main() -> int\nret twice(21)\n",
        )
        .unwrap();
        std::fs::write(
            root.join("math.lume"),
            "fn twice(n:int) -> int\nret n * 2\n",
        )
        .unwrap();

        let funcs = load_source_graph(root.join("main.lume")).unwrap();
        assert_eq!(funcs.len(), 2);
        crate::typecheck::check_program(&funcs).unwrap();
        let value = crate::evaluator::Evaluator::new(funcs)
            .call("main", vec![])
            .unwrap();
        assert!(matches!(value, crate::evaluator::Value::Int(42)));

        std::fs::write(
            root.join("broken.lume"),
            "import \"missing\"\nfn main()\nend\n",
        )
        .unwrap();
        let error = load_source_graph(root.join("broken.lume")).unwrap_err();
        assert!(error.contains("missing.lume"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
