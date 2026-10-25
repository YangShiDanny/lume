use crate::checker::Checker;
use crate::codegen::Codegen;
use crate::ast::{Expr, Item};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};

/// @precompute 的填表程序能不能用 OpenMP。
/// 0 = 还没试过，2 = libomp 不可用（以后都走串行）。
/// 串行填表只是编译期慢一点，生成的表完全一样。
static PRECOMPUTE_OPENMP: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy)]
pub struct Opts {
    pub fastmath: bool,
    pub native: bool,
    pub lto: bool,
    pub opt: u8,
    /// 关掉 a[i] 的运行期边界检查（默认开）。
    /// 只给已经手工验证过下标的热点内核用；单个函数可以写 @unchecked。
    pub unchecked: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self { fastmath: false, native: false, lto: false, opt: 3, unchecked: false }
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

pub fn run(file: &str, opts: &Opts, prog_args: &[String]) {
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
        .args(prog_args)
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

// ─────────────────────────────────────────────────────────────
// 多文件项目（模块系统）
//
// 设计：**文件名即模块名，零 import**。编译器自动把入口文件所在目录
// （含子目录）里的所有 .lume 当成同一个项目的模块合并。
//
// 三条边界规则，都是为了「别把别人的程序捡进来」：
//   1. 带 `fn main` 的文件是**可执行入口**，不参与别人的合并。
//      所以你把 tests_pipe.lume 和 main.lume 放同一目录也不会打架。
//   2. 子目录里只要有自己的入口文件，就算独立项目，整个子目录跳过。
//      所以 tests/multi/ 不会被仓库根目录的构建吞掉。
//   3. 隐藏目录（.git / .workbuddy）、target、node_modules 一律不扫。
//
// 合并顺序按路径排序，保证同一份源码每次生成的 C 完全一致。
//
// 库（`$LUME_PATH` / `~/.lume/lib`）不在这里处理 —— 见 `src/libpkg.rs`。
// 那边是**包 + 按符号需求自动加载**：库根下的一级子目录是一个包，包内
// 可以 `import 别的包` 声明依赖，只有被用到的包才进来。项目侧仍然是零 import。
// ─────────────────────────────────────────────────────────────

use crate::libpkg::{self, collect_lume_files, has_main, parse_file as parse_source};

/// 顶层名字的种类，只用于重复定义的报错措辞。
#[derive(Clone, Copy, PartialEq)]
enum SymKind {
    Fn,
    Struct,
    Enum,
    Const,
}

impl SymKind {
    fn label(self) -> &'static str {
        match self {
            SymKind::Fn => "函数",
            SymKind::Struct => "结构体",
            SymKind::Enum => "枚举",
            SymKind::Const => "常量",
        }
    }
}

pub(crate) struct Parsed {
    pub path: PathBuf,
    pub items: Vec<Item>,
}

/// 一个项目：入口 + 自动合并进来的同目录模块（**不含库**）。
/// `compile_to_program` 和 `lume libs` 共用这段扫描。
pub(crate) struct Project {
    pub entry_display: String,
    pub entry_abs: PathBuf,
    pub entry_items: Vec<Item>,
    pub modules: Vec<Parsed>,
}

pub(crate) fn scan_project(file: &str) -> Project {
    let entry = PathBuf::from(file);
    let entry_abs = std::fs::canonicalize(&entry).unwrap_or_else(|e| {
        eprintln!("error: 读文件失败 {}: {}", file, e);
        std::process::exit(1);
    });

    // 入口文件必须自己解析成功 —— 它的报错不容忍
    let entry_src = std::fs::read_to_string(&entry).unwrap_or_else(|e| {
        eprintln!("error: 读文件失败 {}: {}", file, e);
        std::process::exit(1);
    });
    let entry_items = parse_source(&entry, &entry_src).unwrap_or_else(|msg| {
        eprintln!("parse error: {}", msg);
        std::process::exit(1);
    });
    if !has_main(&entry_items) {
        eprintln!("error: 入口文件 {} 里没有 fn main", entry.display());
        eprintln!("  help: 入口文件必须定义 fn main（模块文件不需要）");
        std::process::exit(1);
    }

    let entry_dir = entry_abs
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // 1. 扫出候选文件（排序保证确定性）
    let mut cands: Vec<PathBuf> = Vec::new();
    collect_lume_files(&entry_dir, &mut cands);
    cands.sort();

    // 2. 逐个解析。非入口文件解析失败只警告并跳过 ——
    //    自动扫描会把整棵树都看一遍，一个写到一半的文件不该炸掉整个构建。
    let mut parsed: Vec<Parsed> = Vec::new();
    for p in &cands {
        let abs = match std::fs::canonicalize(p) {
            Ok(a) => a,
            Err(_) => continue,
        };
        if abs == entry_abs {
            continue; // 入口在下面单独处理
        }
        let src = match std::fs::read_to_string(&abs) {
            Ok(s) => s,
            Err(_) => continue,
        };
        match parse_source(&abs, &src) {
            Ok(items) => parsed.push(Parsed { path: abs, items }),
            Err(msg) => eprintln!(
                "warning: 跳过无法解析的文件（不参与本次构建）\n         {}",
                msg
            ),
        }
    }

    // 3. 子目录自带入口 → 独立项目，整个子树退出本次构建
    let mut project_dirs: Vec<PathBuf> = Vec::new();
    for f in &parsed {
        if has_main(&f.items) {
            if let Some(d) = f.path.parent() {
                if d != entry_dir.as_path() {
                    project_dirs.push(d.to_path_buf());
                }
            }
        }
    }

    let mut modules: Vec<Parsed> = Vec::new();
    for f in parsed {
        if has_main(&f.items) {
            continue; // 别的入口文件不合并
        }
        let inside_other_project = f
            .path
            .parent()
            .map(|d| {
                project_dirs
                    .iter()
                    .any(|p| d == p.as_path() || d.starts_with(p))
            })
            .unwrap_or(false);
        if inside_other_project {
            continue;
        }
        modules.push(f);
    }

    Project {
        entry_display: file.to_string(),
        entry_abs,
        entry_items,
        modules,
    }
}

/// `lume libs <file.lume>`：打印库搜索根、所有库包、依赖关系，
/// 以及本次构建**会真正加载**哪些包。排查「库调库」问题的入口。
pub fn list_libs(file: &str) {
    let p = scan_project(file);
    println!("项目: {}", p.entry_display);
    if !p.modules.is_empty() {
        println!("  同目录模块 {} 个:", p.modules.len());
        for m in &p.modules {
            println!("    {}", m.path.display());
        }
    }

    let roots = libpkg::lib_roots();
    println!("库搜索根:");
    if roots.is_empty() {
        println!("  （无）—— 设 $LUME_PATH 或建 ~/.lume/lib");
    } else {
        for r in &roots {
            println!("  {}", r.display());
        }
    }

    let mut sets: Vec<&[Item]> = Vec::with_capacity(p.modules.len() + 1);
    sets.push(&p.entry_items);
    for m in &p.modules {
        sets.push(&m.items);
    }
    let sum = libpkg::summarize(&sets);

    let sel = match libpkg::select(&sum) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("error: {}", msg);
            std::process::exit(1);
        }
    };
    let chosen: std::collections::HashSet<&str> =
        sel.loaded.iter().map(|(n, _, _)| n.as_str()).collect();

    let pkgs = libpkg::discover();
    println!("\n库包（共 {} 个）:", pkgs.len());
    if pkgs.is_empty() {
        println!("  （无）");
    }
    for pkg in &pkgs {
        let mark = if chosen.contains(pkg.name.as_str()) {
            "✔ 加载"
        } else {
            "  未用"
        };
        let kind = if pkg.loose { "散装" } else { "目录" };
        let deps = if pkg.deps.is_empty() {
            String::new()
        } else {
            format!("  依赖: {}", pkg.deps.join(", "))
        };
        println!(
            "  {} [{}] {}  {}{}",
            mark,
            kind,
            pkg.name,
            pkg.dir.display(),
            deps
        );
        let mut syms: Vec<&String> = pkg.provides.iter().collect();
        syms.sort();
        if !syms.is_empty() {
            let shown: Vec<&str> = syms.iter().take(8).map(|s| s.as_str()).collect();
            let more = if syms.len() > shown.len() {
                format!(" …（共 {} 个）", syms.len())
            } else {
                String::new()
            };
            println!("        提供: {}{}", shown.join(", "), more);
        }
    }
}

pub fn compile_to_program(file: &str) -> crate::ast::Program {
    let proj = scan_project(file);
    let entry_abs = proj.entry_abs;
    let entry_items = proj.entry_items;
    let mut modules = proj.modules;

    // 3.5 库：按符号需求自动加载（详见 libpkg.rs 文件头）。
    //
    // 项目里零 import 也能调库 —— 引用到的顶层名字会被拿去匹配库包，
    // 命中的包连同依赖闭包一起进来，没命中的库完全不参与本次构建。
    // 所以「库调库」不需要写 import：bigint 内部引用了 strutil 的符号，
    // strutil 就会被自动拉进来。
    // 借引用而不是克隆：AST 深拷贝在几十个模块时是明显开销。
    let mut project_sets: Vec<&[Item]> = Vec::with_capacity(modules.len() + 1);
    project_sets.push(&entry_items);
    for m in &modules {
        project_sets.push(&m.items);
    }
    let summary = libpkg::summarize(&project_sets);
    let sel = match libpkg::select(&summary) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("error: {}", msg);
            std::process::exit(1);
        }
    };

    if std::env::var("LUME_LIB_TRACE").is_ok() {
        for (name, dir, deps) in &sel.loaded {
            let d = if deps.is_empty() {
                String::new()
            } else {
                format!("  (依赖 {})", deps.join(", "))
            };
            eprintln!("// 库 {}  {}{}", name, dir.display(), d);
        }
    }

    let mut known: HashSet<PathBuf> = HashSet::new();
    for m in &modules {
        known.insert(m.path.clone());
    }
    for f in sel.files {
        let abs = std::fs::canonicalize(&f.path).unwrap_or_else(|_| f.path.clone());
        if !known.insert(abs.clone()) {
            continue; // 同一个文件从项目和库两条路走进来只算一次
        }
        modules.push(Parsed {
            path: abs,
            items: f.items,
        });
    }

    // 4. 合并 + 重复定义检测
    let mut items: Vec<Item> = Vec::new();
    let mut seen: HashMap<String, (SymKind, PathBuf)> = HashMap::new();

    for (path, its) in std::iter::once((entry_abs.clone(), entry_items))
        .chain(modules.into_iter().map(|m| (m.path, m.items)))
    {
        for item in its {
            let (kind, name) = match &item {
                Item::Fn(f) => (SymKind::Fn, f.name.clone()),
                Item::Struct(s) => (SymKind::Struct, s.name.clone()),
                Item::Enum(e) => (SymKind::Enum, e.name.clone()),
                Item::Const(c) => (SymKind::Const, c.name.clone()),
                _ => {
                    items.push(item);
                    continue;
                }
            };
            if let Some((_, first)) = seen.get(&name) {
                eprintln!(
                    "error: {} '{}' 重复定义\n         {} 和 {}",
                    kind.label(),
                    name,
                    first.display(),
                    path.display()
                );
                eprintln!(
                    "  help: 同名顶层符号在整个项目里只能有一份，改个名字或删掉一份"
                );
                std::process::exit(1);
            }
            seen.insert(name, (kind, path.clone()));
            items.push(item);
        }
    }

    let mut program = crate::ast::Program { items };

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

    // 只跑一遍。以前先调 optimize()（无条件 Aggressive）再按 LUME_OPT
    // 跑第二遍，导致 LUME_OPT=safe 实际无效、编译期也多跑一次。
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
    codegen.set_bounds_check(!opts.unchecked);
    for (name, table) in precomputed {
        codegen.set_precomputed(name, table);
    }
    let c_code = codegen.generate(program);
    // free 误用之类的所有权错误：直接按编译器错误报，不进入 clang
    if !codegen.codegen_errors.is_empty() {
        for e in &codegen.codegen_errors {
            eprintln!("error: {}", e);
        }
        std::process::exit(1);
    }
    c_code
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

    // 我们按「AST 看起来纯」发 `#pragma clang loop vectorize(enable)`，
    // 但代码生成会插入 lume_vec_at（边界检查）、lume_vec_push 这类
    // 运行时调用，clang 到那一步才发现没法向量化，于是每条都回一句
    // 「loop not vectorized」。这些提示对使用者没有可操作性（pragma 只是
    // 建议，clang 自己仍会尝试），关掉这一类诊断。
    cmd.arg("-Wno-pass-failed");

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

    // 编译填表程序。openmp=true 时加 -fopenmp（只影响编译期填表速度）。
    let compile = |openmp: bool| -> std::io::Result<bool> {
        let mut cmd = Command::new("clang");
        cmd.arg(&tmp_c).arg("-o").arg(&tmp_bin)
            .arg(format!("-O{}", opts.opt));

        if opts.native {
            cmd.arg("-march=native");
        }

        #[cfg(not(target_os = "windows"))]
        {
            if openmp {
                cmd.arg("-fopenmp");
            }
            cmd.arg("-lm");
            cmd.arg("-lpthread");
        }
        #[cfg(target_os = "windows")]
        {
            let _ = openmp;
        }

        // 第一次探路（带 -fopenmp）时把 clang 的链接错误吞掉：
        // 没有 libomp 是正常情况，后面会自动串行重试。
        if openmp {
            cmd.stderr(Stdio::null());
        }

        // 没有 libomp 时 `#pragma omp` 会被忽略；别让它的警告刷屏
        if !openmp {
            cmd.arg("-Wno-unknown-pragmas");
        }

        // 预计算程序只读表、不算 errno；关掉 errno 让 sqrt/pow 保持纯函数
        cmd.arg("-fno-math-errno");
        if opts.fastmath {
            cmd.arg("-ffast-math");
        }

        cmd.status().map(|s| s.success())
    };

    #[cfg(not(target_os = "windows"))]
    let openmp = PRECOMPUTE_OPENMP.load(Ordering::Relaxed) != 2;
    #[cfg(target_os = "windows")]
    let openmp = false;

    let mut ok = compile(openmp).map_err(|e| format!("clang: {}", e))?;
    if !ok && openmp {
        // 大多数发行版只装 clang 不装 libomp：链接失败会让 @precompute
        // 整个退化成运行时缓存。填表本来就是编译期一次性的，串行即可。
        eprintln!("// 预计算: libomp 不可用，改用串行填表");
        PRECOMPUTE_OPENMP.store(2, Ordering::Relaxed);
        ok = compile(false).map_err(|e| format!("clang: {}", e))?;
    }
    if !ok {
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