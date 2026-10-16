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

                // 数组 .len()
                if let Expr::Field(obj, method) = callee.as_ref() {
                    if method == "len" && args.is_empty() {
                        if let Expr::Ident(arr_name) = obj.as_ref() {
                            if let Some(&len) = self.array_lens.get(arr_name) {
                                if len != usize::MAX {
                                    return format!("{}LL", len);
                                }
                            }
                            // 数组参数：用 {name}_len
                            if self.var_types.get(arr_name)
                                .map(|t| t.ends_with('*')).unwrap_or(false)
                            {
                                return format!("{}_len", arr_name);
                            }
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
                            if self.methods.contains_key(
                                &(struct_name.clone(), method_name.clone())
                            ) {
                                let args_str: Vec<String> = args.iter()
                                    .map(|a| self.gen_expr(a))
                                    .collect();
                                let mut all_args = vec![format!("&{}", obj_name)];
                                all_args.extend(args_str);
                                return format!(
                                    "{}_{}({})",
                                    struct_name, method_name, all_args.join(", ")
                                );
                            }
                        }
                    }
                }

                // 普通调用
                let callee_str = self.gen_expr(callee);
                let args_str: Vec<String> = args.iter()
                    .map(|a| self.gen_expr(a))
                    .collect();
                format!("{}({})", callee_str, args_str.join(", "))
            }

            Expr::Field(obj, name) => {
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

    pub(crate) fn gen_expr_with_self(&mut self, expr: &Expr, fields: &[String]) -> String {
        match expr {
            Expr::Ident(name) if name == "self" => "_self".to_string(),
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

                // 数组 .len()
                if let Expr::Field(obj, method) = callee.as_ref() {
                    if method == "len" && args.is_empty() {
                        if let Expr::Ident(arr_name) = obj.as_ref() {
                            if let Some(&len) = self.array_lens.get(arr_name) {
                                if len != usize::MAX {
                                    return format!("{}LL", len);
                                }
                            }
                            if self.var_types.get(arr_name)
                                .map(|t| t.ends_with('*')).unwrap_or(false)
                            {
                                return format!("{}_len", arr_name);
                            }
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
                            if self.methods.contains_key(
                                &(struct_name.clone(), method_name.clone())
                            ) {
                                let args_str: Vec<String> = args.iter()
                                    .map(|a| self.gen_expr_with_self(a, fields))
                                    .collect();
                                let mut all_args = vec![format!("&{}", obj_name)];
                                all_args.extend(args_str);
                                return format!(
                                    "{}_{}({})",
                                    struct_name, method_name, all_args.join(", ")
                                );
                            }
                        }
                    }
                }

                let callee_str = self.gen_expr_with_self(callee, fields);
                let args_str: Vec<String> = args.iter()
                    .map(|a| self.gen_expr_with_self(a, fields))
                    .collect();
                format!("{}({})", callee_str, args_str.join(", "))
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