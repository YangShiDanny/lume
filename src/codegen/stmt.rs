use super::Codegen;
use super::expr::bare_parens;
use crate::ast::*;

/// None 字面量：裸 `None` 或 `None()` 调用
fn is_none_literal(e: &Expr) -> bool {
    match e {
        Expr::Ident(n) => n == "None",
        Expr::Call(callee, args) => {
            matches!(callee.as_ref(), Expr::Ident(n) if n == "None") && args.is_empty()
        }
        _ => false,
    }
}

impl Codegen {
    pub(crate) fn gen_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, ty, init } => {
                // 数组返回值的调用：拆成三行，把长度落进 {name}_len
                if let Some(lines) = self.gen_array_let(name, init, None) {
                    for l in lines {
                        self.push_line(&l);
                    }
                    return;
                }

                // 管道融合：xs |> map(f) |> map(g) 编译成单个循环
                if let Some(lines) = self.gen_fused_pipe_let(name, init, None) {
                    for l in lines {
                        self.push_line(&l);
                    }
                    return;
                }

                // 数组高阶内置直调：let r = map(xs, f) / filter(xs, p)
                if let Some(lines) = self.gen_hof_let(name, init) {
                    for l in lines {
                        self.push_line(&l);
                    }
                    return;
                }

                // 切片：同样要拆行（指针 + 长度两个值）
                if let Some(lines) = self.gen_slice_let(name, init, None) {
                    for l in lines {
                        self.push_line(&l);
                    }
                    self.owned_arrays.remove(name); // 切片是借用，不能 free
                    return;
                }

                if let Some((elem_ty, len)) =
                    self.gen_repeat_array_init(name, ty.as_ref(), init, None)
                {
                    self.array_len_exprs.insert(name.clone(), format!("{}LL", len));
                    self.array_elem_types.insert(name.clone(), elem_ty.clone());
                    self.var_types.insert(name.clone(), format!("{}*", elem_ty));
                    self.owned_arrays.remove(name); // 栈数组，不能 free
                    return;
                }

                if let Some((elem_ty, len, initializer)) =
                    self.gen_local_array_init(ty.as_ref(), init, None)
                {
                    self.push_line(&format!(
                        "{} {}[{}] = {};",
                        elem_ty, name, len, initializer
                    ));
                    self.array_len_exprs.insert(name.clone(), format!("{}LL", len));
                    self.array_elem_types.insert(name.clone(), elem_ty.clone());
                    self.var_types.insert(name.clone(), format!("{}*", elem_ty));
                    self.owned_arrays.remove(name); // 栈数组，不能 free
                    return;
                }

                let c_ty = match ty {
                    Some(t) => self.gen_type(t),
                    None => self.infer_type(init),
                };
                let value = if is_none_literal(init)
                    && matches!(ty, Some(Type::Optional(_)) | Some(Type::Error(_)))
                {
                    format!("({}){{0, 0}}", c_ty)
                } else {
                    self.gen_expr(init)
                };
                self.var_types.insert(name.clone(), c_ty.clone());
                // `let b := a` 是别名：a 是 owned 的话 b 也共享同一块内存
                let aliases_owned = matches!(init, Expr::Ident(src) if self.owned_arrays.contains(src));
                if aliases_owned {
                    self.owned_arrays.insert(name.clone());
                } else {
                    self.owned_arrays.remove(name);
                }
                self.push_line(&format!("{} {} = {};", c_ty, name, value));
            }

            Stmt::Assign { target, value } => {
                let t = self.gen_expr(target);
                let v = self.gen_expr(value);
                if let Expr::Ident(name) = target {
                    self.owned_arrays.remove(name);
                }
                self.push_line(&format!("{} = {};", t, v));
            }

            Stmt::Expr(e) => {
                let s = self.gen_expr(e);
                self.push_line(&format!("{};", s));
            }

            Stmt::If { cond, then, els } => {
                let c = self.gen_expr(cond);
                self.push_line(&format!("if ({}) {{", bare_parens(&c)));
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
                // while 循环同样吃向量化提示（clang 会自动判断能不能向量化）
                self.push_vectorize_hint(body);
                self.push_line(&format!("while ({}) {{", bare_parens(&c)));
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
                    // 返回数组：栈上的字面量不能直接返回（悬空指针），必须 malloc
                    if self.cur_ret_is_array {
                        if let Some(lines) = self.gen_array_return(e, None) {
                            for l in lines {
                                self.push_line(&l);
                            }
                            return;
                        }
                    }
                    let s = if is_none_literal(e)
                        && self.is_opt_tag(&self.cur_ret_c)
                    {
                        format!("({}){{0, 0}}", self.cur_ret_c)
                    } else {
                        self.gen_expr(e)
                    };
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
                if let Some(lines) = self.gen_array_let(name, init, Some(fields)) {
                    for l in lines {
                        self.push_line(&l);
                    }
                    return;
                }
                if let Some(lines) = self.gen_fused_pipe_let(name, init, Some(fields)) {
                    for l in lines {
                        self.push_line(&l);
                    }
                    return;
                }
                // 方法体里的 map / filter（以前漏了这一步，会生成
                // `int64_t ys = map(xs, f);` 这种非法 C）
                if let Some(lines) = self.gen_hof_let(name, init) {
                    for l in lines {
                        self.push_line(&l);
                    }
                    return;
                }
                if let Some(lines) = self.gen_slice_let(name, init, Some(fields)) {
                    for l in lines {
                        self.push_line(&l);
                    }
                    self.owned_arrays.remove(name); // 切片是借用，不能 free
                    return;
                }

                if let Some((elem_ty, len)) =
                    self.gen_repeat_array_init(name, ty.as_ref(), init, Some(fields))
                {
                    self.array_len_exprs.insert(name.clone(), format!("{}LL", len));
                    self.array_elem_types.insert(name.clone(), elem_ty.clone());
                    self.var_types.insert(name.clone(), format!("{}*", elem_ty));
                    self.owned_arrays.remove(name); // 栈数组，不能 free
                    return;
                }

                if let Some((elem_ty, len, initializer)) =
                    self.gen_local_array_init(ty.as_ref(), init, Some(fields))
                {
                    self.push_line(&format!(
                        "{} {}[{}] = {};",
                        elem_ty, name, len, initializer
                    ));
                    self.array_len_exprs.insert(name.clone(), format!("{}LL", len));
                    self.array_elem_types.insert(name.clone(), elem_ty.clone());
                    self.var_types.insert(name.clone(), format!("{}*", elem_ty));
                    self.owned_arrays.remove(name); // 栈数组，不能 free
                    return;
                }

                let c_ty = match ty {
                    Some(t) => self.gen_type(t),
                    None => self.infer_type(init),
                };
                let value = if is_none_literal(init)
                    && matches!(ty, Some(Type::Optional(_)) | Some(Type::Error(_)))
                {
                    format!("({}){{0, 0}}", c_ty)
                } else {
                    self.gen_expr_with_self(init, fields)
                };
                self.var_types.insert(name.clone(), c_ty.clone());
                let aliases_owned = matches!(init, Expr::Ident(src) if self.owned_arrays.contains(src));
                if aliases_owned {
                    self.owned_arrays.insert(name.clone());
                } else {
                    self.owned_arrays.remove(name);
                }
                self.push_line(&format!("{} {} = {};", c_ty, name, value));
            }

            Stmt::Assign { target, value } => {
                let t = self.gen_expr_with_self(target, fields);
                let v = self.gen_expr_with_self(value, fields);
                if let Expr::Ident(name) = target {
                    self.owned_arrays.remove(name);
                }
                self.push_line(&format!("{} = {};", t, v));
            }

            Stmt::Expr(e) => {
                let s = self.gen_expr_with_self(e, fields);
                self.push_line(&format!("{};", s));
            }

            Stmt::If { cond, then, els } => {
                let c = self.gen_expr_with_self(cond, fields);
                self.push_line(&format!("if ({}) {{", bare_parens(&c)));
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
                self.push_vectorize_hint(body);
                self.push_line(&format!("while ({}) {{", bare_parens(&c)));
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
                    if self.cur_ret_is_array {
                        if let Some(lines) = self.gen_array_return(e, Some(fields)) {
                            for l in lines {
                                self.push_line(&l);
                            }
                            return;
                        }
                    }
                    let s = if is_none_literal(e)
                        && self.is_opt_tag(&self.cur_ret_c)
                    {
                        format!("({}){{0, 0}}", self.cur_ret_c)
                    } else {
                        self.gen_expr_with_self(e, fields)
                    };
                    self.push_line(&format!("return {};", s));
                }
                None => self.push_line("return;"),
            },

            Stmt::Break => self.push_line("break;"),
            Stmt::Continue => self.push_line("continue;"),
            Stmt::Match { value, arms } => self.gen_match(value, arms),
        }
    }

    // 提示 clang 跳过代价模型、直接尝试向量化。
    // 但循环体里有 say / 未证明纯度的函数调用时，向量化根本不可能，
    // 强制 pragma 只会让 clang 每次构建都报 -Wpass-failed。
    // 所以先扫一遍循环体：只有纯算术 + 已证明纯函数才发提示。
    fn push_vectorize_hint(&mut self, body: &Block) {
        if self.body_has_unsafe_call(body) {
            return;
        }
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

                // 循环变量非负推断：for i in 0..n 里 i 一定 >= 0
                let is_non_neg = matches!(start.as_ref(), Expr::Int(n) if *n >= 0);
                let was_non_neg = self.non_neg_vars.contains(var);
                if is_non_neg {
                    self.non_neg_vars.insert(var.to_string());
                }

                self.push_vectorize_hint(body);
                self.push_line(&format!(
                    "for (int64_t {} = {}; {} {} {}; {}++) {{",
                    var, s, var, cmp, e, var
                ));
                self.var_types.insert(var.to_string(), "int64_t".to_string());

                self.indent += 1;
                for s in &body.stmts {
                    self.gen_stmt(s);
                }
                self.indent -= 1;
                self.push_line("}");

                // 退出循环，恢复原来的非负状态
                if is_non_neg && !was_non_neg {
                    self.non_neg_vars.remove(var);
                }
                return;
            }
            // for v in a[i..j]：切片有显式的指针 + 长度表达式，
            // 不查 array_len_exprs（没有绑定名）
            Expr::Slice(obj, start, end, inclusive) => {
                let base = self.gen_expr(obj);
                let (s, e, base_len) =
                    self.slice_bounds(obj, start, end, *inclusive);
                let base_ty = self.infer_type(obj);
                let elem_ty = base_ty
                    .strip_suffix('*')
                    .unwrap_or(&base_ty)
                    .to_string();
                let ptr = format!(
                    "({} + lume_slice_off({}, {}, {}, {}))",
                    base, base, s, e, base_len
                );
                let len = format!(
                    "lume_slice_len({}, {}, {}, {})",
                    base, s, e, base_len
                );
                self.push_vectorize_hint(body);
                self.push_line(&format!(
                    "for (int64_t _i_s = 0; _i_s < {}; _i_s++) {{",
                    len
                ));
                self.push_line(&format!(
                    "{} {} = {}[_i_s];",
                    elem_ty, var, ptr
                ));
                self.var_types.insert(var.to_string(), elem_ty);
            }
            Expr::Ident(arr_name) => {
                // 局部数组字面量和数组参数都在 array_len_exprs 里：
                // 前者长度是常量，后者是伴随参数 {name}_len。
                // 有了隐式长度参数，for v in a 对两者都能用。
                if let Some(len_expr) = self.array_len_exprs.get(arr_name).cloned() {
                    let elem_ty = self.array_elem_types.get(arr_name)
                        .cloned().unwrap_or_else(|| "int64_t".to_string());
                    let idx = format!("_i_{}", arr_name);
                    self.push_vectorize_hint(body);
                    self.push_line(&format!(
                        "for (int64_t {} = 0; {} < {}; {}++) {{",
                        idx, idx, len_expr, idx
                    ));
                    self.push_line(&format!(
                        "{} {} = {}[{}];",
                        elem_ty, var, arr_name, idx
                    ));
                    self.var_types.insert(var.to_string(), elem_ty);
                } else {
                    self.push_line(&format!(
                        "/* for: 无法确定 {} 的长度（只有数组字面量和数组参数可以）*/",
                        arr_name
                    ));
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
        // _match_val 的类型按被 match 的值推断。
        // 以前一律写成 int64_t，导致只能 match 整数/布尔，
        // 结构体和浮点全被截断。
        let val_ty = self.infer_type(value);
        let val_ty = if val_ty == "void*" { "int64_t".to_string() } else { val_ty };

        // 收集结构体模式涉及的类型。只要有一个结构体臂，
        // 就把这些 struct 装进一个 union：
        //   - 按值的真实类型写进去
        //   - 各臂再按自己的类型读
        // C 标准明确允许读 union 的任意成员，所以这是合法的类型双关；
        // 直接 ((Rect*)&circle)->w 才是 UB，-O3 下可能被优化错。
        // 同时 tag 在运行期兜底，类型不对的臂不会真的执行到读字段那步。
        let mut pattern_structs: Vec<String> = Vec::new();
        for arm in arms {
            if let Pattern::Struct(name, _) = &arm.pattern {
                if !pattern_structs.contains(name) {
                    pattern_structs.push(name.clone());
                }
            }
        }
        let val_is_struct = self.structs.contains_key(&val_ty);
        let use_union = !pattern_structs.is_empty() && val_is_struct;

        self.push_line("{");
        self.indent += 1;

        if use_union {
            let mut members: Vec<String> = vec![val_ty.clone()];
            for n in &pattern_structs {
                if !members.contains(n) {
                    members.push(n.clone());
                }
            }
            self.push_line("union {");
            self.indent += 1;
            for m in &members {
                self.push_line(&format!("{} _m_{};", m, m));
            }
            self.indent -= 1;
            self.push_line("} _match_val;");
            self.push_line(&format!(
                "_match_val._m_{} = {};",
                val_ty, val_str
            ));
            // 记住值真实类型对应的 union 成员名，
            // 字面量/变量绑定臂也要通过它访问 _match_val
            self.match_val_member = Some(val_ty.clone());
        } else {
            self.match_val_member = None;
            self.push_line(&format!("{} _match_val = {};", val_ty, val_str));
        }

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
        // union 模式下，字面量/变量绑定臂要通过值的真实类型成员访问 _match_val
        let mv = match &self.match_val_member {
            Some(m) => format!("_match_val._m_{}", m),
            None => "_match_val".to_string(),
        };
        match pat {
            Pattern::Wildcard => "1".to_string(),
            Pattern::Literal(e) => {
                let lit = self.gen_expr(e);
                format!("{} == {}", mv, lit)
            }
            Pattern::Ident(_) => "1".to_string(),  // 变量绑定总是匹配
            Pattern::Tuple(_) => "1".to_string(),  // 元组总是匹配（第一版）
            // 结构体模式：比较类型标签。
            // tag 在 gen_struct_decl 里加字段、StructLit 里填值，
            // 三处必须同步，否则解构会静默绑错数据。
            // 字段访问走 union 成员 _m_<Struct>（见 gen_match）。
            Pattern::Struct(name, _) => {
                let tag = self.struct_tags.get(name).copied().unwrap_or(-1);
                format!("_match_val._m_{}._tag == {}LL", name, tag)
            }
        }
    }

    fn gen_pattern_bind(&mut self, pat: &Pattern) {
        // union 模式下字面量/变量绑定要通过值的真实类型成员访问
        let mv = |cg: &Codegen| -> String {
            match &cg.match_val_member {
                Some(m) => format!("_match_val._m_{}", m),
                None => "_match_val".to_string(),
            }
        };
        match pat {
            Pattern::Ident(name) => {
                self.push_line(&format!("int64_t {} = {};", name, mv(self)));
                self.var_types.insert(name.clone(), "int64_t".to_string());
            }
            Pattern::Tuple(pats) => {
                // 假设 value 是数组/元组，用 _match_val[i] 访问
                for (i, p) in pats.iter().enumerate() {
                    if let Pattern::Ident(name) = p {
                        self.push_line(&format!(
                            "int64_t {} = {}[{}];",
                            name, mv(self), i
                        ));
                        self.var_types.insert(name.clone(), "int64_t".to_string());
                    }
                }
            }
            // 结构体解构：第 i 个子模式绑定到第 i 个字段。
            // 所以 `Circle(r)` 会把 .r 绑到 r，`Rect(w, h)` 绑 .w / .h。
            Pattern::Struct(sname, subs) => {
                let fields = match self.structs.get(sname).cloned() {
                    Some(f) => f,
                    None => return,
                };
                for (i, (_, p)) in subs.iter().enumerate() {
                    if let Pattern::Ident(vname) = p {
                        if let Some((fname, fty)) = fields.get(i) {
                            self.push_line(&format!(
                                "{} {} = _match_val._m_{}.{};",
                                fty, vname, sname, fname
                            ));
                            self.var_types.insert(vname.clone(), fty.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
// ── 向量化可行性扫描 ──
//
// 只用来决定「要不要发 #pragma clang loop vectorize」。
// 保守：遇到任何未被 proves 纯的函数调用就放弃提示（clang 自己仍会尝试，
// 只是不强制跳过代价模型）。say/write/flush 一定不是纯函数，所以含日志的
// 循环自动不再收到 pragma —— 构建时的 -Wpass-failed 噪音就没了。

impl Codegen {
    pub(crate) fn is_vectorizable_fn(&self, name: &str) -> bool {
        self.const_fns.contains(name)
            || matches!(
                name,
                "sqrt" | "pow" | "sin" | "cos" | "tan" | "floor" | "ceil" | "round"
                    | "abs" | "fabs" | "int_abs" | "float_abs" | "int_min" | "int_max"
            )
    }

    pub(crate) fn body_has_unsafe_call(&self, body: &Block) -> bool {
        body.stmts.iter().any(|s| self.stmt_has_unsafe_call(s))
    }

    fn stmt_has_unsafe_call(&self, s: &Stmt) -> bool {
        match s {
            Stmt::Let { init, .. } => self.expr_has_unsafe_call(init),
            Stmt::Assign { target, value } => {
                self.expr_has_unsafe_call(target) || self.expr_has_unsafe_call(value)
            }
            Stmt::Expr(e) => self.expr_has_unsafe_call(e),
            Stmt::If { cond, then, els } => {
                self.expr_has_unsafe_call(cond)
                    || self.body_has_unsafe_call(then)
                    || els.as_ref().map(|s| self.stmt_has_unsafe_call(s)).unwrap_or(false)
            }
            Stmt::While { cond, body } => {
                self.expr_has_unsafe_call(cond) || self.body_has_unsafe_call(body)
            }
            Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
                self.expr_has_unsafe_call(iter) || self.body_has_unsafe_call(body)
            }
            Stmt::Return(Some(e)) => self.expr_has_unsafe_call(e),
            Stmt::Match { value, arms } => {
                self.expr_has_unsafe_call(value)
                    || arms.iter().any(|a| self.expr_has_unsafe_call(&a.body))
            }
            _ => false,
        }
    }

    fn expr_has_unsafe_call(&self, e: &Expr) -> bool {
        match e {
            Expr::FmtStr(parts) => parts.iter().any(|p| match p {
                StrPart::Expr(x) => self.expr_has_unsafe_call(x),
                StrPart::Text(_) => false,
            }),
            Expr::Binary(l, _, r) => {
                self.expr_has_unsafe_call(l) || self.expr_has_unsafe_call(r)
            }
            Expr::Unary(_, x) | Expr::Await(x) | Expr::Try(x) | Expr::Force(x) => {
                self.expr_has_unsafe_call(x)
            }
            Expr::Call(callee, args) => {
                // 方法调用（callee 是 Field）一律当作不可向量化
                let ok = match callee.as_ref() {
                    Expr::Ident(name) => self.is_vectorizable_fn(name),
                    _ => false,
                };
                if !ok {
                    return true;
                }
                args.iter().any(|a| self.expr_has_unsafe_call(a))
            }
            Expr::Field(o, _) => self.expr_has_unsafe_call(o),
            Expr::Index(o, i) => {
                self.expr_has_unsafe_call(o) || self.expr_has_unsafe_call(i)
            }
            Expr::Slice(o, a, b, _) => {
                self.expr_has_unsafe_call(o)
                    || a.as_ref().map(|x| self.expr_has_unsafe_call(x)).unwrap_or(false)
                    || b.as_ref().map(|x| self.expr_has_unsafe_call(x)).unwrap_or(false)
            }
            Expr::Lambda { body, .. } | Expr::Spawn(body) => self.body_has_unsafe_call(body),
            Expr::Tuple(v) | Expr::List(v) => v.iter().any(|x| self.expr_has_unsafe_call(x)),
            Expr::Map(pairs) => pairs.iter().any(|(k, v)| {
                self.expr_has_unsafe_call(k) || self.expr_has_unsafe_call(v)
            }),
            Expr::StructLit(_, fields) => {
                fields.iter().any(|(_, v)| self.expr_has_unsafe_call(v))
            }
            Expr::Repeat(x, _) => self.expr_has_unsafe_call(x),
            Expr::Range(a, b, _) => {
                self.expr_has_unsafe_call(a) || self.expr_has_unsafe_call(b)
            }
            Expr::If(c, t, e) => {
                self.expr_has_unsafe_call(c)
                    || self.body_has_unsafe_call(t)
                    || e.as_ref().map(|b| self.body_has_unsafe_call(b)).unwrap_or(false)
            }
            Expr::Match(v, arms) => {
                self.expr_has_unsafe_call(v)
                    || arms.iter().any(|a| self.expr_has_unsafe_call(&a.body))
            }
            _ => false,
        }
    }
}
