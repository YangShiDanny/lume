use super::{Checker, FuncSig, Ty, is_builtin};
use crate::ast::*;

impl Checker {
    pub(crate) fn check_expr(&mut self, expr: &Expr) -> Ty {
        match expr {
            Expr::Int(_) => Ty::Int,
            Expr::Float(_) => Ty::Float,
            Expr::Str(_) => Ty::Str,
            Expr::FmtStr(parts) => {
                for p in parts {
                    if let StrPart::Expr(e) = p {
                        let _ = self.check_expr(e);
                    }
                }
                Ty::Str
            }
            Expr::Rune(_) => Ty::Rune,
            Expr::Color(_) => Ty::Int,
            Expr::Bool(_) => Ty::Bool,
            Expr::Nothing => Ty::Unknown,
            Expr::It => Ty::Unknown,

            Expr::Ident(name) => {
                if let Some(ty) = self.lookup(name) {
                    ty
                } else if self.functions.contains_key(name) {
                    Ty::Unknown
                } else {
                    self.error_hint(
                        format!("未定义的变量 '{}'", name),
                        "检查拼写，或先声明这个变量",
                    );
                    Ty::Unknown
                }
            }

            // 管道：a |> f(b) 等价于 f(a, b)。
            // 必须按脱糖后的形式检查 —— 否则右半边的 f(b) 会被当成
            // 一个独立调用，凭空多报一个"参数个数不对"。
            Expr::Binary(l, BinOp::Pipe, r) => {
                if let Expr::Call(callee, extra) = r.as_ref() {
                    let mut args = vec![l.as_ref().clone()];
                    args.extend(extra.iter().cloned());
                    let call = Expr::Call(callee.clone(), args);
                    self.check_expr(&call)
                } else {
                    let _ = self.check_expr(l);
                    self.check_expr(r)
                }
            }

            Expr::Binary(l, op, r) => {
                let lt = self.check_expr(l);
                let rt = self.check_expr(r);
                self.check_binop(op, &lt, &rt)
            }

            Expr::Unary(op, e) => {
                let et = self.check_expr(e);
                match op {
                    UnOp::Neg => {
                        if !et.is_unknown() && et != Ty::Int && et != Ty::Float {
                            self.error(format!(
                                "负号只能用于数字，但得到 {}",
                                et.name()
                            ));
                        }
                        et
                    }
                    UnOp::Not => {
                        if !et.is_unknown() && et != Ty::Bool {
                            self.error(format!(
                                "not 只能用于 bool，但得到 {}",
                                et.name()
                            ));
                        }
                        Ty::Bool
                    }
                }
            }

            Expr::Call(callee, args) => {
                // 内置
                if let Expr::Ident(name) = callee.as_ref() {
                    if is_builtin(name) {
                        for a in args {
                            let _ = self.check_expr(a);
                        }
                        return Ty::Unknown;
                    }
                }
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "say" {
                        for a in args {
                            let _ = self.check_expr(a);
                        }
                        return Ty::Void;
                    }
                    if name == "now" {
                        return Ty::Int;
                    }
                }

                // 用户函数
                if let Expr::Ident(name) = callee.as_ref() {
                    if let Some(sig) = self.functions.get(name).cloned() {
                        if self.generic_fns.contains(name) {
                            if args.len() != sig.params.len() {
                                self.error_hint(
                                    format!(
                                        "函数 '{}' 需要 {} 个参数，但传了 {} 个",
                                        name, sig.params.len(), args.len()
                                    ),
                                    format!("改成传 {} 个参数", sig.params.len()),
                                );
                            }
                            for a in args {
                                let _ = self.check_expr(a);
                            }
                            return Ty::Unknown;
                        }

                        self.check_call_args(name, &sig, args);
                        return sig.ret;
                    } else {
                        self.error_hint(
                            format!("未定义的函数 '{}'", name),
                            "检查拼写，或先定义这个函数",
                        );
                        return Ty::Unknown;
                    }
                }

                // 方法调用
                if let Expr::Field(obj, method) = callee.as_ref() {
                    let obj_ty = self.check_expr(obj);
                    for a in args {
                        let _ = self.check_expr(a);
                    }
                    if let Ty::Struct(sname) = &obj_ty {
                        if let Some(methods) = self.methods.get(sname).cloned() {
                            if let Some(sig) = methods.get(method).cloned() {
                                return sig.ret;
                            }
                        }
                    }
                    return Ty::Unknown;
                }

                let _ = self.check_expr(callee);
                for a in args {
                    let _ = self.check_expr(a);
                }
                Ty::Unknown
            }

            Expr::Field(obj, name) => {
                let obj_ty = self.check_expr(obj);
                if let Ty::Struct(sname) = &obj_ty {
                    if let Some(fields) = self.structs.get(sname) {
                        if let Some(fty) = fields.get(name) {
                            return fty.clone();
                        }
                    }
                }
                Ty::Unknown
            }

            Expr::Index(obj, idx) => {
                let _ = self.check_expr(obj);
                let idx_ty = self.check_expr(idx);
                if !idx_ty.is_unknown() && idx_ty != Ty::Int {
                    self.error(format!(
                        "数组索引必须是 int，但得到 {}",
                        idx_ty.name()
                    ));
                }
                Ty::Int
            }

            Expr::Slice(_, _, _, _) => Ty::Unknown,

            Expr::Lambda { params, body } => {
                self.push_scope();
                for p in params {
                    let ty = self.ty_from_ast(p.ty.as_ref());
                    self.define(p.name.clone(), ty);
                }
                self.check_block(body);
                self.pop_scope();
                Ty::Unknown
            }

            Expr::Tuple(items) => {
                for e in items {
                    let _ = self.check_expr(e);
                }
                Ty::Unknown
            }

            Expr::List(items) => {
                let mut elem_ty = Ty::Unknown;
                for e in items {
                    let t = self.check_expr(e);
                    if elem_ty.is_unknown() {
                        elem_ty = t;
                    } else if !t.is_unknown() && t != elem_ty {
                        self.error(format!(
                            "数组元素类型不一致：{} 和 {}",
                            elem_ty.name(), t.name()
                        ));
                        return Ty::Unknown;
                    }
                }
                Ty::Unknown
            }

            Expr::Map(_) => Ty::Unknown,
            Expr::StructLit(name, fields) => {
                for (_, v) in fields {
                    let _ = self.check_expr(v);
                }
                Ty::Struct(name.clone())
            }

            Expr::Range(a, b, _) => {
                let _ = self.check_expr(a);
                let _ = self.check_expr(b);
                Ty::Unknown
            }

            Expr::If(cond, then, els) => {
                let cond_ty = self.check_expr(cond);
                if !cond_ty.is_unknown() && cond_ty != Ty::Bool {
                    self.error(format!(
                        "if 条件必须是 bool，但得到 {}",
                        cond_ty.name()
                    ));
                }
                self.check_block(then);
                if let Some(e) = els {
                    self.check_block(e);
                }
                Ty::Unknown
            }

            Expr::Match(_, _) => Ty::Unknown,
            Expr::Await(e) => self.check_expr(e),
            Expr::Spawn(_) => Ty::Void,
            Expr::Try(e) => self.check_expr(e),
            Expr::Force(e) => self.check_expr(e),
        }
    }

    pub(crate) fn check_binop(&mut self, op: &BinOp, lt: &Ty, rt: &Ty) -> Ty {
        use BinOp::*;
        match op {
            Add | Sub | Mul | Div | Mod => {
                if lt.is_unknown() || rt.is_unknown() {
                    return Ty::Unknown;
                }
                if lt == &Ty::Int && rt == &Ty::Int {
                    return Ty::Int;
                }
                if (lt == &Ty::Float || lt == &Ty::Int)
                    && (rt == &Ty::Float || rt == &Ty::Int)
                {
                    return Ty::Float;
                }
                self.error(format!(
                    "算术运算类型不匹配：{} 和 {}",
                    lt.name(), rt.name()
                ));
                Ty::Unknown
            }
            Eq | Ne => {
                if lt.is_unknown() || rt.is_unknown() {
                    return Ty::Bool;
                }
                if lt != rt {
                    self.error(format!(
                        "== 两边类型不同：{} 和 {}",
                        lt.name(), rt.name()
                    ));
                }
                Ty::Bool
            }
            Lt | Gt | Le | Ge => {
                if lt.is_unknown() || rt.is_unknown() {
                    return Ty::Bool;
                }
                if lt != rt {
                    self.error(format!(
                        "比较运算符两边类型不同：{} 和 {}",
                        lt.name(), rt.name()
                    ));
                }
                Ty::Bool
            }
            And | Or => {
                if !lt.is_unknown() && lt != &Ty::Bool {
                    self.error(format!(
                        "and/or 左边必须是 bool，但得到 {}",
                        lt.name()
                    ));
                }
                if !rt.is_unknown() && rt != &Ty::Bool {
                    self.error(format!(
                        "and/or 右边必须是 bool，但得到 {}",
                        rt.name()
                    ));
                }
                Ty::Bool
            }
            Pipe => Ty::Unknown,
        }
    }

    pub(crate) fn check_call_args(&mut self, fname: &str, sig: &FuncSig, args: &[Expr]) {
        if args.len() != sig.params.len() {
            self.error_hint(
                format!(
                    "函数 '{}' 需要 {} 个参数，但传了 {} 个",
                    fname, sig.params.len(), args.len()
                ),
                format!("改成传 {} 个参数", sig.params.len()),
            );
        }

        for (i, arg) in args.iter().enumerate() {
            let arg_ty = self.check_expr(arg);
            if let Some(expected) = sig.params.get(i) {
                if !arg_ty.is_unknown()
                    && !expected.is_unknown()
                    && &arg_ty != expected
                {
                    self.error(format!(
                        "函数 '{}' 第 {} 个参数类型不匹配：期望 {}，得到 {}",
                        fname, i + 1, expected.name(), arg_ty.name()
                    ));
                }
            }
        }
    }
}