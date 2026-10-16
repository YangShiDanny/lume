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

pub struct Codegen {
    pub(crate) output: String,
    pub(crate) indent: usize,
    pub(crate) var_types: HashMap<String, String>,
    pub(crate) structs: HashMap<String, Vec<(String, String)>>,
    pub(crate) methods: HashMap<(String, String), String>,
    pub(crate) array_lens: HashMap<String, usize>,
    pub(crate) array_elem_types: HashMap<String, String>,
    // 数组/列表参数名（长度编译期未知，不能当成已知长度用）
    pub(crate) array_params: HashSet<String>,
    // 已证明为纯函数、可以打 __attribute__((const)) 的函数名
    pub(crate) const_fns: HashSet<String>,
    pub(crate) precomputed: HashMap<String, Vec<i64>>,
    pub(crate) generic_fns: HashMap<String, FnDecl>,
    pub(crate) monomorphized: HashMap<String, String>,
    pub(crate) pending_monos: Vec<(FnDecl, String, HashMap<String, String>)>,
    pub(crate) par_counter: usize,
    // par for 的 worker 函数（提到文件顶层，不能嵌在函数里）
    pub(crate) par_workers: String,
    pub(crate) fastmath: bool,
}

impl Codegen {
    pub fn new(fastmath: bool) -> Self {
        Self {
            output: String::new(),
            indent: 0,
            var_types: HashMap::new(),
            structs: HashMap::new(),
            methods: HashMap::new(),
            array_lens: HashMap::new(),
            array_elem_types: HashMap::new(),
            array_params: HashSet::new(),
            const_fns: HashSet::new(),
            precomputed: HashMap::new(),
            generic_fns: HashMap::new(),
            monomorphized: HashMap::new(),
            pending_monos: Vec::new(),
            par_counter: 0,
            par_workers: String::new(),
            fastmath,
        }
    }

    pub fn set_precomputed(&mut self, name: String, table: Vec<i64>) {
        self.precomputed.insert(name, table);
    }

    pub fn generate(&mut self, program: &Program) -> String {
        self.output.push_str(runtime::RUNTIME_HEADER);
        self.output.push('\n');

        // 1. 收集 struct 定义和方法签名
        for item in &program.items {
            if let Item::Struct(s) = item {
                let fields: Vec<(String, String)> = s.fields.iter()
                    .map(|f| (f.name.clone(), self.gen_type(&f.ty)))
                    .collect();
                self.structs.insert(s.name.clone(), fields);

                for m in &s.methods {
                    let ret = match &m.ret {
                        None => "void".to_string(),
                        Some(t) => self.gen_type(t),
                    };
                    self.methods.insert((s.name.clone(), m.name.clone()), ret);
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
        self.push_line("printf(\"%lld\\n\", (long long)_results[i]);");
        self.indent -= 1;
        self.push_line("}");
        self.push_line("free(_results);");
        self.push_line("return 0;");

        self.indent -= 1;
        self.push_line("}");

        std::mem::take(&mut self.output)
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