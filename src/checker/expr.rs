use super::{Checker, FuncSig, Ty, builtin_return_type, is_builtin};
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
                if name == "None" {
                    return Ty::Optional(Box::new(Ty::Unknown));
                }
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
                // 编译期除零：整数字面量 0 作除数/模数直接拒编译。
                // 浮点除零是 IEEE 语义（inf），合法；变量的情况运行时
                // 兜底（lume_checked_div_i64 / lume_checked_mod_i64）。
                if matches!(op, BinOp::Div | BinOp::Mod) {
                    if let Expr::Int(0) = r.as_ref() {
                        if lt == Ty::Int || lt.is_unknown() {
                            let what = match op {
                                BinOp::Div => "division",
                                _ => "modulo",
                            };
                            self.error_hint(
                                format!("{} by zero", what),
                                "整数除零/模零在 C 里是未定义行为，Lume 编译期直接拒绝",
                            );
                        }
                    }
                }
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
                // 内置构造：Some(x) / None
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "Some" {
                        if let Some(x) = args.first() {
                            let inner = self.check_expr(x);
                            return Ty::Optional(Box::new(inner));
                        }
                        return Ty::Optional(Box::new(Ty::Unknown));
                    }
                    if name == "None" {
                        return Ty::Optional(Box::new(Ty::Unknown));
                    }
                }

                // 内置
                if let Expr::Ident(name) = callee.as_ref() {
                    if is_builtin(name) {
                        for a in args {
                            let _ = self.check_expr(a);
                        }
                        return builtin_return_type(name);
                    }
                }
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "say" || name == "write" {
                        for a in args {
                            let _ = self.check_expr(a);
                        }
                        return Ty::Void;
                    }
                    if name == "flush" {
                        if !args.is_empty() {
                            self.error("flush 不接受参数");
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
                    // a.len() → int
                    if method == "len" && args.is_empty() {
                        if let Ty::Array(_) = &obj_ty {
                            return Ty::Int;
                        }
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
                // 可空值的 .has / .val
                if name == "has" || name == "val" {
                    if let Ty::Optional(inner) = &obj_ty {
                        return if name == "has" { Ty::Bool } else { (**inner).clone() };
                    }
                }
                // a.len → int
                if name == "len" {
                    if let Ty::Array(_) = &obj_ty {
                        return Ty::Int;
                    }
                }
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
                let obj_ty = self.check_expr(obj);
                let idx_ty = self.check_expr(idx);
                if !idx_ty.is_unknown() && idx_ty != Ty::Int {
                    self.error(format!(
                        "数组索引必须是 int，但得到 {}",
                        idx_ty.name()
                    ));
                }
                // a[i] 的类型就是数组元素类型；不是数组就还按 int 算
                match &obj_ty {
                    Ty::Array(inner) => inner.as_ref().clone(),
                    _ => Ty::Int,
                }
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
                Ty::Array(Box::new(elem_ty))
            }

            Expr::Repeat(value, _) => Ty::Array(Box::new(self.check_expr(value))),

            Expr::Map(_) => Ty::Unknown,
            Expr::StructLit(name, fields) => {
                // f(a: 1) 与 Point(x: 1) 语法同形，parser 一律解析成 StructLit。
                // 名字不是结构体 → 命名参数函数调用，按签名重排后检查。
                if !self.structs.contains_key(name) {
                    if let Some(sig) = self.functions.get(name).cloned() {
                        return self.check_named_call(name, &sig, fields);
                    }
                    // 既不是结构体也不是函数：只能是用错了。以前这里静默
                    // 放行、codegen 生成一个必然编译不过的结构体字面量；
                    // 现在命名参数进来了，把两种正确写法都在提示里说清楚。
                    self.error_hint(
                        format!("'{}' 不是结构体，也不能这样调用", name),
                        "结构体构造写 Point(x: 1)；函数命名参数写 f(a: 1)（内置函数和 say/write/now 只支持位置参数）",
                    );
                }
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
                // 作为表达式时类型取 then 分支最后一条
                match then.stmts.last() {
                    Some(Stmt::Expr(e)) => {
                        let t = self.check_expr(e);
                        // 有 else 就要求两边同类型
                        if let Some(eb) = els {
                            if let Some(Stmt::Expr(ee)) = eb.stmts.last() {
                                let et = self.check_expr(ee);
                                if !t.is_unknown() && !et.is_unknown() && t != et {
                                    self.error(format!(
                                        "if 表达式两个分支类型不同：{} 和 {}",
                                        t.name(),
                                        et.name()
                                    ));
                                }
                            }
                        }
                        t
                    }
                    _ => Ty::Unknown,
                }
            }

            Expr::Match(_, _) => Ty::Unknown,
            Expr::Await(e) => self.check_expr(e),
            Expr::Spawn(_) => Ty::Void,
            Expr::Try(e) => {
                let et = self.check_expr(e);
                match &et {
                    Ty::Optional(inner) => {
                        let inner = (**inner).clone();
                        // 传播：当前函数必须返回 T?（同 elem），否则提前返回的
                        // None 没法构造，也没法接住。
                        if !self.current_ret.is_unknown() {
                            match &self.current_ret {
                                Ty::Optional(ret_inner) => {
                                    if !ret_inner.is_unknown()
                                        && !inner.is_unknown()
                                        && **ret_inner != inner
                                    {
                                        self.error_hint(
                                            "? 传播要求函数返回 T?，且 T 与解包出的类型一致"
                                                .to_string(),
                                            format!("把函数返回类型改成 {}", inner.name()),
                                        );
                                    }
                                }
                                _ => {
                                    self.error_hint(
                                        "? 只能用在返回 T? 的函数里".to_string(),
                                        format!("把函数返回类型改成 {}", inner.name()),
                                    );
                                }
                            }
                        }
                        inner
                    }
                    Ty::Unknown => Ty::Unknown,
                    _ => {
                        self.error(format!("? 只能用于可选值，但得到 {}", et.name()));
                        Ty::Unknown
                    }
                }
            }
            Expr::Force(e) => {
                let et = self.check_expr(e);
                match &et {
                    Ty::Optional(inner) => (**inner).clone(),
                    Ty::Unknown => Ty::Unknown,
                    _ => {
                        self.error(format!("! 只能用于可选值，但得到 {}", et.name()));
                        Ty::Unknown
                    }
                }
            }
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
        if sig.required == sig.params.len() {
            // 无默认参数：个数必须精确
            if args.len() != sig.params.len() {
                self.error_hint(
                    format!(
                        "函数 '{}' 需要 {} 个参数，但传了 {} 个",
                        fname, sig.params.len(), args.len()
                    ),
                    format!("改成传 {} 个参数", sig.params.len()),
                );
            }
        } else if args.len() < sig.required || args.len() > sig.params.len() {
            // 有默认参数：尾部参数可省略，个数落在 [required, total] 即可
            self.error_hint(
                format!(
                    "函数 '{}' 需要 {} ~ {} 个参数，但传了 {} 个",
                    fname, sig.required, sig.params.len(), args.len()
                ),
                format!(
                    "尾部 {} 个参数有默认值，可以省略",
                    sig.params.len() - sig.required
                ),
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

    /// 命名参数调用 f(b: 2, a: 1)。parser 把这种语法解析成 StructLit，
    /// 由 check_expr 的 StructLit 分支按语义分流过来（名字不是结构体）。
    /// 重排成位置参数后，类型检查与位置调用完全同一套。
    fn check_named_call(&mut self, fname: &str, sig: &FuncSig, fields: &[(String, Expr)]) -> Ty {
        if self.generic_fns.contains(fname) {
            self.error_hint(
                format!("泛型函数 '{}' 暂不支持命名参数", fname),
                "改成位置参数调用",
            );
            for (_, v) in fields {
                let _ = self.check_expr(v);
            }
            return sig.ret.clone();
        }
        match reorder_named_args(fields, &sig.param_names) {
            Ok(slots) => {
                // None 槽位 = 调用处没传，必须由默认值补，否则是缺参
                for (i, slot) in slots.iter().enumerate() {
                    if slot.is_some() {
                        continue;
                    }
                    let has_default = sig
                        .defaults
                        .get(i)
                        .map(|d| d.is_some())
                        .unwrap_or(false);
                    if !has_default {
                        self.error_hint(
                            format!("函数 '{}' 缺少参数 '{}'", fname, sig.param_names[i]),
                            format!("补上 '{}: ...'，或改用位置参数", sig.param_names[i]),
                        );
                    }
                }
                for (i, slot) in slots.iter().enumerate() {
                    if let Some(arg) = slot {
                        let arg_ty = self.check_expr(arg);
                        if let Some(expected) = sig.params.get(i) {
                            if !arg_ty.is_unknown()
                                && !expected.is_unknown()
                                && &arg_ty != expected
                            {
                                self.error(format!(
                                    "函数 '{}' 参数 '{}' 类型不匹配：期望 {}，得到 {}",
                                    fname, sig.param_names[i], expected.name(), arg_ty.name()
                                ));
                            }
                        }
                    }
                }
                sig.ret.clone()
            }
            Err(msg) => {
                self.error_hint(msg, "检查参数名，或改用位置参数调用");
                for (_, v) in fields {
                    let _ = self.check_expr(v);
                }
                sig.ret.clone()
            }
        }
    }
}