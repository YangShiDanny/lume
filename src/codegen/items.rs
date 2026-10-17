// src/codegen/items.rs
use super::{Codegen, is_array_ty};
use crate::ast::*;
use std::collections::HashMap;

impl Codegen {
    pub(crate) fn gen_struct_decl(&mut self, s: &StructDecl) {
        self.push_line(&format!("typedef struct {} {{", s.name));
        self.indent += 1;
        for f in &s.fields {
            let c_ty = self.gen_type(&f.ty);
            self.push_line(&format!("{} {};", c_ty, f.name));
        }
        self.indent -= 1;
        self.push_line(&format!("}} {};", s.name));
        self.output.push('\n');
    }

    // ── 参数生成 ──
    //
    // 数组/列表参数统一加 LUME_RESTRICT。
    // Lume 里数组是值语义、没有裸指针，两个数组参数不可能重叠，
    // 这个信息只有编译器知道 —— 加上之后 clang 才能放心把循环向量化。
    //
    // 数组参数还会额外带一个 `int64_t {name}_len` 伴随参数（见 gen_params）。
    // 这是「数组跨函数」的关键：长度跟着数组一起走，调用点由编译器自动补，
    // 用户不用再手写一个 n，也不用担心它和数组对不上。

    pub(crate) fn gen_param(&self, p: &Param) -> String {
        let ty = match &p.ty {
            Some(t) => self.gen_type(t),
            None => "int64_t".to_string(),
        };
        if is_array_ty(&p.ty) {
            format!("{} LUME_RESTRICT {}", ty, p.name)
        } else {
            format!("{} {}", ty, p.name)
        }
    }

    pub(crate) fn gen_param_subst(
        &self,
        p: &Param,
        type_map: &HashMap<String, String>,
    ) -> String {
        let ty = match &p.ty {
            Some(t) => self.gen_type_subst(t, type_map),
            None => "int64_t".to_string(),
        };
        if is_array_ty(&p.ty) {
            format!("{} LUME_RESTRICT {}", ty, p.name)
        } else {
            format!("{} {}", ty, p.name)
        }
    }

    /// 完整参数列表：每个数组参数展开成「指针 + 伴随长度」两项，
    /// 伴随参数紧跟在指针后面。声明和调用点必须走同一条规则，
    /// 否则 C 原型和实参对不上。
    pub(crate) fn gen_params(&self, params: &[Param]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in params {
            out.push(self.gen_param(p));
            if is_array_ty(&p.ty) {
                out.push(format!("int64_t {}_len", p.name));
            }
        }
        out
    }

    pub(crate) fn gen_params_subst(
        &self,
        params: &[Param],
        type_map: &HashMap<String, String>,
    ) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in params {
            out.push(self.gen_param_subst(p, type_map));
            if is_array_ty(&p.ty) {
                out.push(format!("int64_t {}_len", p.name));
            }
        }
        out
    }

    // 数组参数登记：长度登记成伴随参数名 {name}_len，
    // 这样函数体里 a.len / for v in a 都能用
    pub(crate) fn register_array_params(&mut self, params: &[Param]) {
        for p in params {
            if let Some(Type::Array(inner, _)) | Some(Type::List(inner)) = &p.ty {
                self.array_params.insert(p.name.clone());
                self.array_elem_types
                    .insert(p.name.clone(), self.gen_type(inner));
                self.array_len_exprs
                    .insert(p.name.clone(), format!("{}_len", p.name));
            }
        }
    }

    // 纯函数标记：只有被证明过的函数才带 LUME_PURE
    fn pure_attr(&self, name: &str) -> &'static str {
        if self.const_fns.contains(name) {
            "LUME_PURE "
        } else {
            ""
        }
    }

    // ── 前向声明 ──
    // 让函数/方法可以互相引用，不必按定义顺序排列

    pub(crate) fn gen_method_decl(&mut self, struct_name: &str, m: &FnDecl) {
        let c_name = format!("{}_{}", struct_name, m.name);
        let ret_type = match &m.ret {
            None => "void".to_string(),
            Some(t) => self.gen_type(t),
        };
        let mut params = vec![format!("{}* _self", struct_name)];
        params.extend(self.gen_params(&m.params));
        if is_array_ty(&m.ret) {
            params.push("int64_t* _ret_len".to_string());
        }
        self.push_line(&format!(
            "static {} {}({});",
            ret_type, c_name, params.join(", ")
        ));
    }

    pub(crate) fn gen_fn_decl(&mut self, f: &FnDecl) {
        let is_main = f.name == "main";
        let ret_type = match &f.ret {
            None => if is_main { "int".to_string() } else { "void".to_string() },
            Some(t) => self.gen_type(t),
        };
        let mut params = self.gen_params(&f.params);
        if is_array_ty(&f.ret) {
            params.push("int64_t* _ret_len".to_string());
        }
        let params_str = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };
        // main 不能是 static
        let prefix = if is_main { "" } else { "static " };
        self.push_line(&format!(
            "{}{}{} {}({});",
            prefix,
            self.pure_attr(&f.name),
            ret_type,
            f.name,
            params_str
        ));
    }

    pub(crate) fn gen_method(&mut self, struct_name: &str, m: &FnDecl) {
        let c_name = format!("{}_{}", struct_name, m.name);
        let ret_type = match &m.ret {
            None => "void".to_string(),
            Some(t) => self.gen_type(t),
        };

        let mut params = vec![format!("{}* _self", struct_name)];
        params.extend(self.gen_params(&m.params));
        if is_array_ty(&m.ret) {
            params.push("int64_t* _ret_len".to_string());
        }

        self.push_line(&format!("static {} {}({}) {{", ret_type, c_name, params.join(", ")));
        self.indent += 1;
        self.cur_ret_is_array = is_array_ty(&m.ret);

        for p in &m.params {
            let ty = match &p.ty {
                Some(t) => self.gen_type(t),
                None => "int64_t".to_string(),
            };
            self.var_types.insert(p.name.clone(), ty);
        }
        // 方法里的数组参数也要登记，否则 par for 认不出来
        self.register_array_params(&m.params);

        let field_names: Vec<String> = self.structs.get(struct_name)
            .map(|f| f.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();

        for stmt in &m.body.stmts {
            self.gen_stmt_with_self(stmt, &field_names);
        }

        self.indent -= 1;
        self.push_line("}");
        self.output.push('\n');
    }

    pub(crate) fn gen_fn(&mut self, f: &FnDecl) {
        // 登记数组参数（供 par for 收集、restrict 生效）
        self.register_array_params(&f.params);

        if !f.generics.is_empty() {
            return;
        }

        if f.decorators.iter().any(|d| d.name == "precompute") {
            self.gen_precompute_fn(f);
            return;
        }

        if f.decorators.iter().any(|d| d.name == "cache") {
            self.gen_cached_fn(f);
            return;
        }

        let is_main = f.name == "main";

        let ret_type = match &f.ret {
            None => if is_main { "int".to_string() } else { "void".to_string() },
            Some(t) => self.gen_type(t),
        };

        let mut params = self.gen_params(&f.params);
        if is_array_ty(&f.ret) {
            params.push("int64_t* _ret_len".to_string());
        }

        let params_str = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };

        let prefix = if is_main { "" } else { "static " };
        self.push_line(&format!(
            "{}{}{} {}({}) {{",
            prefix,
            self.pure_attr(&f.name),
            ret_type,
            f.name,
            params_str
        ));
        self.indent += 1;
        self.cur_ret_is_array = is_array_ty(&f.ret);

        for p in &f.params {
            let ty = match &p.ty {
                Some(t) => self.gen_type(t),
                None => "int64_t".to_string(),
            };
            self.var_types.insert(p.name.clone(), ty);
        }

        for stmt in &f.body.stmts {
            self.gen_stmt(stmt);
        }

        if is_main {
            self.push_line("return 0;");
        }

        self.indent -= 1;
        self.push_line("}");
        self.output.push('\n');
    }

    // ── 泛型单态化 ──

    // 只生成声明（用于前向声明）
    pub(crate) fn gen_monomorphized_decl(
        &mut self,
        f: &FnDecl,
        mono_name: &str,
        type_map: &HashMap<String, String>,
    ) {
        let ret_type = match &f.ret {
            None => "void".to_string(),
            Some(t) => self.gen_type_subst(t, type_map),
        };

        let mut params = self.gen_params_subst(&f.params, type_map);
        if is_array_ty(&f.ret) {
            params.push("int64_t* _ret_len".to_string());
        }

        self.push_line(&format!(
            "{} {}({});",
            ret_type, mono_name, params.join(", ")
        ));
    }

    // 生成完整定义
    pub(crate) fn gen_monomorphized(
        &mut self,
        f: &FnDecl,
        mono_name: &str,
        type_map: &HashMap<String, String>,
    ) {
        let ret_type = match &f.ret {
            None => "void".to_string(),
            Some(t) => self.gen_type_subst(t, type_map),
        };

        let mut params = self.gen_params_subst(&f.params, type_map);
        if is_array_ty(&f.ret) {
            params.push("int64_t* _ret_len".to_string());
        }

        self.push_line(&format!(
            "{}{} {}({}) {{",
            self.pure_attr(&f.name), ret_type, mono_name, params.join(", ")
        ));
        self.indent += 1;

        for p in &f.params {
            let ty = match &p.ty {
                Some(t) => self.gen_type_subst(t, type_map),
                None => "int64_t".to_string(),
            };
            self.var_types.insert(p.name.clone(), ty);
        }
        self.register_array_params(&f.params);

        for stmt in &f.body.stmts {
            self.gen_stmt(stmt);
        }

        self.indent -= 1;
        self.push_line("}");
        self.output.push('\n');
    }

    // 推断泛型类型 + 入队
    pub(crate) fn monomorphize_call(&mut self, fname: &str, args: &[Expr]) -> String {
        let generic_fn = match self.generic_fns.get(fname).cloned() {
            Some(f) => f,
            None => return format!("/* unknown generic {} */", fname),
        };

        // 推断类型
        let mut type_map: HashMap<String, String> = HashMap::new();
        for (i, param) in generic_fn.params.iter().enumerate() {
            if let Some(arg) = args.get(i) {
                let arg_ty = self.infer_type(arg);
                if let Some(Type::Named(tname)) = &param.ty {
                    if generic_fn.generics.contains(tname) {
                        type_map.insert(tname.clone(), arg_ty);
                    }
                }
            }
        }

        for g in &generic_fn.generics {
            type_map.entry(g.clone())
                .or_insert_with(|| "int64_t".to_string());
        }

        // 唯一名字
        let suffix: String = generic_fn.generics.iter()
            .map(|g| type_map.get(g).cloned().unwrap_or_else(|| "unknown".to_string()))
            .collect::<Vec<_>>()
            .join("_");
        let mono_name = format!("{}_{}", fname, suffix);

        if !self.monomorphized.contains_key(&mono_name) {
            self.monomorphized.insert(mono_name.clone(), mono_name.clone());
            // 入队，不立即生成
            self.pending_monos.push((
                generic_fn.clone(),
                mono_name.clone(),
                type_map.clone(),
            ));
        }

        // 数组实参后面补长度，和 gen_params_subst 展开出来的伴随参数对齐
        let mut args_str: Vec<String> = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            args_str.push(self.gen_arg(a));
            let is_arr = generic_fn
                .params
                .get(i)
                .map(|p| is_array_ty(&p.ty))
                .unwrap_or(false);
            if is_arr {
                match self.array_len_of(a) {
                    Some(len) => args_str.push(len),
                    None => args_str.push(format!(
                        "/* 无法确定第 {} 个实参的数组长度 */0LL",
                        i + 1
                    )),
                }
            }
        }
        format!("{}({})", mono_name, args_str.join(", "))
    }

    // ── 缓存：开放寻址哈希表，查找 O(1) ──

    fn cache_index(&self, key: &str, cap: &str) -> String {
        format!(
            "(int64_t)((((uint64_t)(int64_t)({}) * 11400714819323198485ULL) >> 32) & (uint64_t)(({}) - 1))",
            key, cap
        )
    }

    fn emit_cache_state(&mut self, fname: &str, ret_type: &str) {
        self.push_line(&format!("static int64_t _cache_{}_len = 0;", fname));
        self.push_line(&format!("static int64_t _cache_{}_cap = 0;", fname));
        self.push_line(&format!("static int64_t* _cache_{}_keys = NULL;", fname));
        self.push_line(&format!(
            "static {}* _cache_{}_values = NULL;",
            ret_type, fname
        ));
        self.push_line(&format!("static unsigned char* _cache_{}_used = NULL;", fname));
    }

    // 扩容 + 重哈希（容量始终是 2 的幂）
    fn emit_cache_grow(&mut self, fname: &str, ret_type: &str) {
        let keys = format!("_cache_{}_keys", fname);
        let values = format!("_cache_{}_values", fname);
        let used = format!("_cache_{}_used", fname);
        let cap = format!("_cache_{}_cap", fname);

        self.push_line(&format!("static void _cache_{}_grow(void) {{", fname));
        self.indent += 1;
        self.push_line(&format!("int64_t _newcap = {} == 0 ? 64 : {} * 2;", cap, cap));
        self.push_line("int64_t* _nk = (int64_t*)malloc((size_t)_newcap * sizeof(int64_t));");
        self.push_line(&format!(
            "{}* _nv = ({}*)malloc((size_t)_newcap * sizeof({}));",
            ret_type, ret_type, ret_type
        ));
        self.push_line("unsigned char* _nu = (unsigned char*)calloc((size_t)_newcap, 1);");
        self.push_line(&format!("for (int64_t _i = 0; _i < {}; _i++) {{", cap));
        self.indent += 1;
        self.push_line(&format!("if (!{}[_i]) continue;", used));
        self.push_line(&format!(
            "int64_t _j = {};",
            self.cache_index(&format!("{}[_i]", keys), "_newcap")
        ));
        self.push_line("while (_nu[_j]) { _j = (_j + 1) & (_newcap - 1); }");
        self.push_line(&format!("_nk[_j] = {}[_i];", keys));
        self.push_line(&format!("_nv[_j] = {}[_i];", values));
        self.push_line("_nu[_j] = 1;");
        self.indent -= 1;
        self.push_line("}");
        self.push_line(&format!("free({});", keys));
        self.push_line(&format!("free({});", values));
        self.push_line(&format!("free({});", used));
        self.push_line(&format!("{} = _nk;", keys));
        self.push_line(&format!("{} = _nv;", values));
        self.push_line(&format!("{} = _nu;", used));
        self.push_line(&format!("{} = _newcap;", cap));
        self.indent -= 1;
        self.push_line("}");
    }

    // 探测：命中就执行 on_hit（其中 %V% 会被替换成缓存值），
    // 未命中时把空槽下标留在 idx_var 里供插入使用
    fn emit_cache_probe(
        &mut self,
        fname: &str,
        pname: &str,
        idx_var: &str,
        on_hit: &[String],
    ) {
        let keys = format!("_cache_{}_keys", fname);
        let values = format!("_cache_{}_values", fname);
        let used = format!("_cache_{}_used", fname);
        let cap = format!("_cache_{}_cap", fname);
        let val = format!("{}[{}]", values, idx_var);

        self.push_line(&format!(
            "int64_t {} = {};",
            idx_var,
            self.cache_index(pname, &cap)
        ));
        self.push_line(&format!("while ({}[{}]) {{", used, idx_var));
        self.indent += 1;
        self.push_line(&format!("if ({}[{}] == {}) {{", keys, idx_var, pname));
        self.indent += 1;
        for line in on_hit {
            self.push_line(&line.replace("%V%", &val));
        }
        self.indent -= 1;
        self.push_line("}");
        self.push_line(&format!("{} = ({} + 1) & ({} - 1);", idx_var, idx_var, cap));
        self.indent -= 1;
        self.push_line("}");
    }

    // ── @cache（单线程，无锁）──

    fn gen_cached_fn(&mut self, f: &FnDecl) {
        if f.params.len() != 1 || is_array_ty(&f.params[0].ty) {
            self.push_line(&format!(
                "/* @cache 需要单个标量参数，退化成普通函数: {} */",
                f.name
            ));
            self.gen_fn_decl(f);
            self.gen_fn_body_impl(f, &f.name);
            return;
        }

        let ret_type = match &f.ret {
            None => "void".to_string(),
            Some(t) => self.gen_type(t),
        };
        if ret_type == "void" {
            // 没有返回值，缓存没有意义：退化成普通函数
            self.push_line(&format!("/* @cache: {} 无返回值，跳过缓存 */", f.name));
            self.gen_fn(f);
            return;
        }

        let fname = f.name.clone();
        let impl_name = format!("_{}_impl", fname);
        let pname = f.params[0].name.clone();
        let pdecl = self.gen_param(&f.params[0]);
        let cap = format!("_cache_{}_cap", fname);
        let keys = format!("_cache_{}_keys", fname);
        let values = format!("_cache_{}_values", fname);
        let used = format!("_cache_{}_used", fname);
        let len = format!("_cache_{}_len", fname);

        self.output.push('\n');
        self.push_line(&format!("static {} {}({});", ret_type, fname, pdecl));
        self.emit_cache_state(&fname, &ret_type);
        self.emit_cache_grow(&fname, &ret_type);
        self.gen_fn_body_impl(f, &impl_name);

        self.push_line(&format!(
            "static {} {}({}) {{",
            ret_type, fname, pdecl
        ));
        self.indent += 1;

        // 命中直接返回
        self.push_line(&format!("if ({} > 0) {{", len));
        self.indent += 1;
        self.emit_cache_probe(&fname, &pname, "_j", &["return %V%;".to_string()]);
        self.indent -= 1;
        self.push_line("}");

        self.push_line(&format!("{} _result = {}({});", ret_type, impl_name, pname));

        // 负载因子 0.75 时扩容
        self.push_line(&format!(
            "if (({} + 1) * 4 >= {} * 3) _cache_{}_grow();",
            len, cap, fname
        ));
        self.emit_cache_probe(
            &fname,
            &pname,
            "_j2",
            &["_result = %V%;".to_string(), "return _result;".to_string()],
        );

        self.push_line(&format!("{}[_j2] = {};", keys, pname));
        self.push_line(&format!("{}[_j2] = _result;", values));
        self.push_line(&format!("{}[_j2] = 1;", used));
        self.push_line(&format!("{}++;", len));
        self.push_line("return _result;");

        self.indent -= 1;
        self.push_line("}");
        self.output.push('\n');
    }

    // ── 线程安全版（用于 @precompute 与预计算程序）──

    pub(crate) fn gen_thread_safe_cached_fn(&mut self, f: &FnDecl) {
        if f.params.len() != 1 || is_array_ty(&f.params[0].ty) {
            self.push_line(&format!(
                "/* 预计算需要单个标量参数，退化成普通函数: {} */",
                f.name
            ));
            self.gen_fn_decl(f);
            self.gen_fn_body_impl(f, &f.name);
            return;
        }

        let ret_type = match &f.ret {
            None => "void".to_string(),
            Some(t) => self.gen_type(t),
        };
        if ret_type == "void" {
            self.push_line(&format!(
                "/* 预计算暂只支持有返回值的函数: {} */",
                f.name
            ));
            return;
        }

        let fname = f.name.clone();
        let impl_name = format!("_{}_impl", fname);
        let mutex = format!("_cache_{}_mutex", fname);
        let pname = f.params[0].name.clone();
        let pdecl = self.gen_param(&f.params[0]);
        let cap = format!("_cache_{}_cap", fname);
        let keys = format!("_cache_{}_keys", fname);
        let values = format!("_cache_{}_values", fname);
        let used = format!("_cache_{}_used", fname);
        let len = format!("_cache_{}_len", fname);

        self.output.push('\n');
        self.push_line(&format!("static {} {}({});", ret_type, fname, pdecl));
        self.push_line(&format!("static LUME_MUTEX_T {} = LUME_MUTEX_INIT;", mutex));
        self.emit_cache_state(&fname, &ret_type);
        self.emit_cache_grow(&fname, &ret_type);
        self.gen_fn_body_impl(f, &impl_name);

        self.push_line(&format!(
            "static {} {}({}) {{",
            ret_type, fname, pdecl
        ));
        self.indent += 1;

        // 查表（持锁）
        self.push_line(&format!("LUME_MUTEX_LOCK({});", mutex));
        self.push_line(&format!("if ({} > 0) {{", len));
        self.indent += 1;
        self.emit_cache_probe(
            &fname,
            &pname,
            "_j",
            &[
                format!("{} _v = %V%;", ret_type),
                format!("LUME_MUTEX_UNLOCK({});", mutex),
                "return _v;".to_string(),
            ],
        );
        self.indent -= 1;
        self.push_line("}");
        self.push_line(&format!("LUME_MUTEX_UNLOCK({});", mutex));

        self.push_line(&format!("{} _result = {}({});", ret_type, impl_name, pname));

        // 写回（持锁）
        self.push_line(&format!("LUME_MUTEX_LOCK({});", mutex));
        self.push_line(&format!(
            "if (({} + 1) * 4 >= {} * 3) _cache_{}_grow();",
            len, cap, fname
        ));
        self.emit_cache_probe(
            &fname,
            &pname,
            "_j2",
            &[
                "_result = %V%;".to_string(),
                format!("LUME_MUTEX_UNLOCK({});", mutex),
                "return _result;".to_string(),
            ],
        );
        self.push_line(&format!("{}[_j2] = {};", keys, pname));
        self.push_line(&format!("{}[_j2] = _result;", values));
        self.push_line(&format!("{}[_j2] = 1;", used));
        self.push_line(&format!("{}++;", len));
        self.push_line(&format!("LUME_MUTEX_UNLOCK({});", mutex));
        self.push_line("return _result;");

        self.indent -= 1;
        self.push_line("}");
        self.output.push('\n');
    }

    // ── @precompute ──

    fn gen_precompute_fn(&mut self, f: &FnDecl) {
        if f.params.len() != 1 || is_array_ty(&f.params[0].ty) {
            self.push_line(&format!(
                "/* @precompute 需要单个标量参数，退化成普通函数: {} */",
                f.name
            ));
            self.gen_fn_decl(f);
            self.gen_fn_body_impl(f, &f.name);
            return;
        }

        let fname = f.name.clone();
        let table_name = format!("_{}_table", fname);

        let ret_type = match &f.ret {
            None => "int64_t".to_string(),
            Some(t) => self.gen_type(t),
        };

        let precompute = f.decorators.iter()
            .find(|d| d.name == "precompute").unwrap();
        let (start, end) = match (precompute.args.get(0), precompute.args.get(1)) {
            (Some(Expr::Int(s)), Some(Expr::Int(e))) => (*s, *e),
            _ => {
                self.push_line(&format!(
                    "/* @precompute 参数必须是两个整数: {} */",
                    fname
                ));
                return;
            }
        };

        self.output.push('\n');

        // 常量表（编译期算好的）
        let has_table = if let Some(table) = self.precomputed.get(&fname).cloned() {
            let len = table.len();
            self.push_line(&format!(
                "static const {} {}[{}] = {{",
                ret_type, table_name, len
            ));
            self.indent += 1;
            for chunk in table.chunks(10) {
                let line: Vec<String> = chunk.iter()
                    .map(|v| format!("{}LL", v))
                    .collect();
                self.push_line(&format!("{},", line.join(", ")));
            }
            self.indent -= 1;
            self.push_line("};");
            self.output.push('\n');
            true
        } else {
            self.push_line(&format!(
                "/* @precompute 表未生成，回退到运行时缓存: {} */",
                fname
            ));
            false
        };

        // 超出范围时走线程安全缓存（复用同一套状态）
        self.gen_thread_safe_cached_fn_body(f, &ret_type, has_table, start, end, &table_name);
    }

    // @precompute 的包装函数：先查常量表，再走缓存
    fn gen_thread_safe_cached_fn_body(
        &mut self,
        f: &FnDecl,
        ret_type: &str,
        has_table: bool,
        start: i64,
        end: i64,
        table_name: &str,
    ) {
        let fname = f.name.clone();
        let impl_name = format!("_{}_impl", fname);
        let mutex = format!("_cache_{}_mutex", fname);
        let pname = f.params[0].name.clone();
        let pdecl = self.gen_param(&f.params[0]);
        let cap = format!("_cache_{}_cap", fname);
        let keys = format!("_cache_{}_keys", fname);
        let values = format!("_cache_{}_values", fname);
        let used = format!("_cache_{}_used", fname);
        let len = format!("_cache_{}_len", fname);

        self.push_line(&format!("static {} {}({});", ret_type, fname, pdecl));
        self.push_line(&format!("static LUME_MUTEX_T {} = LUME_MUTEX_INIT;", mutex));
        self.emit_cache_state(&fname, ret_type);
        self.emit_cache_grow(&fname, ret_type);
        self.gen_fn_body_impl(f, &impl_name);

        self.push_line(&format!(
            "static {} {}({}) {{",
            ret_type, fname, pdecl
        ));
        self.indent += 1;

        if has_table {
            self.push_line(&format!(
                "if ({} >= {}LL && {} <= {}LL) {{",
                pname, start, pname, end
            ));
            self.indent += 1;
            self.push_line(&format!(
                "return {}[{} - {}LL];",
                table_name, pname, start
            ));
            self.indent -= 1;
            self.push_line("}");
        }

        self.push_line(&format!("LUME_MUTEX_LOCK({});", mutex));
        self.push_line(&format!("if ({} > 0) {{", len));
        self.indent += 1;
        self.emit_cache_probe(
            &fname,
            &pname,
            "_j",
            &[
                format!("{} _v = %V%;", ret_type),
                format!("LUME_MUTEX_UNLOCK({});", mutex),
                "return _v;".to_string(),
            ],
        );
        self.indent -= 1;
        self.push_line("}");
        self.push_line(&format!("LUME_MUTEX_UNLOCK({});", mutex));

        self.push_line(&format!("{} _result = {}({});", ret_type, impl_name, pname));

        self.push_line(&format!("LUME_MUTEX_LOCK({});", mutex));
        self.push_line(&format!(
            "if (({} + 1) * 4 >= {} * 3) _cache_{}_grow();",
            len, cap, fname
        ));
        self.emit_cache_probe(
            &fname,
            &pname,
            "_j2",
            &[
                "_result = %V%;".to_string(),
                format!("LUME_MUTEX_UNLOCK({});", mutex),
                "return _result;".to_string(),
            ],
        );
        self.push_line(&format!("{}[_j2] = {};", keys, pname));
        self.push_line(&format!("{}[_j2] = _result;", values));
        self.push_line(&format!("{}[_j2] = 1;", used));
        self.push_line(&format!("{}++;", len));
        self.push_line(&format!("LUME_MUTEX_UNLOCK({});", mutex));
        self.push_line("return _result;");

        self.indent -= 1;
        self.push_line("}");
        self.output.push('\n');
    }

    pub(crate) fn gen_fn_body_impl(&mut self, f: &FnDecl, impl_name: &str) {
        let ret_type = match &f.ret {
            None => "void".to_string(),
            Some(t) => self.gen_type(t),
        };

        let mut params = self.gen_params(&f.params);
        if is_array_ty(&f.ret) {
            params.push("int64_t* _ret_len".to_string());
        }

        let params_str = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };

        self.push_line(&format!("static {} {}({}) {{", ret_type, impl_name, params_str));
        self.indent += 1;
        self.cur_ret_is_array = is_array_ty(&f.ret);

        for p in &f.params {
            let ty = match &p.ty {
                Some(t) => self.gen_type(t),
                None => "int64_t".to_string(),
            };
            self.var_types.insert(p.name.clone(), ty);
        }
        self.register_array_params(&f.params);

        for stmt in &f.body.stmts {
            self.gen_stmt(stmt);
        }

        self.indent -= 1;
        self.push_line("}");
        self.output.push('\n');
    }

    pub(crate) fn gen_const(&mut self, c: &ConstDecl) {
        let ty = match &c.ty {
            Some(t) => self.gen_type(t),
            None => "int64_t".to_string(),
        };
        let value = self.gen_expr(&c.value);
        self.push_line(&format!("const {} {} = {};", ty, c.name, value));
    }
}
