// src/codegen/expr.rs
use super::Codegen;
use crate::ast::*;

impl Codegen {
    pub(crate) fn gen_expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Int(n) => format!("{}LL", n),
            Expr::Float(f) => {
                let s = format!("{}", f);
                if s.contains('.') || s.contains('e') {
                    s
                } else {
                    format!("{}.0", s)
                }
            }
            Expr::Str(s) => {
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{}\"", escaped)
            }
            // 带插值的字符串出现在非 say 位置时，退化成纯文本拼接
            Expr::FmtStr(parts) => {
                let mut s = String::new();
                for p in parts {
                    if let StrPart::Text(t) = p {
                        s.push_str(t);
                    }
                }
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{}\"", escaped)
            }
            Expr::Rune(c) => format!("'{}'", c),
            Expr::Color(c) => format!("0x{:06X}", c),
            Expr::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
            Expr::Nothing => "NULL".to_string(),
            Expr::Ident(name) => name.clone(),
            Expr::It => "_it".to_string(),

            Expr::Binary(l, op, r) => {
                let ls = self.gen_expr(l);
                let rs = self.gen_expr(r);
                let op_str = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Lt => "<",
                    BinOp::Gt => ">",
                    BinOp::Le => "<=",
                    BinOp::Ge => ">=",
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                    BinOp::Pipe => {
                        // a |> f()   →  f(a)
                        // a |> f(b)  →  f(a, b)
                        let mut pipe_args = vec![ls];
                        let callee = match r.as_ref() {
                            Expr::Call(callee, args) => {
                                for a in args {
                                    pipe_args.push(self.gen_expr(a));
                                }
                                self.gen_expr(callee)
                            }
                            other => self.gen_expr(other),
                        };
                        return format!("{}({})", callee, pipe_args.join(", "));
                    }
                };
                format!("({} {} {})", ls, op_str, rs)
            }

            Expr::Unary(op, e) => {
                let es = self.gen_expr(e);
                let op_str = match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                };
                format!("({}{})", op_str, es)
            }

            Expr::Call(callee, args) => {
                // 内置函数
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "say" {
                        return self.gen_say(args);
                    }
                    if name == "now" {
                        return "lume_now_ns()".to_string();
                    }
                    if let Some(c_name) = builtin_map(name) {
                        let args_str: Vec<String> = args.iter()
                            .map(|a| self.gen_expr(a))
                            .collect();
                        return format!("{}({})", c_name, args_str.join(", "));
                    }
                }

                // a.len() —— 数组长度。
                // 局部字面量是常量，数组参数是伴随参数 {name}_len
                if let Expr::Field(obj, method) = callee.as_ref() {
                    if method == "len" && args.is_empty() {
                        if let Some(len) = self.array_len_of(obj) {
                            return len;
                        }
                    }
                }

                // 泛型调用
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.generic_fns.contains_key(fname) {
                        return self.monomorphize_call(fname, args);
                    }
                }

                // 方法调用 p.len()
                if let Expr::Field(obj, method_name) = callee.as_ref() {
                    if let Expr::Ident(obj_name) = obj.as_ref() {
                        if let Some(struct_name) = self.var_types.get(obj_name).cloned() {
                            let key = (struct_name.clone(), method_name.clone());
                            if self.methods.contains_key(&key) {
                                let arr_flags = self
                                    .method_array_params
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or_default();
                                let mut all_args = vec![format!("&{}", obj_name)];
                                for (i, a) in args.iter().enumerate() {
                                    all_args.push(self.gen_expr(a));
                                    if arr_flags.get(i).copied().unwrap_or(false) {
                                        match self.array_len_of(a) {
                                            Some(len) => all_args.push(len),
                                            None => all_args.push(format!(
                                                "/* 无法确定第 {} 个实参的数组长度 */0LL",
                                                i + 1
                                            )),
                                        }
                                    }
                                }
                                return format!(
                                    "{}_{}({})",
                                    struct_name, method_name, all_args.join(", ")
                                );
                            }
                        }
                    }
                }

                // 普通调用。数组实参后面自动补一个长度实参，
                // 和被调用函数的伴随参数一一对应（见 gen_params）
                let callee_str = self.gen_expr(callee);
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.array_ret_fns.contains(fname) {
                        return format!(
                            "/* {}() 返回数组，请写成 `let r = {}(...)` 才能拿到长度 */",
                            fname, fname
                        );
                    }
                }
                let arg_strs = self.gen_call_args(callee, args, None);
                format!("{}({})", callee_str, arg_strs.join(", "))
            }

            Expr::Field(obj, name) => {
                // a.len —— 数组长度（局部字面量和数组参数都支持）
                if name == "len" {
                    if let Some(len) = self.array_len_of(obj) {
                        return len;
                    }
                }
                let o = self.gen_expr(obj);
                format!("{}.{}", o, name)
            }

            Expr::Index(obj, idx) => {
                let o = self.gen_expr(obj);
                let i = self.gen_expr(idx);
                format!("{}[{}]", o, i)
            }

            Expr::StructLit(name, fields) => {
                let field_strs: Vec<String> = fields.iter()
                    .map(|(fname, fval)| {
                        let v = self.gen_expr(fval);
                        format!(".{} = {}", fname, v)
                    })
                    .collect();
                format!("({}){{ {} }}", name, field_strs.join(", "))
            }

            Expr::List(items) => {
                let s: Vec<String> = items.iter().map(|e| self.gen_expr(e)).collect();
                format!("{{{}}}", s.join(", "))
            }

            _ => "/* TODO */".to_string(),
        }
    }

    /// 数组实参的长度表达式。
    /// 数组变量查 array_len_exprs；数组字面量直接用元素个数
    /// （C99 复合字面量的生命周期覆盖整次调用，作为实参是安全的）。
    pub(crate) fn array_len_of(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(n) => self.array_len_exprs.get(n).cloned(),
            Expr::List(items) => Some(format!("{}LL", items.len())),
            _ => None,
        }
    }

    /// 生成一次普通函数调用的实参列表，数组实参后面自动补长度。
    /// 声明处用 gen_params 展开伴随参数，两边规则必须一致。
    /// fields 非空表示在结构体方法里，表达式要走 with_self 版本。
    pub(crate) fn gen_call_args(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        fields: Option<&[String]>,
    ) -> Vec<String> {
        let arr_flags: Option<Vec<bool>> = match callee {
            Expr::Ident(fname) => self.fn_array_params.get(fname).cloned(),
            _ => None,
        };
        let mut out: Vec<String> = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let s = match fields {
                Some(f) => self.gen_expr_with_self(a, f),
                None => self.gen_expr(a),
            };
            out.push(s);
            let need_len = arr_flags
                .as_ref()
                .and_then(|fl| fl.get(i))
                .copied()
                .unwrap_or(false);
            if need_len {
                match self.array_len_of(a) {
                    Some(len) => out.push(len),
                    None => out.push(format!(
                        "/* 无法确定第 {} 个实参的数组长度 */0LL",
                        i + 1
                    )),
                }
            }
        }
        out
    }

    /// `let r = f(...)` 且 f 返回数组：C 函数多一个长度出参，
    /// 必须拆成「声明指针 + 声明长度 + 赋值」三行，长度才有地方落。
    /// 返回 None 表示不是这种情况，走普通 let。
    pub(crate) fn gen_array_let(
        &mut self,
        name: &str,
        init: &Expr,
        fields: Option<&[String]>,
    ) -> Option<Vec<String>> {
        let (callee, args) = match init {
            Expr::Call(c, a) => (c.as_ref(), a),
            _ => return None,
        };
        let fname = match callee {
            Expr::Ident(f) => f.clone(),
            _ => return None,
        };
        if !self.array_ret_fns.contains(&fname) {
            return None;
        }
        let elem_ty = self
            .fn_ret_types
            .get(&fname)
            .cloned()
            .unwrap_or_else(|| "int64_t*".to_string())
            .trim_end_matches('*')
            .to_string();
        let mut parts = self.gen_call_args(callee, args, fields);
        parts.push(format!("&{}_len", name));
        // 登记成长度和普通数组一样，后面 r.len / for v in r 都能用
        self.array_len_exprs
            .insert(name.to_string(), format!("{}_len", name));
        self.array_elem_types
            .insert(name.to_string(), elem_ty.clone());
        self.var_types
            .insert(name.to_string(), format!("{}*", elem_ty));
        Some(vec![
            format!("{}* {};", elem_ty, name),
            format!("int64_t {}_len;", name),
            format!("{} = {}({});", name, fname, parts.join(", ")),
        ])
    }

    /// 返回数组的两种合法形式：
    ///   return [1,2,3]  → malloc 拷贝（栈上的复合字面量离开函数就悬空）
    ///   return a        → 把入参数组直接传出去，长度一起带出去
    /// 其他形式返回 None，走普通 return
    pub(crate) fn gen_array_return(
        &mut self,
        e: &Expr,
        fields: Option<&[String]>,
    ) -> Option<Vec<String>> {
        match e {
            Expr::List(items) => {
                let elem_ty = if let Some(first) = items.first() {
                    self.infer_type(first)
                } else {
                    "int64_t".to_string()
                };
                let n = items.len();
                let mut out = vec![format!(
                    "{}* _r = ({}*)malloc((size_t){} * sizeof({}));",
                    elem_ty, elem_ty, n, elem_ty
                )];
                for (i, it) in items.iter().enumerate() {
                    let v = match fields {
                        Some(f) => self.gen_expr_with_self(it, f),
                        None => self.gen_expr(it),
                    };
                    out.push(format!("_r[{}] = {};", i, v));
                }
                out.push(format!("*_ret_len = {}LL;", n));
                out.push("return _r;".to_string());
                Some(out)
            }
            Expr::Ident(src) => {
                let len = self.array_len_exprs.get(src).cloned()?;
                if self.array_params.contains(src) {
                    // 入参数组指向调用方的内存，活得过函数返回，直接传出去
                    Some(vec![
                        format!("*_ret_len = {};", len),
                        format!("return {};", src),
                    ])
                } else {
                    // 局部数组在 C 里是栈上的 `T x[N]`，返回它就是悬空指针。
                    // 必须 malloc 拷一份。
                    let elem_ty = self.array_elem_types.get(src).cloned()
                        .unwrap_or_else(|| "int64_t".to_string());
                    Some(vec![
                        format!(
                            "{}* _r = ({}*)malloc((size_t)({}) * sizeof({}));",
                            elem_ty, elem_ty, len, elem_ty
                        ),
                        format!(
                            "for (int64_t _i = 0; _i < ({}); _i++) {{ _r[_i] = {}[_i]; }}",
                            len, src
                        ),
                        format!("*_ret_len = {};", len),
                        "return _r;".to_string(),
                    ])
                }
            }
            _ => None,
        }
    }

    pub(crate) fn gen_expr_with_self(&mut self, expr: &Expr, fields: &[String]) -> String {
        match expr {
            // `self` 单独作为值出现时（f(self)、return self），
            // Lume 的 struct 是值类型，传的应该是结构体本身而不是指针。
            // _self 是 Point*，解引用成 Point 才和 `fn f(p: Point)` 对得上。
            // 注意 self.x / 裸字段名走的是别的分支，不受这里影响。
            Expr::Ident(name) if name == "self" => "(*_self)".to_string(),
            Expr::Ident(name) if fields.contains(name) => {
                format!("_self->{}", name)
            }
            // 带插值的字符串出现在非 say 位置时，退化成纯文本拼接
            Expr::FmtStr(parts) => {
                let mut s = String::new();
                for p in parts {
                    if let StrPart::Text(t) = p {
                        s.push_str(t);
                    }
                }
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{}\"", escaped)
            }

            Expr::Call(callee, args) => {
                // say（方法里的打印，插值要走 with_self 版本）
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "say" {
                        return self.gen_say_with(args, Some(fields));
                    }
                }
                // now
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "now" {
                        return "lume_now_ns()".to_string();
                    }
                    if let Some(c_name) = builtin_map(name) {
                        let args_str: Vec<String> = args.iter()
                            .map(|a| self.gen_expr_with_self(a, fields))
                            .collect();
                        return format!("{}({})", c_name, args_str.join(", "));
                    }
                }

                // a.len() —— 数组长度。
                // 局部字面量是常量，数组参数是伴随参数 {name}_len
                if let Expr::Field(obj, method) = callee.as_ref() {
                    if method == "len" && args.is_empty() {
                        if let Some(len) = self.array_len_of(obj) {
                            return len;
                        }
                    }
                }

                // 泛型调用
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.generic_fns.contains_key(fname) {
                        return self.monomorphize_call(fname, args);
                    }
                }

                // 方法调用
                if let Expr::Field(obj, method_name) = callee.as_ref() {
                    if let Expr::Ident(obj_name) = obj.as_ref() {
                        if let Some(struct_name) = self.var_types.get(obj_name).cloned() {
                            let key = (struct_name.clone(), method_name.clone());
                            if self.methods.contains_key(&key) {
                                let arr_flags = self
                                    .method_array_params
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or_default();
                                let mut all_args = vec![format!("&{}", obj_name)];
                                for (i, a) in args.iter().enumerate() {
                                    all_args.push(self.gen_expr_with_self(a, fields));
                                    if arr_flags.get(i).copied().unwrap_or(false) {
                                        match self.array_len_of(a) {
                                            Some(len) => all_args.push(len),
                                            None => all_args.push(format!(
                                                "/* 无法确定第 {} 个实参的数组长度 */0LL",
                                                i + 1
                                            )),
                                        }
                                    }
                                }
                                return format!(
                                    "{}_{}({})",
                                    struct_name, method_name, all_args.join(", ")
                                );
                            }
                        }
                    }
                }

                let callee_str = self.gen_expr_with_self(callee, fields);
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.array_ret_fns.contains(fname) {
                        return format!(
                            "/* {}() 返回数组，请写成 `let r = {}(...)` 才能拿到长度 */",
                            fname, fname
                        );
                    }
                }
                let arg_strs = self.gen_call_args(callee, args, Some(fields));
                format!("{}({})", callee_str, arg_strs.join(", "))
            }

            Expr::Binary(l, op, r) => {
                let ls = self.gen_expr_with_self(l, fields);
                let rs = self.gen_expr_with_self(r, fields);
                let op_str = match op {
                    BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
                    BinOp::Div => "/", BinOp::Mod => "%",
                    BinOp::Eq => "==", BinOp::Ne => "!=",
                    BinOp::Lt => "<", BinOp::Gt => ">",
                    BinOp::Le => "<=", BinOp::Ge => ">=",
                    BinOp::And => "&&", BinOp::Or => "||",
                    BinOp::Pipe => {
                        let mut pipe_args = vec![ls];
                        let callee = match r.as_ref() {
                            Expr::Call(callee, args) => {
                                for a in args {
                                    pipe_args.push(self.gen_expr_with_self(a, fields));
                                }
                                self.gen_expr_with_self(callee, fields)
                            }
                            other => self.gen_expr_with_self(other, fields),
                        };
                        return format!("{}({})", callee, pipe_args.join(", "));
                    }
                };
                format!("({} {} {})", ls, op_str, rs)
            }

            Expr::Unary(op, e) => {
                let es = self.gen_expr_with_self(e, fields);
                let op_str = match op {
                    UnOp::Neg => "-", UnOp::Not => "!",
                };
                format!("({}{})", op_str, es)
            }

            // self.x → _self->x（裸字段名由上面的 Ident 分支处理）
            Expr::Field(obj, name) => {
                if name == "len" {
                    if let Some(len) = self.array_len_of(obj) {
                        return len;
                    }
                }
                if let Expr::Ident(id) = obj.as_ref() {
                    if id == "self" && fields.contains(name) {
                        return format!("_self->{}", name);
                    }
                }
                let o = self.gen_expr_with_self(obj, fields);
                format!("{}.{}", o, name)
            }

            Expr::Index(obj, idx) => {
                let o = self.gen_expr_with_self(obj, fields);
                let i = self.gen_expr_with_self(idx, fields);
                format!("{}[{}]", o, i)
            }

            Expr::StructLit(name, fields_list) => {
                let field_strs: Vec<String> = fields_list.iter()
                    .map(|(fname, fval)| {
                        let v = self.gen_expr_with_self(fval, fields);
                        format!(".{} = {}", fname, v)
                    })
                    .collect();
                format!("({}){{ {} }}", name, field_strs.join(", "))
            }

            _ => self.gen_expr(expr),
        }
    }
}

fn builtin_map(name: &str) -> Option<&'static str> {
    Some(match name {
        // str
        "str_len" => "lume_str_len",
        "str_upper" => "lume_str_upper",
        "str_lower" => "lume_str_lower",
        "str_trim" => "lume_str_trim",
        "str_to_int" => "lume_str_to_int",
        "str_to_float" => "lume_str_to_float",
        // int
        "int_abs" => "lume_int_abs",
        "int_min" => "lume_int_min",
        "int_max" => "lume_int_max",
        // float
        "float_sqrt" | "sqrt" => "lume_float_sqrt",
        "float_abs" => "lume_float_abs",
        "float_pow" | "pow" => "lume_float_pow",
        "float_sin" | "sin" => "lume_float_sin",
        "float_cos" | "cos" => "lume_float_cos",
        "float_tan" | "tan" => "lume_float_tan",
        "float_floor" | "floor" => "lume_float_floor",
        "float_ceil" | "ceil" => "lume_float_ceil",
        "float_round" | "round" => "lume_float_round",
        _ => return None,
    })
}