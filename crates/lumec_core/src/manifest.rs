//! Minimal manifest support for Lume projects.
//!
//! This is deliberately a small, dependency-free TOML subset: it understands
//! `[project]`, `[[module]]`, `[ffi.extern]` and `[ffi.export]`, which is all
//! the bootstrap build graph needs today. Unknown keys are ignored so that
//! future fields can be added without breaking this parser.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ffi::{FfiError, FfiSignature, FfiType};

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDecl {
    pub path: PathBuf,
    pub lang: String,
    pub cflags: Vec<String>,
    pub export: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternDecl {
    pub signature: FfiSignature,
    pub signature_text: String,
    /// Explicit shared library path. If absent, the build graph looks for a C
    /// module that exports this symbol and loads its generated shared library.
    pub library: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub path: PathBuf,
    pub project_dir: PathBuf,
    pub name: Option<String>,
    pub build_mode: Option<String>,
    pub modules: Vec<ModuleDecl>,
    pub externs: HashMap<String, ExternDecl>,
    pub exports: Vec<String>,
}

impl Manifest {
    pub fn parse(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read manifest {}: {error}", path.display()))?;
        let project_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let mut manifest = Self {
            path: path.clone(),
            project_dir,
            name: None,
            build_mode: None,
            modules: Vec::new(),
            externs: HashMap::new(),
            exports: Vec::new(),
        };

        enum Section {
            None,
            Project,
            Module(usize),
            FfiExtern,
            FfiExport,
        }
        let mut section = Section::None;

        for (index, raw) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = strip_toml_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with("[[") && line.ends_with("]]") {
                let name = line[2..line.len() - 2].trim();
                if name != "module" {
                    return Err(manifest_error(
                        &path,
                        line_number,
                        "unsupported array table",
                    ));
                }
                manifest.modules.push(ModuleDecl {
                    path: manifest.project_dir.join("main.lume"),
                    lang: String::new(),
                    cflags: Vec::new(),
                    export: Vec::new(),
                });
                section = Section::Module(manifest.modules.len() - 1);
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let name = line[1..line.len() - 1].trim();
                section = match name {
                    "project" => Section::Project,
                    "ffi.extern" => Section::FfiExtern,
                    "ffi.export" => Section::FfiExport,
                    _ => Section::None,
                };
                continue;
            }
            let Some((key, raw_value)) = split_key_value(line) else {
                return Err(manifest_error(&path, line_number, "expected key = value"));
            };
            let key = key.trim();
            let value = raw_value.trim();

            match section {
                Section::Project => match key {
                    "name" => manifest.name = Some(parse_string(value, &path, line_number)?),
                    "build_mode" => {
                        manifest.build_mode = Some(parse_string(value, &path, line_number)?)
                    }
                    _ => {}
                },
                Section::Module(position) => {
                    let module = &mut manifest.modules[position];
                    match key {
                        "path" => {
                            module.path =
                                manifest
                                    .project_dir
                                    .join(parse_string(value, &path, line_number)?)
                        }
                        "lang" => module.lang = parse_string(value, &path, line_number)?,
                        "cflags" => module.cflags = parse_string_array(value, &path, line_number)?,
                        "export" => module.export = parse_string_array(value, &path, line_number)?,
                        _ => {}
                    }
                }
                Section::FfiExtern => {
                    let table = parse_inline_table(value, &path, line_number)?;
                    let (signature, signature_text) = if let Some(spec) = table.get("signature") {
                        let spec = parse_string(spec, &path, line_number)?;
                        let signature = FfiSignature::parse(&spec).map_err(|error| {
                            manifest_error(&path, line_number, &error.to_string())
                        })?;
                        (signature, spec)
                    } else {
                        let params = table
                            .get("params")
                            .ok_or_else(|| {
                                manifest_error(&path, line_number, "extern is missing `params`")
                            })?
                            .clone();
                        let display_params = parse_string_array(&params, &path, line_number)?;
                        let param_types = display_params
                            .iter()
                            .map(|ty| manifest_ffi_type(ty, &path, line_number))
                            .collect::<Result<Vec<_>, _>>()?;
                        let result = table
                            .get("ret")
                            .ok_or_else(|| {
                                manifest_error(&path, line_number, "extern is missing `ret`")
                            })?
                            .clone();
                        let result = manifest_ffi_type(
                            &parse_string(&result, &path, line_number)?,
                            &path,
                            line_number,
                        )?;
                        let signature =
                            FfiSignature::new(result.clone(), param_types).map_err(|error| {
                                manifest_error(&path, line_number, &error.to_string())
                            })?;
                        let signature_text =
                            format!("{}({})", c_type_name(result), display_params.join(", "));
                        (signature, signature_text)
                    };
                    let library = table
                        .get("library")
                        .map(|value| parse_string(value, &path, line_number))
                        .transpose()?;
                    manifest.externs.insert(
                        key.to_string(),
                        ExternDecl {
                            signature,
                            signature_text,
                            library,
                        },
                    );
                }
                Section::FfiExport => {
                    if key == "export" || key == "symbols" {
                        manifest
                            .exports
                            .extend(parse_string_array(value, &path, line_number)?);
                    } else if value.eq_ignore_ascii_case("true") {
                        manifest.exports.push(key.to_string());
                    }
                }
                Section::None => {}
            }
        }

        if manifest.modules.is_empty() {
            return Err(format!(
                "{}: manifest has no [[module]] entries",
                path.display()
            ));
        }
        Ok(manifest)
    }

    pub fn lume_modules(&self) -> impl Iterator<Item = &ModuleDecl> {
        self.modules.iter().filter(|module| module.lang == "lume")
    }

    pub fn c_modules(&self) -> impl Iterator<Item = &ModuleDecl> {
        self.modules.iter().filter(|module| module.lang == "c")
    }
}

fn manifest_error(path: &Path, line: usize, message: &str) -> String {
    format!("{}:{line}: {message}", path.display())
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_quote = false;
    for (index, character) in line.char_indices() {
        match character {
            '"' => in_quote = !in_quote,
            '#' if !in_quote => return &line[..index],
            _ => {}
        }
    }
    line
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let mut in_quote = false;
    for (index, character) in line.char_indices() {
        match character {
            '"' => in_quote = !in_quote,
            '=' if !in_quote => return Some((&line[..index], &line[index + 1..])),
            _ => {}
        }
    }
    None
}

fn parse_string(value: &str, path: &Path, line: usize) -> Result<String, String> {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        Ok(value[1..value.len() - 1].to_string())
    } else {
        Err(manifest_error(path, line, "expected a quoted string"))
    }
}

fn parse_string_array(value: &str, path: &Path, line: usize) -> Result<Vec<String>, String> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err(manifest_error(path, line, "expected a string array"));
    }
    let body = value[1..value.len() - 1].trim();
    if body.is_empty() {
        return Ok(Vec::new());
    }
    body.split(',')
        .map(|entry| parse_string(entry.trim(), path, line))
        .collect()
}

fn parse_inline_table(
    value: &str,
    path: &Path,
    line: usize,
) -> Result<HashMap<String, String>, String> {
    let value = value.trim();
    if !value.starts_with('{') || !value.ends_with('}') {
        return Err(manifest_error(path, line, "expected an inline table"));
    }
    let body = value[1..value.len() - 1].trim();
    if body.is_empty() {
        return Ok(HashMap::new());
    }
    let mut entries = HashMap::new();
    for part in split_top_level(body, ',') {
        let Some((key, value)) = split_key_value(part.trim()) else {
            return Err(manifest_error(
                path,
                line,
                "expected key = value in inline table",
            ));
        };
        entries.insert(key.trim().to_string(), value.trim().to_string());
    }
    Ok(entries)
}

fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quote = false;
    let mut depth = 0usize;
    for (index, character) in text.char_indices() {
        match character {
            '"' => in_quote = !in_quote,
            '[' | '{' if !in_quote => depth += 1,
            ']' | '}' if !in_quote => depth = depth.saturating_sub(1),
            c if c == separator && !in_quote && depth == 0 => {
                parts.push(&text[start..index]);
                start = index + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn manifest_ffi_type(name: &str, path: &Path, line: usize) -> Result<FfiType, String> {
    let name = name.trim();
    if let Some(inner) = name.strip_prefix('*') {
        if inner.trim().is_empty() {
            return Err(manifest_error(
                path,
                line,
                "pointer type is missing a pointee",
            ));
        }
        return Ok(FfiType::Pointer);
    }
    FfiType::parse(name).map_err(|error: FfiError| manifest_error(path, line, &error.to_string()))
}

fn c_type_name(ty: FfiType) -> &'static str {
    match ty {
        FfiType::Void => "void",
        FfiType::I8 => "i8",
        FfiType::I16 => "i16",
        FfiType::I32 => "i32",
        FfiType::I64 => "i64",
        FfiType::U8 => "u8",
        FfiType::U16 => "u16",
        FfiType::U32 => "u32",
        FfiType::U64 => "u64",
        FfiType::F32 => "f32",
        FfiType::F64 => "f64",
        FfiType::Pointer => "pointer",
        FfiType::Struct(_) => "struct",
        FfiType::Union(_) => "union",
    }
}

/// Maps a Lume annotation to a generated C type.
pub fn lume_c_type(annotation: Option<&str>) -> Result<&'static str, String> {
    match annotation.unwrap_or("any").trim() {
        "int" | "i64" | "isize" => Ok("int64_t"),
        "uint" | "u64" | "usize" => Ok("uint64_t"),
        "u8" | "bool" => Ok("uint8_t"),
        "u16" => Ok("uint16_t"),
        "u32" => Ok("uint32_t"),
        "i8" => Ok("int8_t"),
        "i16" => Ok("int16_t"),
        "i32" => Ok("int32_t"),
        "float" | "f64" => Ok("double"),
        "f32" => Ok("float"),
        "string" | "str" => Ok("const char *"),
        "unit" | "()" => Ok("void"),
        other => Err(format!("type `{other}` has no C ABI mapping")),
    }
}

pub fn ffi_c_type(ty: &FfiType) -> String {
    match ty {
        FfiType::Void => "void".into(),
        FfiType::I8 => "int8_t".into(),
        FfiType::I16 => "int16_t".into(),
        FfiType::I32 => "int32_t".into(),
        FfiType::I64 => "int64_t".into(),
        FfiType::U8 => "uint8_t".into(),
        FfiType::U16 => "uint16_t".into(),
        FfiType::U32 => "uint32_t".into(),
        FfiType::U64 => "uint64_t".into(),
        FfiType::F32 => "float".into(),
        FfiType::F64 => "double".into(),
        FfiType::Pointer => "void *".into(),
        FfiType::Struct(fields) => format!(
            "struct {{ {} }}",
            fields
                .iter()
                .enumerate()
                .map(|(index, field)| format!("{} field{index};", ffi_c_type(field)))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        FfiType::Union(members) => format!(
            "union {{ {} }}",
            members
                .iter()
                .enumerate()
                .map(|(index, member)| format!("{} member{index};", ffi_c_type(member)))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_demo_manifest_subset() {
        let manifest = Manifest::parse(Path::new("../../examples/demo_c_interop/manifest.toml"))
            .expect("demo manifest parses");
        assert_eq!(manifest.name.as_deref(), Some("demo_c_interop"));
        assert_eq!(manifest.modules.len(), 2);
        assert_eq!(manifest.modules[0].lang, "lume");
        assert_eq!(manifest.modules[1].lang, "c");
        let extern_decl = manifest.externs.get("c_matmul").expect("extern is parsed");
        assert_eq!(
            extern_decl.signature,
            FfiSignature::new(FfiType::I64, vec![FfiType::I64, FfiType::Pointer]).unwrap()
        );
    }

    #[test]
    fn parses_signature_inline_table_and_export_section() {
        let manifest = Manifest::parse(Path::new("../../examples/demo_c_interop/manifest.toml"))
            .expect("demo manifest parses");
        assert!(manifest.modules[1].export.contains(&"c_matmul".to_string()));
        assert_eq!(lume_c_type(Some("int")).unwrap(), "int64_t");
    }
}
