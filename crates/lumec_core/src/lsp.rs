use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{Value as JsonValue, json};

use crate::parser::{FuncDef, parse_source};

const KEYWORDS: &[&str] = &[
    "fn", "ret", "if", "else", "while", "match", "case", "import", "and", "or", "not", "is",
    "true", "false", "end",
];
const TYPES: &[&str] = &[
    "int", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "float", "f32", "f64",
    "string", "str", "bool", "pointer", "unit", "void", "any",
];
const BUILTINS: &[(&str, &str)] = &[
    ("print", "print(value) -> unit"),
    ("len", "len(value) -> int"),
    ("is_alpha", "is_alpha(value:string) -> bool"),
    ("is_digit", "is_digit(value:string) -> bool"),
    ("is_alphanumeric", "is_alphanumeric(value:string) -> bool"),
    ("is_whitespace", "is_whitespace(value:string) -> bool"),
    ("is_newline", "is_newline(value:string) -> bool"),
    (
        "is_identifier_start",
        "is_identifier_start(value:string) -> bool",
    ),
    (
        "is_identifier_continue",
        "is_identifier_continue(value:string) -> bool",
    ),
    ("now_ms", "now_ms() -> int"),
    ("mono_ms", "mono_ms() -> int"),
    ("u8", "u8(value:int) -> u8"),
    ("u32", "u32(value:int) -> u32"),
    ("buf_new", "buf_new(size) -> buffer"),
    ("buf_len", "buf_len(buffer) -> int"),
    ("buf_get_u8", "buf_get_u8(buffer, offset) -> u8"),
    ("buf_set_u8", "buf_set_u8(buffer, offset, value) -> unit"),
    ("buf_get_u32", "buf_get_u32(buffer, offset) -> u32"),
    ("buf_set_u32", "buf_set_u32(buffer, offset, value) -> unit"),
    (
        "memcpy",
        "memcpy(dst, dst_offset, src, src_offset, size) -> unit",
    ),
    (
        "memmove",
        "memmove(dst, dst_offset, src, src_offset, size) -> unit",
    ),
    ("str_to_buf", "str_to_buf(string) -> buffer"),
    ("buf_to_str", "buf_to_str(buffer) -> string"),
    ("ffi_unsafe_load", "ffi_unsafe_load(path) -> library"),
    ("ffi_symbol", "ffi_symbol(library, name) -> symbol"),
    (
        "ffi_unsafe_call",
        "ffi_unsafe_call(symbol, signature, args) -> value",
    ),
    (
        "ffi_callback",
        "ffi_callback(signature, function) -> callback",
    ),
    (
        "ffi_callback_error",
        "ffi_callback_error(callback) -> string",
    ),
    ("ffi_null", "ffi_null() -> pointer"),
    ("ffi_buffer", "ffi_buffer(size, alignment) -> buffer"),
    (
        "ffi_buffer_read",
        "ffi_buffer_read(buffer, offset, type) -> value",
    ),
    (
        "ffi_buffer_write",
        "ffi_buffer_write(buffer, offset, type, value) -> unit",
    ),
    (
        "ffi_buffer_write_cstr",
        "ffi_buffer_write_cstr(buffer, offset, value) -> unit",
    ),
    (
        "ffi_buffer_read_cstr",
        "ffi_buffer_read_cstr(buffer, offset, max_length) -> string",
    ),
    ("ffi_struct", "ffi_struct(type, fields) -> aggregate"),
    (
        "ffi_export_ptr",
        "ffi_export_ptr(name, signature) -> callback",
    ),
    ("sin", "sin(x:float) -> float"),
    ("cos", "cos(x:float) -> float"),
    ("tan", "tan(x:float) -> float"),
    ("asin", "asin(x:float) -> float"),
    ("acos", "acos(x:float) -> float"),
    ("atan", "atan(x:float) -> float"),
    ("sinh", "sinh(x:float) -> float"),
    ("cosh", "cosh(x:float) -> float"),
    ("tanh", "tanh(x:float) -> float"),
    ("asinh", "asinh(x:float) -> float"),
    ("acosh", "acosh(x:float) -> float"),
    ("atanh", "atanh(x:float) -> float"),
    ("exp", "exp(x:float) -> float"),
    ("exp2", "exp2(x:float) -> float"),
    ("expm1", "expm1(x:float) -> float"),
    ("ln", "ln(x:float) -> float"),
    ("sqrt", "sqrt(x:float) -> float"),
    ("cbrt", "cbrt(x:float) -> float"),
    ("floor", "floor(x:float) -> float"),
    ("ceil", "ceil(x:float) -> float"),
    ("round", "round(x:float) -> float"),
    ("trunc", "trunc(x:float) -> float"),
    ("abs", "abs(x:float) -> float"),
    ("log2", "log2(x:float) -> float"),
    ("log10", "log10(x:float) -> float"),
    ("log1p", "log1p(x:float) -> float"),
    ("pow", "pow(x:float, y:float) -> float"),
    ("atan2", "atan2(x:float, y:float) -> float"),
    ("hypot", "hypot(x:float, y:float) -> float"),
    ("fmod", "fmod(x:float, y:float) -> float"),
    ("remainder", "remainder(x:float, y:float) -> float"),
    ("copysign", "copysign(x:float, y:float) -> float"),
    ("sleef_sin_u10", "sleef_sin_u10(x:float) -> float"),
    ("sleef_sin_u35", "sleef_sin_u35(x:float) -> float"),
    ("sleef_cos_u10", "sleef_cos_u10(x:float) -> float"),
    ("sleef_cos_u35", "sleef_cos_u35(x:float) -> float"),
    ("sleef_tan_u10", "sleef_tan_u10(x:float) -> float"),
    ("sleef_exp_u10", "sleef_exp_u10(x:float) -> float"),
    ("sleef_log_u10", "sleef_log_u10(x:float) -> float"),
    ("sleef_sqrt_u05", "sleef_sqrt_u05(x:float) -> float"),
    ("sleef_pow_u10", "sleef_pow_u10(x:float, y:float) -> float"),
    ("sleef_pow_u35", "sleef_pow_u35(x:float, y:float) -> float"),
    ("llvm_sin", "llvm_sin(x:float) -> float"),
    ("llvm_cos", "llvm_cos(x:float) -> float"),
    ("llvm_exp", "llvm_exp(x:float) -> float"),
    ("llvm_log", "llvm_log(x:float) -> float"),
    ("llvm_sqrt", "llvm_sqrt(x:float) -> float"),
    ("trace", "trace(message) -> unit"),
    ("log", "log(x:float) -> float"),
    ("log_trace", "log_trace(message) -> unit"),
    ("log_debug", "log_debug(message) -> unit"),
    ("log_info", "log_info(message) -> unit"),
    ("log_warn", "log_warn(message) -> unit"),
    ("log_error", "log_error(message) -> unit"),
    ("log_at", "log_at(level, message) -> unit"),
    (
        "ffi_struct_get",
        "ffi_struct_get(aggregate, index) -> value",
    ),
    ("debug", "debug(message) -> unit"),
    ("info", "info(message) -> unit"),
    ("warn", "warn(message) -> unit"),
    ("error", "error(message) -> unit"),
];

#[derive(Default)]
struct ServerState {
    documents: HashMap<String, String>,
}

pub fn serve_stdio() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let mut state = ServerState::default();

    while let Some(message) = read_message(&mut input)? {
        if message.get("method").and_then(JsonValue::as_str) == Some("exit") {
            break;
        }
        for response in handle_message(message, &mut state) {
            write_message(&mut output, &response)?;
        }
    }
    Ok(())
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<JsonValue>> {
    let mut content_length = None;
    let mut saw_header = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return if saw_header {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF in LSP message headers",
                ))
            } else {
                Ok(None)
            };
        }
        saw_header = true;
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = Some(value.trim().parse::<usize>().map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid Content-Length: {error}"),
                )
            })?);
        }
    }
    let length = content_length.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "LSP message has no Content-Length",
        )
    })?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn write_message(writer: &mut impl Write, message: &JsonValue) -> io::Result<()> {
    let body = serde_json::to_vec(message)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn handle_message(message: JsonValue, state: &mut ServerState) -> Vec<JsonValue> {
    let id = message.get("id").cloned();
    let Some(method) = message.get("method").and_then(JsonValue::as_str) else {
        return id
            .map(|id| vec![response_error(id, -32600, "invalid LSP request")])
            .unwrap_or_default();
    };
    let params = message.get("params").cloned().unwrap_or(JsonValue::Null);

    match method {
        "initialize" => id
            .map(|id| {
                vec![json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "capabilities": {
                            "textDocumentSync": 1,
                            "completionProvider": { "triggerCharacters": [".", ":"] },
                            "hoverProvider": true
                        },
                        "serverInfo": { "name": "lume-language-server", "version": env!("CARGO_PKG_VERSION") }
                    }
                })]
            })
            .unwrap_or_default(),
        "initialized" | "$/setTrace" => Vec::new(),
        "textDocument/didOpen" => {
            let Some(document) = params.get("textDocument") else {
                return Vec::new();
            };
            let Some(uri) = document.get("uri").and_then(JsonValue::as_str) else {
                return Vec::new();
            };
            let Some(text) = document.get("text").and_then(JsonValue::as_str) else {
                return Vec::new();
            };
            state.documents.insert(uri.to_string(), text.to_string());
            vec![publish_diagnostics(uri, text)]
        }
        "textDocument/didChange" => {
            let Some(uri) = params
                .get("textDocument")
                .and_then(|document| document.get("uri"))
                .and_then(JsonValue::as_str)
            else {
                return Vec::new();
            };
            let Some(text) = params
                .get("contentChanges")
                .and_then(JsonValue::as_array)
                .and_then(|changes| changes.last())
                .and_then(|change| change.get("text"))
                .and_then(JsonValue::as_str)
            else {
                return Vec::new();
            };
            state.documents.insert(uri.to_string(), text.to_string());
            vec![publish_diagnostics(uri, text)]
        }
        "textDocument/didClose" => {
            let Some(uri) = params
                .get("textDocument")
                .and_then(|document| document.get("uri"))
                .and_then(JsonValue::as_str)
            else {
                return Vec::new();
            };
            state.documents.remove(uri);
            vec![json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": { "uri": uri, "diagnostics": [] }
            })]
        }
        "textDocument/completion" => id
            .map(|id| {
                let uri = document_uri(&params);
                let prefix = completion_prefix(
                    state.documents.get(uri).map(String::as_str).unwrap_or_default(),
                    params.pointer("/position/line").and_then(JsonValue::as_u64).unwrap_or(0) as usize,
                    params.pointer("/position/character").and_then(JsonValue::as_u64).unwrap_or(0) as usize,
                );
                vec![json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "isIncomplete": false,
                        "items": completion_items(
                            state.documents.get(uri),
                            &prefix,
                            params.pointer("/position/line").and_then(JsonValue::as_u64).unwrap_or(0) as usize,
                            params.pointer("/position/character").and_then(JsonValue::as_u64).unwrap_or(0) as usize
                        )
                    }
                })]
            })
            .unwrap_or_default(),
        "textDocument/hover" => id
            .map(|id| {
                let uri = document_uri(&params);
                let text = state.documents.get(uri).map(String::as_str).unwrap_or_default();
                let line_number = params.pointer("/position/line").and_then(JsonValue::as_u64).unwrap_or(0) as usize;
                let character = params.pointer("/position/character").and_then(JsonValue::as_u64).unwrap_or(0) as usize;
                let contents = hover_content(text, line_number, character);
                let result = contents.map(|contents| json!({ "contents": { "kind": "markdown", "value": contents } }));
                vec![json!({ "jsonrpc": "2.0", "id": id, "result": result })]
            })
            .unwrap_or_default(),
        "shutdown" => id
            .map(|id| vec![json!({ "jsonrpc": "2.0", "id": id, "result": null })])
            .unwrap_or_default(),
        "exit" => Vec::new(),
        _ => id
            .map(|id| vec![response_error(id, -32601, "method not found")])
            .unwrap_or_default(),
    }
}

fn document_uri(params: &JsonValue) -> &str {
    params
        .pointer("/textDocument/uri")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
}

fn response_error(id: JsonValue, code: i32, message: &str) -> JsonValue {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn publish_diagnostics(uri: &str, source: &str) -> JsonValue {
    let mut diagnostics = Vec::new();
    match parse_source(source) {
        Err(error) => {
            let line = error.line.saturating_sub(1);
            let character = line_text(source, line).map(utf16_len).unwrap_or(0);
            diagnostics.push(json!({
                "range": { "start": { "line": line, "character": 0 }, "end": { "line": line, "character": character } },
                "severity": 1,
                "source": "lume",
                "message": error.message
            }));
        }
        Ok(functions) => {
            if let Err(error) = crate::typecheck::check_program(&functions) {
                let function_name = error.split_once(": ").map(|(name, _)| name);
                let line = function_name
                    .and_then(|name| {
                        source.lines().position(|source_line| {
                            let declaration = source_line.trim_start();
                            declaration.starts_with(&format!("fn {name}("))
                                || declaration.starts_with(&format!("fn {name}<"))
                        })
                    })
                    .unwrap_or(0);
                let character = line_text(source, line).map(utf16_len).unwrap_or(0);
                diagnostics.push(json!({
                    "range": { "start": { "line": line, "character": 0 }, "end": { "line": line, "character": character } },
                    "severity": 1,
                    "source": "lume",
                    "message": error
                }));
            }
        }
    }
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": diagnostics }
    })
}

fn completion_prefix(source: &str, line: usize, character: usize) -> String {
    let Some(line) = line_text(source, line) else {
        return String::new();
    };
    let prefix = take_utf16(line, character);
    prefix
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

fn completion_items(
    source: Option<&String>,
    prefix: &str,
    line: usize,
    character: usize,
) -> Vec<JsonValue> {
    let mut items = Vec::new();
    let mut push = |label: &str, kind: u8, detail: &str, insert_text: Option<String>| {
        if label.starts_with(prefix) {
            let mut item = json!({ "label": label, "kind": kind, "detail": detail });
            if let Some(insert_text) = insert_text {
                item["insertText"] = json!(insert_text);
                item["insertTextFormat"] = json!(2);
            }
            items.push(item);
        }
    };
    for keyword in KEYWORDS {
        let snippet = match *keyword {
            "fn" => Some("fn ${1:name}(${2:arg}:${3:int}) -> ${4:int}\n$0\nend".into()),
            "if" => Some("if ${1:condition}\n$0\nend".into()),
            "while" => Some("while ${1:condition}\n$0\nend".into()),
            _ => None,
        };
        push(keyword, 14, "Lume keyword", snippet);
    }
    for ty in TYPES {
        push(ty, 25, "Lume type", None);
    }
    for (name, signature) in BUILTINS {
        push(name, 3, signature, Some(format!("{name}(${{1}})")));
    }
    if let Some(source) = source {
        let functions = parse_source(source)
            .map(|functions| functions.iter().map(function_signature).collect::<Vec<_>>())
            .unwrap_or_else(|_| {
                source
                    .lines()
                    .filter_map(|line| function_header(line).map(str::to_string))
                    .collect()
            });
        for signature in functions {
            if let Some(name) = function_name(&signature) {
                push(name, 3, &signature, Some(format!("{name}(${{1}})")));
            }
        }
        for (name, detail) in local_variables(source, line, character) {
            if name.starts_with(prefix) {
                items.push(json!({
                    "label": name,
                    "kind": 6,
                    "detail": detail,
                    "sortText": format!("0_{name}")
                }));
            }
        }
    }
    items
}

fn hover_content(source: &str, line_number: usize, character: usize) -> Option<String> {
    let line = line_text(source, line_number)?;
    let (word, _, _) = word_at(line, character)?;
    if let Some(signature) = BUILTINS
        .iter()
        .find_map(|(name, signature)| (*name == word).then_some(*signature))
    {
        return Some(format!(
            "```lume\n{signature}\n```\n\nLume built-in function."
        ));
    }
    if let Ok(functions) = parse_source(source) {
        if let Some(function) = functions.iter().find(|function| function.name == word) {
            return Some(format!(
                "```lume\n{}\n```\n\nDefined in this document.",
                function_signature(function)
            ));
        }
    } else if let Some(signature) = source.lines().find_map(|line| {
        function_header(line).filter(|header| function_name(header) == Some(word.as_str()))
    }) {
        return Some(format!(
            "```lume\n{signature}\n```\n\nDefined in this document."
        ));
    }
    let description = match word.as_str() {
        "fn" => "declare a function",
        "ret" => "return from the current function",
        "if" => "conditional block or block expression",
        "else" => "alternate conditional branch",
        "while" => "loop block or block expression",
        "match" => "match expression",
        "case" => "match arm pattern",
        "import" => "import a relative Lume module",
        "end" => "close a block",
        "and" => "short-circuit boolean and",
        "or" => "short-circuit boolean or",
        "not" => "boolean negation",
        "is" => "identity comparison",
        "true" | "false" => "boolean literal",
        value if TYPES.contains(&value) => return Some(format!("`{value}` — built-in Lume type.")),
        _ => return None,
    };
    Some(format!("`{word}` — {description}."))
}

fn function_signature(function: &FuncDef) -> String {
    let params = function
        .params
        .iter()
        .map(|param| match &param.ty {
            Some(ty) => format!("{}:{ty}", param.name),
            None => param.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    match &function.ret_ty {
        Some(ret) => format!("fn {}({params}) -> {ret}", function.name),
        None => format!("fn {}({params})", function.name),
    }
}

fn function_header(line: &str) -> Option<&str> {
    let header = line.trim().strip_prefix("fn ")?;
    let open = header.find('(')?;
    let name = header[..open].split('<').next()?.trim();
    (!name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '.'
        }))
    .then_some(line.trim())
}

fn function_name(signature: &str) -> Option<&str> {
    let signature = signature.trim().strip_prefix("fn ")?;
    let end = signature.find(['(', '<'])?;
    Some(signature[..end].trim())
}

fn local_variables(source: &str, cursor_line: usize, character: usize) -> Vec<(String, String)> {
    let mut variables = HashMap::new();
    let lines: Vec<_> = source.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let cursor_line = cursor_line.min(lines.len().saturating_sub(1));
    let Some(function_line) = lines[..=cursor_line]
        .iter()
        .rposition(|line| line.trim_start().starts_with("fn "))
    else {
        return Vec::new();
    };

    let header = lines[function_line].trim();
    if let (Some(open), Some(close)) = (header.find('('), header.rfind(')'))
        && open < close
    {
        for parameter in header[open + 1..close].split(',') {
            let mut parts = parameter.trim().splitn(2, ':');
            let name = parts.next().unwrap_or_default().trim();
            if valid_identifier(name) {
                let detail = parts
                    .next()
                    .map(|ty| format!("parameter: {}", ty.trim()))
                    .unwrap_or_else(|| "parameter".into());
                variables.insert(name.to_string(), detail);
            }
        }
    }

    for (index, line) in lines
        .iter()
        .enumerate()
        .take(cursor_line + 1)
        .skip(function_line + 1)
    {
        let line = if index == cursor_line {
            take_utf16(line, character)
        } else {
            line
        };
        let line = strip_lume_comment(line).trim();
        let Some((lhs, _)) = line
            .split_once(":=")
            .or_else(|| assignment_left_hand_side(line))
        else {
            continue;
        };
        let lhs = lhs.trim();
        let (name, ty) = lhs
            .split_once(':')
            .map(|(name, ty)| (name.trim(), Some(ty.trim())))
            .unwrap_or((lhs, None));
        if valid_identifier(name) {
            variables.insert(
                name.to_string(),
                ty.map(|ty| format!("variable: {ty}"))
                    .unwrap_or_else(|| "variable".into()),
            );
        }
    }
    let mut variables: Vec<_> = variables.into_iter().collect();
    variables.sort_by(|left, right| left.0.cmp(&right.0));
    variables
}

fn assignment_left_hand_side(line: &str) -> Option<(&str, &str)> {
    for (index, _) in line.match_indices('=') {
        if line.as_bytes().get(index + 1) == Some(&b'=')
            || matches!(
                index
                    .checked_sub(1)
                    .and_then(|previous| line.as_bytes().get(previous)),
                Some(b':' | b'=' | b'<' | b'>' | b'!')
            )
        {
            continue;
        }
        return Some((&line[..index], &line[index + 1..]));
    }
    None
}

fn strip_lume_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character == '#' && !quoted {
            return &line[..index];
        }
    }
    line
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(character) if character.is_ascii_alphabetic() || character == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn line_text(source: &str, line: usize) -> Option<&str> {
    source.lines().nth(line)
}

fn take_utf16(text: &str, units: usize) -> &str {
    let mut end = 0;
    let mut count = 0;
    for character in text.chars() {
        if count + character.len_utf16() > units {
            break;
        }
        count += character.len_utf16();
        end += character.len_utf8();
    }
    &text[..end]
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn word_at(line: &str, character: usize) -> Option<(String, usize, usize)> {
    let char_index = take_utf16(line, character).chars().count();
    let chars: Vec<char> = line.chars().collect();
    let mut start = char_index.min(chars.len());
    let mut end = start;
    while start > 0 && (chars[start - 1].is_ascii_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    if start == end {
        return None;
    }
    let word = chars[start..end].iter().collect();
    let start_units = chars[..start]
        .iter()
        .map(|character| character.len_utf16())
        .sum();
    let end_units = start_units
        + chars[start..end]
            .iter()
            .map(|character| character.len_utf16())
            .sum::<usize>();
    Some((word, start_units, end_units))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_parser_and_type_errors_with_lsp_ranges() {
        let syntax = publish_diagnostics("file:///test.lume", "fn main() -> int\n    ret (");
        let diagnostic = &syntax["params"]["diagnostics"][0];
        assert_eq!(diagnostic["range"]["start"]["line"], 1);
        assert_eq!(diagnostic["severity"], 1);

        let type_error =
            publish_diagnostics("file:///test.lume", "fn main() -> int\n    ret \"wrong\"\n");
        assert_eq!(
            type_error["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn completion_and_hover_include_local_functions_and_builtins() {
        let source = "fn answer(value:int) -> int\nret value\n\nfn main() -> int\nret answer(42)";
        let items = completion_items(Some(&source.to_string()), "ans", 4, 4);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["label"], "answer");
        let hover = hover_content(source, 0, 5).unwrap();
        assert!(hover.contains("fn answer(value:int) -> int"));
        assert!(
            hover_content(source, 4, 5)
                .unwrap()
                .contains("fn answer(value:int) -> int")
        );
        assert!(
            hover_content("fn main()\nret sqrt(1.0)", 1, 5)
                .unwrap()
                .contains("sqrt(x:float)")
        );
    }

    #[test]
    fn completion_includes_variables_declared_in_the_current_function() {
        let source = "fn main() -> int\n    amount:int = 1\n    ret am";
        let items = completion_items(Some(&source.to_string()), "am", 2, 10);
        assert!(
            items
                .iter()
                .any(|item| item["label"] == "amount" && item["kind"] == 6)
        );
    }

    #[test]
    fn handles_initialize_open_and_completion_requests() {
        let mut state = ServerState::default();
        let initialized = handle_message(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            &mut state,
        );
        assert_eq!(
            initialized[0]["result"]["capabilities"]["hoverProvider"],
            true
        );

        let source = "fn main() -> int\n    amount:int = 1\n    ret am";
        let opened = handle_message(
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": "file:///test.lume", "text": source } }
            }),
            &mut state,
        );
        assert_eq!(
            opened[0]["params"]["diagnostics"].as_array().unwrap().len(),
            1
        );

        let completion = handle_message(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "textDocument/completion",
                "params": {
                    "textDocument": { "uri": "file:///test.lume" },
                    "position": { "line": 2, "character": 10 }
                }
            }),
            &mut state,
        );
        assert!(
            completion[0]["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["label"] == "amount")
        );

        let changed = handle_message(
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": "file:///test.lume", "version": 2 },
                    "contentChanges": [{ "text": "fn main() -> int\n    amount:int=1\n    ret amount" }]
                }
            }),
            &mut state,
        );
        assert!(
            changed[0]["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn handles_unicode_positions_as_utf16_code_units() {
        assert_eq!(completion_prefix("fn main()\n    ret café", 1, 11), "caf");
        assert_eq!(word_at("😀answer", 7).unwrap().0, "answer");
    }

    #[test]
    fn parses_framed_messages() {
        let payload = br#"{"jsonrpc":"2.0","method":"initialized"}"#;
        let message = format!("Content-Length: {}\r\n\r\n", payload.len());
        let mut bytes = message.into_bytes();
        bytes.extend_from_slice(payload);
        let parsed = read_message(&mut io::Cursor::new(bytes)).unwrap().unwrap();
        assert_eq!(parsed["method"], "initialized");
    }
}
