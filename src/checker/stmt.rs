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
                let declared_ty = match ty {
                    Some(t) => self.ty_from_ast(Some(t)),
                    None => init_ty.clone(),
                };

                if !declared_ty.is_unknown()
                    && !init_ty.is_unknown()
                    && declared_ty != init_ty
                {
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
                for s in &body.stmts {
                    self.check_stmt(s);
                }
                self.pop_scope();
            }

            Stmt::Return(v) => {
                let actual_ty = match v {
                    Some(e) => self.check_expr(e),
                    None => Ty::Void,
                };
                let expected = self.current_ret.clone();
                if !expected.is_unknown()
                    && !actual_ty.is_unknown()
                    && expected != actual_ty
                {
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