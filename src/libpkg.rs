// ─────────────────────────────────────────────────────────────
// 库包发现与选择（`lume` 的「库调库」）
//
// 背景：Lume 是零 import 语言 —— 同目录 / 子目录的 .lume 自动合并。
// 这套规则对**项目**很好用，但库不行：库要能被不同项目复用，就必然
// 放在项目目录之外（`$LUME_PATH` / `~/.lume/lib`），而且库之间还会
// 互相依赖（bigint 用 strutil）。
//
// 以前的做法是把所有库根下的 .lume **全部**无条件合并。库少时能跑，
// 库一多就三件事全崩：
//   1. 编译变慢（每个项目都要编译所有库）；
//   2. 命名冲突（两个库都有 `parse` 就报重名，哪怕你只用其中一个）；
//   3. 跨根依赖没法表达（bigint 在 ~/libs/bigint，strutil 在
//      /opt/lume-libs/strutil，两个根不同，只能靠两个都在 LUME_PATH 里）；
//   4. 库目录里放个带 `fn main` 的示例程序就得整份跳过。
//
// 现在的模型：**库包（package）+ 按符号需求自动加载**。
//
//   * 库根下的每个**一级子目录**是一个库包，包名 = 目录名。
//   * 库根下直接放的 `.lume` 也算一个包，包名 = 文件名（散装库，兼容旧布局）。
//   * 包内文件递归收集，带 `fn main` 的当作示例程序剔除（其余文件照收）。
//   * 包内可以写 `import 别的包名` 声明依赖 —— 这是「库调库」的显式通路。
//     依赖会在加载本包时被一并加载（递归），不要求在同一个库根下。
//
// 加载时机（关键）：**按需**。
//   从入口项目引用到的顶层名字出发，若某个名字恰好只被一个库包提供，
//   就加载那个包；加载后把它引用的名字并入需求集，继续迭代到不动点。
//   所以项目里一行 import 都不写，调 `big_add` 就能自动把 bigint 拉进来，
//   bigint 内部调 `su_trim` 又会自动把 strutil 拉进来。
//   两个包提供同一个名字且真的被用到 → 报歧义，让你 `import` 指定。
//
// 这样：没用到的库不解析进 program（不冲突、不拖慢），用到的库及其
// 依赖闭包自动到齐，而项目的零 import 体验一点没变。
// ─────────────────────────────────────────────────────────────

use crate::ast::{Block, Expr, Item, Pattern, Stmt, StrPart};
use crate::lexer::Lexer;
use crate::parser::Parser;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 自动扫描时跳过的目录：隐藏目录（.git / .workbuddy / .lib）+ 构建产物。
pub(crate) const SKIP_DIRS: &[&str] = &["target", "node_modules"];

pub(crate) fn skip_dir(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

/// 递归收集目录下的 .lume（不跳过隐藏子目录时会连 .lib 一起收，
/// 所以调用方决定传哪个 dir）。
pub(crate) fn collect_lume_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !skip_dir(&name) {
                collect_lume_files(&p, out);
            }
        } else if name.ends_with(".lume") {
            out.push(p);
        }
    }
}

pub(crate) fn parse_file(path: &Path, src: &str) -> Result<Vec<Item>, String> {
    let mut lexer = Lexer::new(src);
    let tokens = lexer
        .tokenize()
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    let mut parser = Parser::new(tokens);
    let program = parser
        .parse_program()
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    Ok(program.items)
}

pub(crate) fn has_main(items: &[Item]) -> bool {
    items
        .iter()
        .any(|i| matches!(i, Item::Fn(f) if f.name == "main"))
}

// ─────────────────────────────────────────────────────────────
// 库搜索根
// ─────────────────────────────────────────────────────────────

/// 共享库目录，按优先级：
///   1. `$LUME_PATH` 里列的目录（`:` 分隔；Windows 盘符里有冒号，
///      所以 Windows 上按 `;` 拆，其它平台按 `:` 拆）
///   2. `~/.lume/lib`（存在才算）
///
/// 不存在的目录直接忽略 —— 绝大多数项目不需要库，不该因为少个目录就报错。
pub(crate) fn lib_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(v) = std::env::var("LUME_PATH") {
        let sep: &[char] = if cfg!(windows) { &[';'] } else { &[':'] };
        for part in v.split(sep) {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            let pb = PathBuf::from(p);
            if pb.is_dir() {
                roots.push(pb);
            }
        }
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    if let Some(h) = home {
        let p = PathBuf::from(h).join(".lume").join("lib");
        if p.is_dir() {
            roots.push(p);
        }
    }

    roots
}

// ─────────────────────────────────────────────────────────────
// 包
// ─────────────────────────────────────────────────────────────

pub(crate) struct PkgFile {
    pub path: PathBuf,
    pub items: Vec<Item>,
}

pub(crate) struct Pkg {
    /// 包名。子目录包 = 目录名；散装库 = 文件名（去 .lume）。
    pub name: String,
    pub dir: PathBuf,
    /// 已剔除「带 fn main 的示例程序」和「解析失败的文件」。
    pub files: Vec<PkgFile>,
    /// `import x` 声明的依赖（只取第一段：`import std.str` → "std"）。
    pub deps: Vec<String>,
    /// 本包对外提供的顶层符号（fn / struct / enum / const）。
    pub provides: HashSet<String>,
    /// 本包引用到的、不是自己定义的顶层名字。
    pub refs: HashSet<String>,
    /// 是否散装库（库根下直接放的单个 .lume）。
    pub loose: bool,
}

fn build_pkg(name: String, dir: PathBuf, loose: bool, files: Vec<PathBuf>) -> Option<Pkg> {
    let mut parsed: Vec<PkgFile> = Vec::new();
    let mut provides: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut deps: Vec<String> = Vec::new();

    for path in files {
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let items = match parse_file(&path, &src) {
            Ok(i) => i,
            Err(msg) => {
                eprintln!("warning: 跳过无法解析的库文件\n         {}", msg);
                continue;
            }
        };
        // 库目录里放示例程序是常态：整份跳过，别把 main 带进别人的程序。
        if has_main(&items) {
            continue;
        }
        collect_provides(&items, &mut provides);
        collect_imports(&items, &mut deps, &name);
        parsed.push(PkgFile { path, items });
    }

    if parsed.is_empty() {
        return None;
    }

    let mut locals: HashSet<String> = HashSet::new();
    for f in &parsed {
        collect_locals(&f.items, &mut locals);
    }
    for f in &parsed {
        collect_refs(&f.items, &mut refs);
    }
    // 自己提供的、显然是局部变量的，都不算「对外引用」。
    for s in &provides {
        refs.remove(s);
    }
    for s in &locals {
        refs.remove(s);
    }

    Some(Pkg {
        name,
        dir,
        files: parsed,
        deps,
        provides,
        refs,
        loose,
    })
}

/// 扫所有库根，建成包列表。同名包按库根优先级先到先得。
pub(crate) fn discover() -> Vec<Pkg> {
    let mut out: Vec<Pkg> = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();

    for root in lib_roots() {
        let rd = match std::fs::read_dir(&root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut loose: Vec<PathBuf> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let fname = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !skip_dir(&fname) {
                    dirs.push(p);
                }
            } else if fname.ends_with(".lume") {
                loose.push(p);
            }
        }
        dirs.sort();
        loose.sort();

        for d in dirs {
            let name = match d.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            if !taken.insert(name.clone()) {
                continue; // 高优先级的根里已经有同名包
            }
            let mut files = Vec::new();
            collect_lume_files(&d, &mut files);
            files.sort();
            if let Some(p) = build_pkg(name, d, false, files) {
                out.push(p);
            }
        }

        for f in loose {
            let name = match f.file_stem() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            if !taken.insert(name.clone()) {
                continue;
            }
            if let Some(p) = build_pkg(name, root.clone(), true, vec![f]) {
                out.push(p);
            }
        }
    }

    out
}

// ─────────────────────────────────────────────────────────────
// 符号收集
// ─────────────────────────────────────────────────────────────

/// 顶层符号。`main` 不算（它是入口，不是库 API）。
fn collect_provides(items: &[Item], out: &mut HashSet<String>) {
    for it in items {
        match it {
            Item::Fn(f) => {
                if f.name != "main" {
                    out.insert(f.name.clone());
                }
            }
            Item::Struct(s) => {
                out.insert(s.name.clone());
            }
            Item::Enum(e) => {
                out.insert(e.name.clone());
            }
            Item::Const(c) => {
                out.insert(c.name.clone());
            }
            _ => {}
        }
    }
}

fn collect_imports(items: &[Item], out: &mut Vec<String>, self_name: &str) {
    for it in items {
        if let Item::Import(imp) = it {
            if let Some(first) = imp.path.first() {
                if first == self_name {
                    continue; // import 自己，忽略
                }
                if !out.contains(first) {
                    out.push(first.clone());
                }
            }
        }
    }
}

// ── 局部名（用来把「变量」从「顶层符号引用」里剔掉） ──

fn collect_locals(items: &[Item], out: &mut HashSet<String>) {
    for it in items {
        match it {
            Item::Fn(f) => {
                for p in &f.params {
                    out.insert(p.name.clone());
                }
                collect_locals_block(&f.body, out);
            }
            // 方法体的局部名同样要收集 —— 库 A 的方法里写 `let x`
            // 不该被当成「引用顶层符号 x」而去拉一个叫 x 的库。
            Item::Struct(s) => {
                for m in &s.methods {
                    for p in &m.params {
                        out.insert(p.name.clone());
                    }
                    // 方法体里的裸字段名不是顶层符号
                    for f in &s.fields {
                        out.insert(f.name.clone());
                    }
                    collect_locals_block(&m.body, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_locals_block(b: &Block, out: &mut HashSet<String>) {
    for s in &b.stmts {
        collect_locals_stmt(s, out);
    }
}

fn collect_locals_stmt(s: &Stmt, out: &mut HashSet<String>) {
    match s {
        Stmt::Let { name, .. } => {
            out.insert(name.clone());
        }
        Stmt::Assign { .. } | Stmt::Expr(_) | Stmt::Return(_) => {}
        Stmt::If { then, els, .. } => {
            collect_locals_block(then, out);
            if let Some(e) = els {
                collect_locals_stmt(e, out);
            }
        }
        Stmt::While { body, .. } => collect_locals_block(body, out),
        Stmt::For { var, body, .. } | Stmt::ParFor { var, body, .. } => {
            out.insert(var.clone());
            collect_locals_block(body, out);
        }
        Stmt::Break | Stmt::Continue => {}
        Stmt::Match { arms, .. } => {
            for arm in arms {
                collect_pattern_locals(&arm.pattern, out);
                if let Expr::If(_, then, els) = &arm.body {
                    collect_locals_block(then, out);
                    if let Some(e) = els {
                        collect_locals_block(e, out);
                    }
                }
            }
        }
        Stmt::Scope { body } => collect_locals_block(body, out),
    }
}

fn collect_pattern_locals(p: &Pattern, out: &mut HashSet<String>) {
    match p {
        Pattern::Ident(n) => {
            out.insert(n.clone());
        }
        Pattern::Tuple(v) => {
            for x in v {
                collect_pattern_locals(x, out);
            }
        }
        Pattern::Struct(_, fields) => {
            for (_, x) in fields {
                collect_pattern_locals(x, out);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) | Pattern::EnumVariant(_, _) => {}
    }
}

// ── 引用名 ──

fn collect_refs(items: &[Item], out: &mut HashSet<String>) {
    for it in items {
        match it {
            Item::Fn(f) => {
                for dec in &f.decorators {
                    for a in &dec.args {
                        collect_expr_refs(a, out);
                    }
                }
                // 签名里的类型名也算引用：`fn area(d: Dim) -> int`
                // 的 Dim 可能是别的库提供的。只看函数体是收不到的。
                collect_sig_type_refs(f, out);
                collect_block_refs(&f.body, out);
            }
            Item::Struct(s) => {
                for f in &s.fields {
                    collect_type_refs(&f.ty, out);
                }
                for m in &s.methods {
                    collect_sig_type_refs(m, out);
                    collect_block_refs(&m.body, out);
                }
            }
            Item::Const(c) => {
                if let Some(t) = &c.ty {
                    collect_type_refs(t, out);
                }
                collect_expr_refs(&c.value, out);
            }
            _ => {}
        }
    }
}

fn collect_sig_type_refs(f: &crate::ast::FnDecl, out: &mut HashSet<String>) {
    for p in &f.params {
        if let Some(t) = &p.ty {
            collect_type_refs(t, out);
        }
    }
    if let Some(t) = &f.ret {
        collect_type_refs(t, out);
    }
}

fn collect_type_refs(t: &crate::ast::Type, out: &mut HashSet<String>) {
    use crate::ast::Type;
    match t {
        Type::Named(n) => {
            out.insert(n.clone());
        }
        Type::List(i) | Type::Optional(i) | Type::Error(i) => collect_type_refs(i, out),
        Type::Array(i, _) => collect_type_refs(i, out),
        Type::Map(k, v) => {
            collect_type_refs(k, out);
            collect_type_refs(v, out);
        }
        Type::Tuple(v) => {
            for x in v {
                collect_type_refs(x, out);
            }
        }
        Type::Generic(base, args) => {
            collect_type_refs(base, out);
            for a in args {
                collect_type_refs(a, out);
            }
        }
        _ => {}
    }
}

fn collect_block_refs(b: &Block, out: &mut HashSet<String>) {
    for s in &b.stmts {
        collect_stmt_refs(s, out);
    }
}

fn collect_stmt_refs(s: &Stmt, out: &mut HashSet<String>) {
    match s {
        Stmt::Let { init, .. } => collect_expr_refs(init, out),
        Stmt::Assign { target, value } => {
            collect_expr_refs(target, out);
            collect_expr_refs(value, out);
        }
        Stmt::Expr(e) => collect_expr_refs(e, out),
        Stmt::If { cond, then, els } => {
            collect_expr_refs(cond, out);
            collect_block_refs(then, out);
            if let Some(e) = els {
                collect_stmt_refs(e, out);
            }
        }
        Stmt::While { cond, body } => {
            collect_expr_refs(cond, out);
            collect_block_refs(body, out);
        }
        Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
            collect_expr_refs(iter, out);
            collect_block_refs(body, out);
        }
        Stmt::Return(v) => {
            if let Some(e) = v {
                collect_expr_refs(e, out);
            }
        }
        Stmt::Break | Stmt::Continue => {}
        Stmt::Match { value, arms } => {
            collect_expr_refs(value, out);
            for arm in arms {
                collect_expr_refs(&arm.body, out);
            }
        }
        Stmt::Scope { body } => collect_block_refs(body, out),
    }
}

fn collect_expr_refs(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Ident(n) => {
            out.insert(n.clone());
        }
        Expr::StructLit(name, fields) => {
            out.insert(name.clone());
            for (_, v) in fields {
                collect_expr_refs(v, out);
            }
        }
        Expr::FmtStr(parts) => {
            for p in parts {
                if let StrPart::Expr(x) = p {
                    collect_expr_refs(x, out);
                }
            }
        }
        Expr::Binary(a, _, b) => {
            collect_expr_refs(a, out);
            collect_expr_refs(b, out);
        }
        Expr::Unary(_, a) | Expr::Await(a) | Expr::Try(a) | Expr::Force(a) => {
            collect_expr_refs(a, out)
        }
        Expr::Call(c, args) => {
            collect_expr_refs(c, out);
            for a in args {
                collect_expr_refs(a, out);
            }
        }
        Expr::Field(o, _) => collect_expr_refs(o, out),
        Expr::Index(o, i) => {
            collect_expr_refs(o, out);
            collect_expr_refs(i, out);
        }
        Expr::Slice(o, a, b, _) => {
            collect_expr_refs(o, out);
            if let Some(x) = a {
                collect_expr_refs(x, out);
            }
            if let Some(x) = b {
                collect_expr_refs(x, out);
            }
        }
        Expr::Lambda { body, .. } => collect_block_refs(body, out),
        Expr::Tuple(v) | Expr::List(v) => {
            for x in v {
                collect_expr_refs(x, out);
            }
        }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_expr_refs(k, out);
                collect_expr_refs(v, out);
            }
        }
        Expr::Repeat(x, _) => collect_expr_refs(x, out),
        Expr::Range(a, b, _) => {
            collect_expr_refs(a, out);
            collect_expr_refs(b, out);
        }
        Expr::If(c, then, els) => {
            collect_expr_refs(c, out);
            collect_block_refs(then, out);
            if let Some(e) = els {
                collect_block_refs(e, out);
            }
        }
        Expr::Match(v, arms) => {
            collect_expr_refs(v, out);
            for arm in arms {
                collect_expr_refs(&arm.body, out);
            }
        }
        Expr::Spawn(b) => collect_block_refs(b, out),
        Expr::Int(_)
        | Expr::Float(_)
        | Expr::Str(_)
        | Expr::Rune(_)
        | Expr::Color(_)
        | Expr::Bool(_)
        | Expr::Nothing
        | Expr::It => {}
    }
}

// ─────────────────────────────────────────────────────────────
// 选择
// ─────────────────────────────────────────────────────────────

/// 一批源文件的「对外画像」：引用了什么、import 了什么。
/// （自己提供的符号已经从 refs 里剔掉了 —— 那些引用指向文件内部。）
pub(crate) struct Summary {
    pub refs: HashSet<String>,
    pub imports: Vec<String>,
}

pub(crate) fn summarize(sets: &[&[Item]]) -> Summary {
    let mut provides = HashSet::new();
    let mut refs = HashSet::new();
    let mut imports = Vec::new();
    let mut locals = HashSet::new();

    for &items in sets {
        collect_provides(items, &mut provides);
        collect_imports(items, &mut imports, "");
        collect_locals(items, &mut locals);
        collect_refs(items, &mut refs);
    }
    for s in &provides {
        refs.remove(s);
    }
    for s in &locals {
        refs.remove(s);
    }
    Summary { refs, imports }
}

pub(crate) struct Selection {
    pub files: Vec<PkgFile>,
    /// (包名, 目录, 依赖) —— 给 `lume libs` 和 LUME_LIB_TRACE 用。
    pub loaded: Vec<(String, PathBuf, Vec<String>)>,
}

/// 从「项目画像」出发，选出需要参与本次构建的库文件。
///
/// 迭代到不动点：项目引用的名字 → 唯一提供它的包 → 该包引用的名字 → ...
/// 显式 `import x` 的包无条件加载（即使没引用它的符号）。
pub(crate) fn select(sum: &Summary) -> Result<Selection, String> {
    let mut pkgs = discover();
    // 注意：pkgs 为空也要往下走 —— 项目里写了 `import x` 而环境里一个库
    // 都没有时，得报「找不到对应的库」，不能静默当没写。

    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for (i, p) in pkgs.iter().enumerate() {
        by_name.insert(p.name.as_str(), i);
    }

    let mut loaded: HashSet<usize> = HashSet::new();
    let mut order: Vec<usize> = Vec::new();
    let mut need: HashSet<String> = sum.refs.clone();

    // 1. 显式 import。
    //
    // 项目侧**宽松**：`import` 以前是空操作（`import std.str` 更是什么都
    // 不做），现在把它当「软提示」—— 找得到就加载，找不到就忽略，
    // 免得历史代码里一句 `import foo` 直接编译不过。真用到了 foo 的符号
    // 而 foo 又不在，checker 会报「未定义的函数」。
    // 库侧**严格**：包的 import 是依赖声明，缺了必然编不过，直接报错
    // （见下面 deps 的处理）。
    let mut queue: Vec<String> = sum.imports.clone();
    let mut qi = 0;
    while qi < queue.len() {
        let name = queue[qi].clone();
        qi += 1;
        if name == "std" {
            continue;
        }
        match by_name.get(name.as_str()) {
            Some(&idx) => {
                if loaded.insert(idx) {
                    order.push(idx);
                    need.extend(pkgs[idx].refs.iter().cloned());
                    for d in &pkgs[idx].deps {
                        if !queue.contains(d) {
                            queue.push(d.clone());
                        }
                    }
                }
            }
            None => {
                if std::env::var("LUME_LIB_TRACE").is_ok() {
                    eprintln!(
                        "// import '{}' 没有对应的库包，当作模块提示忽略",
                        name
                    );
                }
            }
        }
    }

    // 2. 按符号需求反复挑包，直到没有新包可加载
    loop {
        // 已被加载的包提供的名字 → 需求已满足
        let mut satisfied: HashSet<&str> = HashSet::new();
        for &idx in &loaded {
            for s in &pkgs[idx].provides {
                satisfied.insert(s.as_str());
            }
        }

        let mut hit: Option<usize> = None;
        let mut ambiguous: Option<(String, Vec<String>)> = None;

        let mut names: Vec<&String> = need.iter().collect();
        names.sort(); // 报错顺序稳定

        for n in names {
            if satisfied.contains(n.as_str()) {
                continue;
            }
            let cands: Vec<usize> = pkgs
                .iter()
                .enumerate()
                .filter(|(i, p)| !loaded.contains(i) && p.provides.contains(n))
                .map(|(i, _)| i)
                .collect();
            if cands.len() > 1 {
                ambiguous = Some((
                    n.clone(),
                    cands.iter().map(|&i| pkgs[i].name.clone()).collect(),
                ));
                break;
            }
            if cands.len() == 1 {
                hit = Some(cands[0]);
                break;
            }
        }

        if let Some((name, owners)) = ambiguous {
            return Err(format!(
                "库符号 '{}' 有歧义，同时由 {} 个库提供：{}\n  help: 用 `import <库名>` 指定用哪个，或给其中一个函数改个带前缀的名字",
                name,
                owners.len(),
                owners.join(" / ")
            ));
        }

        let idx = match hit {
            Some(i) => i,
            None => break,
        };

        loaded.insert(idx);
        order.push(idx);
        need.extend(pkgs[idx].refs.iter().cloned());
        // 依赖声明：包被加载后，它 import 的包也要在
        for d in pkgs[idx].deps.clone() {
            if d == "std" {
                continue;
            }
            match by_name.get(d.as_str()) {
                Some(&di) => {
                    if loaded.insert(di) {
                        order.push(di);
                        need.extend(pkgs[di].refs.iter().cloned());
                    }
                }
                None => {
                    return Err(format!(
                        "库 '{}' 依赖 '{}'，但找不到那个库\n  help: 把提供 '{}' 的目录加到 $LUME_PATH，或把该库放进同一个库根",
                        pkgs[idx].name, d, d
                    ));
                }
            }
        }
    }

    // 3. 收集文件（按加载顺序，依赖在前）。
    //    这里 move 而不是 clone：AST 深拷贝在大库上是实打实的开销，
    //    而 pkgs 之后不再需要。
    let mut files: Vec<PkgFile> = Vec::new();
    let mut out_loaded: Vec<(String, PathBuf, Vec<String>)> = Vec::new();
    for &idx in &order {
        let name = pkgs[idx].name.clone();
        let dir = pkgs[idx].dir.clone();
        let deps = pkgs[idx].deps.clone();
        for f in pkgs[idx].files.drain(..) {
            files.push(f);
        }
        out_loaded.push((name, dir, deps));
    }

    Ok(Selection {
        files,
        loaded: out_loaded,
    })
}
