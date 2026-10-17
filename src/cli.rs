use crate::checker::Checker;
use crate::codegen::Codegen;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::ast::{Expr, Item};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy)]
pub struct Opts {
    pub fastmath: bool,
    pub native: bool,
    pub lto: bool,
    pub opt: u8,
}

impl Default for Opts {
    fn default() -> Self {
        Self { fastmath: false, native: false, lto: false, opt: 3 }
    }
}

pub fn build(file: &str, opts: &Opts) {
    if !file.ends_with(".lume") {
        eprintln!("warning: 文件后缀不是 .lume: {}", file);
    }
    let program = compile_to_program(file);
    let c_code = compile_to_c(&program, opts);
    let c_path = output_c_path(file);
    std::fs::write(&c_path, &c_code).expect("写 C 文件失败");

    let out = output_bin_path(file);
    compile_c(&c_path, &out, opts);
    println!("// 编译成功: {}", out.display());
}

pub fn run(file: &str, opts: &Opts) {
    build(file, opts);
    let out = output_bin_path(file);
    let out_str = out.to_string_lossy();
    // 相对路径前面加 ./
    let cmd_path = if out_str.starts_with('/') || out_str.starts_with('.') {
        out_str.to_string()
    } else {
        format!("./{}", out_str)
    };
    let status = Command::new(&cmd_path)
        .status()
        .expect("运行失败");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

// ── 内部 ──

fn output_c_path(file: &str) -> PathBuf {
    let p = Path::new(file);
    let stem = p.file_stem().unwrap().to_string_lossy().to_string();
    PathBuf::from(format!("{}.c", stem))
}

fn output_bin_path(file: &str) -> PathBuf {
    let p = Path::new(file);
    let stem = p.file_stem().unwrap().to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    return PathBuf::from(format!("{}.exe", stem));
    #[cfg(not(target_os = "windows"))]
    return PathBuf::from(stem);
}

pub fn compile_to_program(file: &str) -> crate::ast::Program {
    let src = std::fs::read_to_string(file)
        .unwrap_or_else(|e| {
            eprintln!("error: 读文件失败 {}: {}", file, e);
            std::process::exit(1);
        });

    let mut lexer = Lexer::new(&src);
    let tokens = lexer.tokenize().unwrap_or_else(|e| {
        eprintln!("lex error: {}", e);
        std::process::exit(1);
    });

    let mut parser = Parser::new(tokens);
    let program = parser.parse_program().unwrap_or_else(|e| {
        eprintln!("parse error: {}", e);
        std::process::exit(1);
    });

    let mut checker = Checker::new();
    if let Err(errors) = checker.check(&program) {
        for e in &errors {
            eprintln!("error: {}", e.msg);
            if let Some(h) = &e.hint {
                eprintln!("  help: {}", h);
            }
        }
        eprintln!("// 类型检查失败，共 {} 个错误", errors.len());
        std::process::exit(1);
    }
    
    let mut program = program;
    crate::optimize::optimize(&mut program);   // ← 新增
    let opt_level = match std::env::var("LUME_OPT").as_deref() {
        Ok("safe") => crate::optimize::Level::Safe,
        _ => crate::optimize::Level::Aggressive,
    };
    crate::optimize::optimize_with(&mut program, opt_level);
    
    program
}

pub fn compile_to_c(program: &crate::ast::Program, opts: &Opts) -> String {
    let mut precomputed: HashMap<String, Vec<i64>> = HashMap::new();

    for item in &program.items {
        if let Item::Fn(f) = item {
            if let Some(d) = f.decorators.iter().find(|d| d.name == "precompute") {
                let (start, end) = match (d.args.get(0), d.args.get(1)) {
                    (Some(Expr::Int(s)), Some(Expr::Int(e))) => (*s, *e),
                    _ => continue,
                };
                match compute_precompute_table_cached(f, start, end, opts) {
                    Ok((table, from_cache)) => {
                        let tag = if from_cache { "缓存" } else { "新算" };
                        eprintln!("// 预计算 {}: {}..={} ({})", f.name, start, end, tag);
                        precomputed.insert(f.name.clone(), table);
                    }
                    Err(e) => eprintln!("// 预计算失败 {}: {}", f.name, e),
                }
            }
        }
    }

    let mut codegen = Codegen::new();
    for (name, table) in precomputed {
        codegen.set_precomputed(name, table);
    }
    codegen.generate(program)
}

fn compile_c(src: &Path, out: &Path, opts: &Opts) {
    let mut cmd = Command::new("clang");
    cmd.arg(src).arg("-o").arg(out);
    // 默认 -O3：比 -O2 更激进地展开和向量化，数值循环收益明显
    cmd.arg(format!("-O{}", opts.opt));

    // 针对本机 CPU 生成向量指令（AVX2 等），数值密集的循环收益明显
    if opts.native {
        cmd.arg("-march=native");
    }

    // 单文件程序，LTO 收益不大，但能再省掉一层冗余
    if opts.lto {
        cmd.arg("-flto");
    }

    #[cfg(not(target_os = "windows"))]
    {
        cmd.arg("-lm");
        cmd.arg("-lpthread");
    }

    // 数学函数不再写 errno。这样 sqrt/pow 之类才是真正的纯函数，
    // 编译器证明过的 LUME_PURE 标注才站得住
    cmd.arg("-fno-math-errno");

    if opts.fastmath {
        cmd.arg("-ffast-math");
    }

    let status = cmd.status().unwrap_or_else(|e| {
        eprintln!("error: 找不到 clang: {}", e);
        std::process::exit(1);
    });
    if !status.success() {
        eprintln!("clang 编译失败");
        std::process::exit(1);
    }
}

// ── 预计算（从原 main.rs 搬过来）──

// 对整份预计算 C 源码做 hash。
// 这样函数名、参数、函数体、返回类型、范围全都参与，
// 不会出现「改了代码却命中旧缓存」的情况。
fn hash_source(src: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    format!("{:x}", h.finish())
}

fn precompute_bin_path(f: &crate::ast::FnDecl) -> String {
    #[cfg(target_os = "windows")]
    return format!("_precompute_{}.exe", f.name);
    #[cfg(not(target_os = "windows"))]
    return format!("_precompute_{}", f.name);
}

fn compute_precompute_table_cached(
    f: &crate::ast::FnDecl,
    start: i64,
    end: i64,
    opts: &Opts,
) -> Result<(Vec<i64>, bool), String> {
    // 先把预计算程序生成出来，再用它的源码做缓存键
    let mut cg = Codegen::new();
    let c_code = cg.generate_precompute_program(f, start, end);
    let hash = hash_source(&c_code);
    let cache_dir = ".lume_cache";
    let cache_path = format!("{}/precompute_{}.bin", cache_dir, hash);

    if let Ok(data) = std::fs::read(&cache_path) {
        if !data.is_empty() && data.len() % 8 == 0 {
            let mut table = Vec::with_capacity(data.len() / 8);
            for chunk in data.chunks_exact(8) {
                let bytes: [u8; 8] = chunk.try_into().unwrap();
                table.push(i64::from_le_bytes(bytes));
            }
            return Ok((table, true));
        }
    }

    let table = compute_precompute_table(&c_code, f, opts)?;

    let _ = std::fs::create_dir_all(cache_dir);
    let mut bytes = Vec::with_capacity(table.len() * 8);
    for v in &table {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let _ = std::fs::write(&cache_path, &bytes);
    Ok((table, false))
}

fn compute_precompute_table(
    c_code: &str,
    f: &crate::ast::FnDecl,
    opts: &Opts,
) -> Result<Vec<i64>, String> {
    let tmp_c = format!("_precompute_{}.c", f.name);
    let tmp_bin = precompute_bin_path(f);

    std::fs::write(&tmp_c, c_code).map_err(|e| format!("写失败: {}", e))?;

    let mut cmd = Command::new("clang");
    cmd.arg(&tmp_c).arg("-o").arg(&tmp_bin)
        .arg(format!("-O{}", opts.opt));

    if opts.native {
        cmd.arg("-march=native");
    }

    #[cfg(not(target_os = "windows"))]
    {
        cmd.arg("-fopenmp");
        cmd.arg("-lm");
        cmd.arg("-lpthread");
    }

    // 预计算程序只读表、不算 errno；关掉 errno 让 sqrt/pow 保持纯函数
    cmd.arg("-fno-math-errno");
    if opts.fastmath {
        cmd.arg("-ffast-math");
    }

    let status = cmd.status().map_err(|e| format!("clang: {}", e))?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp_c);
        return Err("clang 失败".to_string());
    }

    let output = Command::new(format!("./{}", tmp_bin))
        .output()
        .map_err(|e| format!("运行: {}", e))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp_c);
        let _ = std::fs::remove_file(&tmp_bin);
        return Err("退出码非零".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut table = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        match line.parse::<i64>() {
            Ok(n) => table.push(n),
            Err(_) => return Err(format!("无法解析: {}", line)),
        }
    }

    let _ = std::fs::remove_file(&tmp_c);
    let _ = std::fs::remove_file(&tmp_bin);
    Ok(table)
}