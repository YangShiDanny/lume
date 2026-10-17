// src/codegen/mod.rs
mod expr;
mod items;
mod parallel;
mod purity;
mod runtime;
mod say;
mod stmt;
mod types;

use crate::ast::*;
use std::collections::{HashMap, HashSet};

/// 数组类型判定：`[T]` 解析成 List，`[T; N]` 解析成 Array。
/// 两者在 codegen 里一视同仁：指针 + 一个隐式长度伴随参数。
pub(crate) fn is_array_ty(t: &Option<Type>) -> bool {
    matches!(t, Some(Type::Array(_, _)) | Some(Type::List(_)))
}

pub struct Codegen {
    pub(crate) output: String,
    pub(crate) indent: usize,
    pub(crate) var_types: HashMap<String, String>,
    pub(crate) structs: HashMap<String, Vec<(String, String)>>,
    // struct 名 → 类型标签编号。C 没有 RTTI，match 结构体解构靠它判断类型
    pub(crate) struct_tags: HashMap<String, i64>,
    pub(crate) methods: HashMap<(String, String), String>,
    // 数组名 → 该数组长度的 C 表达式。
    //   局部数组字面量 → "5LL"
    //   数组参数       → "a_len"（伴随参数）
    //   函数返回的数组 → "_r_len"
    // 这是「数组跨函数」的核心：长度跟着数组一起走，调用点自动补。
    pub(crate) array_len_exprs: HashMap<String, String>,
    pub(crate) array_elem_types: HashMap<String, String>,
    // 数组/列表参数名（长度编译期未知）
    pub(crate) array_params: HashSet<String>,
    // 已证明为纯函数、可以打 __attribute__((const)) 的函数名
    pub(crate) const_fns: HashSet<String>,
    pub(crate) precomputed: HashMap<String, Vec<i64>>,
    pub(crate) generic_fns: HashMap<String, FnDecl>,
    // 普通函数的返回类型（供插值/类型推断用，见 generate 步骤 2.4）
    pub(crate) fn_ret_types: HashMap<String, String>,
    // 函数名 → 每个参数是不是数组（调用点据此补长度实参）
    pub(crate) fn_array_params: HashMap<String, Vec<bool>>,
    // (结构体名, 方法名) → 同上
    pub(crate) method_array_params: HashMap<(String, String), Vec<bool>>,
    // 返回数组的函数：C 签名多一个 int64_t* _ret_len 出参
    pub(crate) array_ret_fns: HashSet<String>,
    pub(crate) monomorphized: HashMap<String, String>,
    pub(crate) pending_monos: Vec<(FnDecl, String, HashMap<String, String>)>,
    pub(crate) par_counter: usize,
    // par for 的 worker 函数（提到文件顶层，不能嵌在函数里）
    pub(crate) par_workers: String,
    // 当前 par worker 里通过 _p-> 访问的只读外层标量（见 parallel.rs）
    pub(crate) par_scalars: Vec<String>,
    // 当前函数是否返回数组（决定 return 要不要 malloc 拷贝）
    pub(crate) cur_ret_is_array: bool,
    // match 生成 union 时，值的真实类型对应的成员名（如 "Circle"）。
    // None 表示没走 union 路径。字面量/变量绑定臂要通过它访问 _match_val。
    pub(crate) match_val_member: Option<String>,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            var_types: HashMap::new(),
            structs: HashMap::new(),
            struct_tags: HashMap::new(),
            methods: HashMap::new(),
            array_len_exprs: HashMap::new(),
            array_elem_types: HashMap::new(),
            array_params: HashSet::new(),
            const_fns: HashSet::new(),
            precomputed: HashMap::new(),
            generic_fns: HashMap::new(),
            fn_ret_types: HashMap::new(),
            fn_array_params: HashMap::new(),
            method_array_params: HashMap::new(),
            array_ret_fns: HashSet::new(),
            monomorphized: HashMap::new(),
            pending_monos: Vec::new(),
            par_counter: 0,
            par_workers: String::new(),
            par_scalars: Vec::new(),
            cur_ret_is_array: false,
            match_val_member: None,
        }
    }

    pub fn set_precomputed(&mut self, name: String, table: Vec<i64>) {
        self.precomputed.insert(name, table);
    }

    pub fn generate(&mut self, program: &Program) -> String {
        self.output.push_str(runtime::RUNTIME_HEADER);
        self.output.push('\n');

        // 1. 收集 struct 定义和方法签名
        let mut tag_counter: i64 = 0;
        for item in &program.items {
            if let Item::Struct(s) = item {
                let fields: Vec<(String, String)> = s.fields.iter()
                    .map(|f| (f.name.clone(), self.gen_type(&f.ty)))
                    .collect();
                self.structs.insert(s.name.clone(), fields);
                // 按出现顺序分配类型标签
                self.struct_tags.insert(s.name.clone(), tag_counter);
                tag_counter += 1;

                for m in &s.methods {
                    let ret = match &m.ret {
                        None => "void".to_string(),
                        Some(t) => self.gen_type(t),
                    };
                    self.methods.insert((s.name.clone(), m.name.clone()), ret);
                    self.method_array_params.insert(
                        (s.name.clone(), m.name.clone()),
                        m.params.iter().map(|p| is_array_ty(&p.ty)).collect(),
                    );
                }
            }
        }

        // 2. 收集泛型函数
        for item in &program.items {
            if let Item::Fn(f) = item {
                if !f.generics.is_empty() {
                    self.generic_fns.insert(f.name.clone(), f.clone());
                }
            }
        }

        // 2.4 收集普通函数的返回类型 + 数组参数标记。
        //     没有返回类型表，say("{f(x)}") 里 f 返回 float 也会被推断成
        //     int64_t，插值就用 %lld 打印，小数全丢。
        //     数组参数标记则是调用点自动补长度的依据。
        for item in &program.items {
            if let Item::Fn(f) = item {
                let ret = match &f.ret {
                    None => "void".to_string(),
                    Some(t) => self.gen_type(t),
                };
                self.fn_ret_types.insert(f.name.clone(), ret);
                self.fn_array_params.insert(
                    f.name.clone(),
                    f.params.iter().map(|p| is_array_ty(&p.ty)).collect(),
                );
                if is_array_ty(&f.ret) {
                    self.array_ret_fns.insert(f.name.clone());
                }
            }
        }

        // 2.45 预置内置函数签名。
        //      infer_type / 数组长度推导都要查这些表，
        //      不预置的话 say("{str_contains(a,b)}") 会被推断成 int64_t。
        self.register_builtin_sigs();

        // 2.5 纯函数分析：必须在发前向声明之前跑完，
        //     因为 __attribute__((const)) 要写在声明上，调用点才能看到
        self.collect_const_fns(program);

        // 3. 生成 struct 定义
        for item in &program.items {
            if let Item::Struct(s) = item {
                self.gen_struct_decl(s);
            }
        }

        // par for 的 worker 插在这里：必须在所有函数定义之前，
        // 这样后面的函数才能直接调用它
        let worker_pos = self.output.len();

        // 3.5 前向声明（方法 + 普通函数）
        //     这样函数可以互相调用、也可以调用后面才定义的函数
        for item in &program.items {
            if let Item::Struct(s) = item {
                for m in &s.methods {
                    self.gen_method_decl(&s.name, m);
                }
            }
        }
        for item in &program.items {
            if let Item::Fn(f) = item {
                // 泛型函数走单态化；带装饰器的函数自己会发原型
                if f.generics.is_empty() && f.decorators.is_empty() {
                    self.gen_fn_decl(f);
                }
            }
        }
        self.output.push('\n');

        // 4. 生成方法
        for item in &program.items {
            if let Item::Struct(s) = item {
                for m in &s.methods {
                    self.gen_method(&s.name, m);
                }
            }
        }

        // 5. 预扫描：收集所有泛型调用 → pending_monos
        self.collect_generic_calls(program);

        // 6. 循环收集，直到没有新的泛型调用
        let mut monos: Vec<(FnDecl, String, HashMap<String, String>)> = Vec::new();
        loop {
            self.collect_generic_calls(program);
            let batch: Vec<_> = self.pending_monos.drain(..).collect();
            if batch.is_empty() {
                break;
            }
            monos.extend(batch);
        }

        // 7. 生成所有单态化的前向声明
        for (f, name, tm) in &monos {
            self.gen_monomorphized_decl(f, name, tm);
        }
        if !monos.is_empty() {
            self.output.push('\n');
        }

        // 8. 单态化定义
        for (f, name, tm) in &monos {
            self.gen_monomorphized(f, name, tm);
        }

       // 9. 普通函数
        for item in &program.items {
            match item {
                Item::Fn(f) => self.gen_fn(f),
                Item::Const(c) => self.gen_const(c),
                _ => {}
            }
        }

        let mut out = std::mem::take(&mut self.output);
        if !self.par_workers.is_empty() {
            out.insert_str(worker_pos, &self.par_workers);
        }
        out
    }

    pub fn generate_precompute_program(
        &mut self,
        f: &FnDecl,
        start: i64,
        end: i64,
    ) -> String {
        self.output.push_str(runtime::RUNTIME_HEADER);
        self.output.push('\n');

        self.gen_thread_safe_cached_fn(f);

        let n = end - start + 1;
        self.push_line("int main(void) {");
        self.indent += 1;

        self.push_line(&format!("int64_t _N = {}LL;", n));
        self.push_line("int64_t* _results = malloc(_N * sizeof(int64_t));");
        self.push_line(&format!(
            "int64_t _start = {}LL, _end = {}LL;",
            start, end
        ));
        self.push_line("#pragma omp parallel for");
        self.push_line("for (int64_t i = _start; i <= _end; i++) {");
        self.indent += 1;
        self.push_line(&format!(
            "_results[i - _start] = {}(i);",
            f.name
        ));
        self.indent -= 1;
        self.push_line("}");
        self.push_line("for (int64_t i = 0; i < _N; i++) {");
        self.indent += 1;
        self.push_line(&format!("printf(\"%lld\\n\", (long long)_results[i]);"));
        self.indent -= 1;
        self.push_line("}");
        self.push_line("free(_results);");
        self.push_line("return 0;");

        self.indent -= 1;
        self.push_line("}");

        std::mem::take(&mut self.output)
    }

    /// 内置函数签名表。返回 "const char**" 的（只有 str_split）
    /// 同时登记成数组返回函数，走 `let x = f(...)` 的三行拆开。
    fn register_builtin_sigs(&mut self) {
        let sigs: &[(&str, &str)] = &[
            ("str_len", "int64_t"),
            ("str_upper", "const char*"),
            ("str_lower", "const char*"),
            ("str_trim", "const char*"),
            ("str_to_int", "int64_t"),
            ("str_to_float", "double"),
            ("str_contains", "bool"),
            ("str_starts_with", "bool"),
            ("str_ends_with", "bool"),
            ("str_replace", "const char*"),
            ("str_split", "const char**"),
            ("int_abs", "int64_t"),
            ("int_min", "int64_t"),
            ("int_max", "int64_t"),
            ("float_abs", "double"),
            ("sqrt", "double"),
            ("pow", "double"),
            ("sin", "double"),
            ("cos", "double"),
            ("tan", "double"),
            ("floor", "double"),
            ("ceil", "double"),
            ("round", "double"),
        ];
        for (name, ret) in sigs {
            self.fn_ret_types.insert(name.to_string(), ret.to_string());
            if *ret == "const char**" {
                self.array_ret_fns.insert(name.to_string());
            }
        }
    }

    // ── 函数局部作用域 ──
    //
    // var_types / array_len_exprs / array_elem_types / array_params
    // 描述的都是「当前函数内部」的符号。Codegen 是单个实例、跨函数复用的，
    // 不清空就会串味：`out` 在 dbl() 里是数组参数，在 scaled() 里是局部
    // 数组，混在一起就分不清它是入参还是栈上变量 —— 而这两者的返回值
    // 处理完全不同（入参能直接 return，栈数组必须 malloc 拷贝）。
    pub(crate) fn begin_fn_scope(&mut self) {
        self.var_types.clear();
        self.array_len_exprs.clear();
        self.array_elem_types.clear();
        self.array_params.clear();
    }

    pub(crate) fn push_line(&mut self, s: &str) {
        self.output.push_str(&"    ".repeat(self.indent));
        self.output.push_str(s);
        self.output.push('\n');
    }

    // ── 泛型调用预扫描 ──

    fn collect_generic_calls(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Fn(f) => {
                    if !f.generics.is_empty() {
                        continue;
                    }
                    for stmt in &f.body.stmts {
                        self.walk_stmt_for_generics(stmt);
                    }
                }
                Item::Struct(s) => {
                    for m in &s.methods {
                        for stmt in &m.body.stmts {
                            self.walk_stmt_for_generics(stmt);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn walk_stmt_for_generics(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(e) => self.walk_expr_for_generics(e),
            Stmt::Return(Some(e)) => self.walk_expr_for_generics(e),
            Stmt::Let { init, .. } => self.walk_expr_for_generics(init),
            Stmt::Assign { target, value } => {
                self.walk_expr_for_generics(target);
                self.walk_expr_for_generics(value);
            }
            Stmt::If { cond, then, els } => {
                self.walk_expr_for_generics(cond);
                for s in &then.stmts {
                    self.walk_stmt_for_generics(s);
                }
                if let Some(e) = els {
                    self.walk_stmt_for_generics(e);
                }
            }
            Stmt::While { cond, body } => {
                self.walk_expr_for_generics(cond);
                for s in &body.stmts {
                    self.walk_stmt_for_generics(s);
                }
            }
            Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
                self.walk_expr_for_generics(iter);
                for s in &body.stmts {
                    self.walk_stmt_for_generics(s);
                }
            }
            _ => {}
        }
    }

    fn walk_expr_for_generics(&mut self, expr: &Expr) {
        match expr {
            Expr::Call(callee, args) => {
                // 泛型调用 → 入队
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.generic_fns.contains_key(fname) {
                        let fname = fname.clone();
                        let _ = self.monomorphize_call(&fname, args);
                    }
                }
                self.walk_expr_for_generics(callee);
                for a in args {
                    self.walk_expr_for_generics(a);
                }
            }
            Expr::Binary(l, _, r) => {
                self.walk_expr_for_generics(l);
                self.walk_expr_for_generics(r);
            }
            Expr::Unary(_, e) => self.walk_expr_for_generics(e),
            Expr::Field(o, _) => self.walk_expr_for_generics(o),
            Expr::Index(o, i) => {
                self.walk_expr_for_generics(o);
                self.walk_expr_for_generics(i);
            }
            Expr::StructLit(_, fields) => {
                for (_, v) in fields {
                    self.walk_expr_for_generics(v);
                }
            }
            Expr::List(items) => {
                for e in items {
                    self.walk_expr_for_generics(e);
                }
            }
            Expr::Tuple(items) => {
                for e in items {
                    self.walk_expr_for_generics(e);
                }
            }
            Expr::If(cond, then, els) => {
                self.walk_expr_for_generics(cond);
                for s in &then.stmts {
                    self.walk_stmt_for_generics(s);
                }
                if let Some(b) = els {
                    for s in &b.stmts {
                        self.walk_stmt_for_generics(s);
                    }
                }
            }
            Expr::Lambda { body, .. } => {
                for s in &body.stmts {
                    self.walk_stmt_for_generics(s);
                }
            }
            _ => {}
        }
    }
}
