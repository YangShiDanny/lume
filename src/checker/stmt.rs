use super::{Checker, Ty};
use crate::ast::*;

impl Checker {
    pub(crate) fn check_fn(&mut self, f: &FnDecl, self_struct: Option<&str>) {
        self.push_scope();

        if let Some(sname) = self_struct {
            self.define("self".to_string(), Ty::Struct(sname.to_string()));

            // 方法体里可以直接写裸字段名（codegen 会映射成 _self->f）。
            // 检查器必须把字段也放进作用域，否则会把 x / y 误报成
            // "未定义的变量"。注意：参数在下面定义，会覆盖同名字段，
            // 和 codegen 的遮蔽顺序一致。
            if let Some(fields) = self.structs.get(sname).cloned() {
                for (fname, fty) in fields {
                    self.define(fname, fty);
                }
            }
        }

        for p in &f.params {
            let ty = self.ty_from_ast(p.ty.as_ref());
            // 默认值在调用处展开，但类型必须和参数一致
            // （引用其他参数也在这里被抓到——callee 作用域里没有它们）
            if let Some(d) = &p.default {
                let dty = self.check_expr(d);
                if !dty.is_unknown() && !ty.is_unknown() && dty != ty {
                    self.error(format!(
                        "参数 '{}' 的默认值类型不匹配：期望 {}，得到 {}",
                        p.name,
                        ty.name(),
                        dty.name()
                    ));
                }
            }
            self.define(p.name.clone(), ty);
        }

        let ret_ty = self.ty_from_ast(f.ret.as_ref());
        let prev_ret = std::mem::replace(&mut self.current_ret, ret_ty);

        self.check_block(&f.body);

        self.current_ret = prev_ret;
        self.pop_scope();
    }

    pub(crate) fn check_block(&mut self, block: &Block) {
        self.push_scope();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        self.pop_scope();
    }

    pub(crate) fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { name, ty, init } => {
                let init_ty = self.check_expr(init);
                if matches!(init_ty, Ty::Void) {
                    self.error(format!(
                        "void 表达式不能赋给变量 '{}'（free/say 只能作语句）",
                        name
                    ));
                }
                if let Some(Type::Array(_, Some(expected))) = ty {
                    let actual = match init {
                        Expr::List(items) => Some(items.len()),
                        Expr::Repeat(_, count) => Some(*count),
                        _ => None,
                    };
                    if let Some(actual) = actual {
                        if actual != *expected {
                            self.error(format!(
                                "数组 '{}' 的长度不匹配：声明为 {}，初始化为 {}",
                                name, expected, actual
                            ));
                        }
                    }
                }
                let declared_ty = match ty {
                    Some(t) => self.ty_from_ast(Some(t)),
                    None => init_ty.clone(),
                };

                if !self.ty_eq_loose(&declared_ty, &init_ty) {
                    self.error_hint(
                        format!(
                            "类型不匹配：{} 声明为 {}，但初始值是 {}",
                            name, declared_ty.name(), init_ty.name()
                        ),
                        format!("把初始值改成 {}", declared_ty.name()),
                    );
                }

                self.define(name.clone(), declared_ty);
            }

            Stmt::Assign { target, value } => {
                let _ = self.check_expr(target);
                let _ = self.check_expr(value);
            }

            Stmt::Expr(e) => {
                let _ = self.check_expr(e);
            }

            Stmt::If { cond, then, els } => {
                let cond_ty = self.check_expr(cond);
                if !cond_ty.is_unknown() && cond_ty != Ty::Bool {
                    self.error_hint(
                        format!("if 条件必须是 bool，但得到 {}", cond_ty.name()),
                        "用比较运算符（==、!=、< 等）产生 bool",
                    );
                }
                self.check_block(then);
                if let Some(e) = els {
                    self.check_stmt(e);
                }
            }

            Stmt::While { cond, body } => {
                let cond_ty = self.check_expr(cond);
                if !cond_ty.is_unknown() && cond_ty != Ty::Bool {
                    self.error_hint(
                        format!("while 条件必须是 bool，但得到 {}", cond_ty.name()),
                        "用比较运算符产生 bool",
                    );
                }
                self.check_block(body);
            }

            Stmt::For { var, iter, body } => {
                let elem_ty = self.check_iter(iter);
                self.push_scope();
                self.define(var.clone(), elem_ty);
                for s in &body.stmts {
                    self.check_stmt(s);
                }
                self.pop_scope();
            }

            Stmt::ParFor { var, iter, body } => {
                let elem_ty = self.check_iter(iter);
                self.push_scope();
                self.define(var.clone(), elem_ty);
                self.par_depth += 1;
                for s in &body.stmts {
                    self.check_stmt(s);
                }
                self.par_depth -= 1;
                self.pop_scope();
            }

            Stmt::Return(v) => {
                let actual_ty = match v {
                    Some(e) => self.check_expr(e),
                    None => Ty::Void,
                };
                let expected = self.current_ret.clone();
                if !self.ty_eq_loose(&expected, &actual_ty) {
                    self.error_hint(
                        format!(
                            "返回类型不匹配：函数声明返回 {}，但这里是 {}",
                            expected.name(), actual_ty.name()
                        ),
                        format!("把返回值改成 {}", expected.name()),
                    );
                }
            }

            Stmt::Break | Stmt::Continue => {}

            Stmt::Match { value, arms } => {
                let _ = self.check_expr(value);
                for arm in arms {
                    self.push_scope();
                    self.define_pattern(&arm.pattern);
                    let _ = self.check_expr(&arm.body);
                    self.pop_scope();
                }
            }

            // 变量作用域和普通块一致：块内声明的名字在外面看不到。
            // 字符串层面的回收由 codegen 插入 arena 水位释放，类型检查不管。
            Stmt::Scope { body } => {
                if self.par_depth > 0 {
                    self.error_hint(
                        "par for 的循环体里不能用 scope".to_string(),
                        "par for 的多个 worker 共享同一个 arena，回退水位会把别的 worker \
                         刚分配的字符串一起回收。要压内存就在 par for 外面套 scope，\
                         或者改用普通 for",
                    );
                }
                self.check_block(body);
            }
        }
    }

    /// 宽松类型相等：Optional(Unknown)（即 None 的字面量类型）与任何
    /// Optional 都兼容，这样 `x: int? = None` 不会被误报类型不匹配。
    fn ty_eq_loose(&self, a: &Ty, b: &Ty) -> bool {
        if a.is_unknown() || b.is_unknown() {
            return true;
        }
        if a == b {
            return true;
        }
        match (a, b) {
            (Ty::Optional(inner_a), Ty::Optional(_)) if inner_a.is_unknown() => true,
            (Ty::Optional(_), Ty::Optional(inner_b)) if inner_b.is_unknown() => true,
            _ => false,
        }
    }

    pub(crate) fn check_iter(&mut self, iter: &Expr) -> Ty {
        match iter {
            Expr::Range(a, b, _) => {
                let _ = self.check_expr(a);
                let _ = self.check_expr(b);
                Ty::Int
            }
            Expr::Ident(_) => {
                // for v in a：v 的类型是数组元素类型
                let t = self.check_expr(iter);
                if let Ty::Array(inner) = &t {
                    return inner.as_ref().clone();
                }
                Ty::Unknown
            }
            _ => {
                let _ = self.check_expr(iter);
                Ty::Unknown
            }
        }
    }
    
    fn define_pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Ident(name) => {
                self.define(name.clone(), Ty::Unknown);
            }
            Pattern::Tuple(pats) => {
                for p in pats {
                    self.define_pattern(p);
                }
            }
            Pattern::Struct(_, fields) => {
                for (_, p) in fields {
                    self.define_pattern(p);
                }
            }
            _ => {}
        }
    }
}