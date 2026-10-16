use super::Codegen;
use crate::ast::*;

impl Codegen {
    pub(crate) fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, ty, init } => {
                if let Expr::List(items) = init {
                    let len = items.len();
                    let elem_ty = if let Some(first) = items.first() {
                        self.infer_type(first)
                    } else {
                        "int64_t".to_string()
                    };
                    let elem_strs: Vec<String> = items.iter()
                        .map(|e| self.gen_expr(e))
                        .collect();
                    self.push_line(&format!(
                        "{} {}[] = {{{}}};",
                        elem_ty, name, elem_strs.join(", ")
                    ));
                    self.array_lens.insert(name.clone(), len);
                    self.array_elem_types.insert(name.clone(), elem_ty.clone());
                    self.var_types.insert(name.clone(), format!("{}*", elem_ty));
                    return;
                }

                let c_ty = match ty {
                    Some(t) => self.gen_type(t),
                    None => self.infer_type(init),
                };
                let value = self.gen_expr(init);
                self.var_types.insert(name.clone(), c_ty.clone());
                self.push_line(&format!("{} {} = {};", c_ty, name, value));
            }

            Stmt::Assign { target, value } => {
                let t = self.gen_expr(target);
                let v = self.gen_expr(value);
                self.push_line(&format!("{} = {};", t, v));
            }

            Stmt::Expr(e) => {
                let s = self.gen_expr(e);
                self.push_line(&format!("{};", s));
            }

            Stmt::If { cond, then, els } => {
                let c = self.gen_expr(cond);
                self.push_line(&format!("if ({}) {{", c));
                self.indent += 1;
                for s in &then.stmts {
                    self.gen_stmt(s);
                }
                self.indent -= 1;
                if let Some(else_stmt) = els {
                    self.push_line("} else {");
                    self.indent += 1;
                    // parser 把 `else { ... }` 包成了 Expr::If(Bool(true), block)，
                    // 这里展开成块内语句，否则会退化成 /* TODO */
                    match else_stmt.as_ref() {
                        Stmt::Expr(Expr::If(cond, block, _))
                            if matches!(cond.as_ref(), Expr::Bool(true)) =>
                        {
                            for s in &block.stmts {
                                self.gen_stmt(s);
                            }
                        }
                        other => self.gen_stmt(other),
                    }
                    self.indent -= 1;
                }
                self.push_line("}");
            }

            Stmt::While { cond, body } => {
                let c = self.gen_expr(cond);
                self.push_line(&format!("while ({}) {{", c));
                self.indent += 1;
                for s in &body.stmts {
                    self.gen_stmt(s);
                }
                self.indent -= 1;
                self.push_line("}");
            }

            Stmt::For { var, iter, body } => self.gen_for(var, iter, body),
            Stmt::ParFor { var, iter, body } => self.gen_par_for(var, iter, body),

            Stmt::Return(v) => match v {
                Some(e) => {
                    let s = self.gen_expr(e);
                    self.push_line(&format!("return {};", s));
                }
                None => self.push_line("return;"),
            },

            Stmt::Break => self.push_line("break;"),
            Stmt::Continue => self.push_line("continue;"),
            Stmt::Match { value, arms } => self.gen_match(value, arms),
        }
    }

    pub(crate) fn gen_stmt_with_self(&mut self, stmt: &Stmt, fields: &[String]) {
        match stmt {
            Stmt::Let { name, ty, init } => {
                if let Expr::List(items) = init {
                    let len = items.len();
                    let elem_ty = if let Some(first) = items.first() {
                        self.infer_type(first)
                    } else {
                        "int64_t".to_string()
                    };
                    let elem_strs: Vec<String> = items.iter()
                        .map(|e| self.gen_expr_with_self(e, fields))
                        .collect();
                    self.push_line(&format!(
                        "{} {}[] = {{{}}};",
                        elem_ty, name, elem_strs.join(", ")
                    ));
                    self.array_lens.insert(name.clone(), len);
                    self.array_elem_types.insert(name.clone(), elem_ty.clone());
                    self.var_types.insert(name.clone(), format!("{}*", elem_ty));
                    return;
                }

                let c_ty = match ty {
                    Some(t) => self.gen_type(t),
                    None => self.infer_type(init),
                };
                let value = self.gen_expr_with_self(init, fields);
                self.var_types.insert(name.clone(), c_ty.clone());
                self.push_line(&format!("{} {} = {};", c_ty, name, value));
            }

            Stmt::Assign { target, value } => {
                let t = self.gen_expr_with_self(target, fields);
                let v = self.gen_expr_with_self(value, fields);
                self.push_line(&format!("{} = {};", t, v));
            }

            Stmt::Expr(e) => {
                let s = self.gen_expr_with_self(e, fields);
                self.push_line(&format!("{};", s));
            }

            Stmt::If { cond, then, els } => {
                let c = self.gen_expr_with_self(cond, fields);
                self.push_line(&format!("if ({}) {{", c));
                self.indent += 1;
                for s in &then.stmts {
                    self.gen_stmt_with_self(s, fields);
                }
                self.indent -= 1;
                if let Some(else_stmt) = els {
                    self.push_line("} else {");
                    self.indent += 1;
                    match else_stmt.as_ref() {
                        Stmt::Expr(Expr::If(cond, block, _))
                            if matches!(cond.as_ref(), Expr::Bool(true)) =>
                        {
                            for s in &block.stmts {
                                self.gen_stmt_with_self(s, fields);
                            }
                        }
                        other => self.gen_stmt_with_self(other, fields),
                    }
                    self.indent -= 1;
                }
                self.push_line("}");
            }

            Stmt::While { cond, body } => {
                let c = self.gen_expr_with_self(cond, fields);
                self.push_line(&format!("while ({}) {{", c));
                self.indent += 1;
                for s in &body.stmts {
                    self.gen_stmt_with_self(s, fields);
                }
                self.indent -= 1;
                self.push_line("}");
            }

            Stmt::For { var, iter, body } => self.gen_for(var, iter, body),
            Stmt::ParFor { var, iter, body } => self.gen_par_for(var, iter, body),

            Stmt::Return(v) => match v {
                Some(e) => {
                    let s = self.gen_expr_with_self(e, fields);
                    self.push_line(&format!("return {};", s));
                }
                None => self.push_line("return;"),
            },

            Stmt::Break => self.push_line("break;"),
            Stmt::Continue => self.push_line("continue;"),
            Stmt::Match { value, arms } => self.gen_match(value, arms),
        }
    }

    // Lume 的数组不会别名，循环一定可以向量化。
    // 加上这个提示，clang 会跳过代价模型判断直接向量化（安全依赖不变）
    fn push_vectorize_hint(&mut self) {
        self.push_line("#if defined(__clang__)");
        self.push_line("#pragma clang loop vectorize(enable) interleave(enable)");
        self.push_line("#endif");
    }

    pub(crate) fn gen_for(&mut self, var: &str, iter: &Expr, body: &Block) {
        match iter {
            Expr::Range(start, end, inclusive) => {
                let s = self.gen_expr(start);
                let e = self.gen_expr(end);
                let cmp = if *inclusive { "<=" } else { "<" };
                self.push_vectorize_hint();
                self.push_line(&format!(
                    "for (int64_t {} = {}; {} {} {}; {}++) {{",
                    var, s, var, cmp, e, var
                ));
                self.var_types.insert(var.to_string(), "int64_t".to_string());
            }
            Expr::Ident(arr_name) => {
                if let Some(&len) = self.array_lens.get(arr_name) {
                    let elem_ty = self.array_elem_types.get(arr_name)
                        .cloned().unwrap_or_else(|| "int64_t".to_string());
                    let idx = format!("_i_{}", arr_name);
                    self.push_vectorize_hint();
                    self.push_line(&format!(
                        "for (int64_t {} = 0; {} < {}; {}++) {{",
                        idx, idx, len, idx
                    ));
                    self.push_line(&format!(
                        "{} {} = {}[{}];",
                        elem_ty, var, arr_name, idx
                    ));
                    self.var_types.insert(var.to_string(), elem_ty);
                } else if self.array_params.contains(arr_name) {
                    // 数组参数：编译期不知道长度，不能拿 usize::MAX 当上界
                    self.push_line(&format!(
                        "/* for: {} 是数组参数，长度未知，请写 for i in 0..n */",
                        arr_name
                    ));
                } else {
                    self.push_line("/* for: 未知数组 */");
                }
            }
            _ => self.push_line("/* for: 暂不支持 */"),
        }
        self.indent += 1;
        for s in &body.stmts {
            self.gen_stmt(s);
        }
        self.indent -= 1;
        self.push_line("}");
    }
        pub(crate) fn gen_match(
        &mut self,
        value: &Expr,
        arms: &[MatchArm],
    ) {
        let val_str = self.gen_expr(value);

        self.push_line("{");
        self.indent += 1;
        self.push_line(&format!("int64_t _match_val = {};", val_str));

        let mut has_wildcard = false;

        for (i, arm) in arms.iter().enumerate() {
            // 检查是不是通配
            let is_wildcard = matches!(arm.pattern, Pattern::Wildcard);

            // 生成条件
            if is_wildcard {
                has_wildcard = true;
                if i == 0 {
                    // 第一臂就是 _，直接执行
                } else {
                    self.push_line("else {");
                }
            } else if i == 0 {
                let cond = self.gen_pattern_cond(&arm.pattern);
                self.push_line(&format!("if ({}) {{", cond));
            } else {
                let cond = self.gen_pattern_cond(&arm.pattern);
                self.push_line(&format!("else if ({}) {{", cond));
            }

            self.indent += 1;

            // 绑定变量
            self.gen_pattern_bind(&arm.pattern);

            // 执行 body
            let body_str = self.gen_expr(&arm.body);
            self.push_line(&format!("{};", body_str));

            self.indent -= 1;
            self.push_line("}");
        }

        // 没有 _ 的时候加个空 else（避免悬挂）
        if !has_wildcard {
            // 不生成 else，让未匹配的情况静默跳过
        }

        self.indent -= 1;
        self.push_line("}");
    }

    fn gen_pattern_cond(&mut self, pat: &Pattern) -> String {
        match pat {
            Pattern::Wildcard => "1".to_string(),
            Pattern::Literal(e) => {
                let lit = self.gen_expr(e);
                format!("_match_val == {}", lit)
            }
            Pattern::Ident(_) => "1".to_string(),  // 变量绑定总是匹配
            Pattern::Tuple(_) => "1".to_string(),  // 元组总是匹配（第一版）
            Pattern::Struct(_, _) => "1".to_string(),
        }
    }

    fn gen_pattern_bind(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Ident(name) => {
                self.push_line(&format!("int64_t {} = _match_val;", name));
                self.var_types.insert(name.clone(), "int64_t".to_string());
            }
            Pattern::Tuple(pats) => {
                // 假设 value 是数组/元组，用 _match_val[i] 访问
                for (i, p) in pats.iter().enumerate() {
                    if let Pattern::Ident(name) = p {
                        self.push_line(&format!(
                            "int64_t {} = _match_val[{}];",
                            name, i
                        ));
                        self.var_types.insert(name.clone(), "int64_t".to_string());
                    }
                }
            }
            _ => {}
        }
    }
}