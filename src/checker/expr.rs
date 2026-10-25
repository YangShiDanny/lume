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
                        let t = self.check_expr(e);
                        if matches!(t, Ty::Void) {
                            self.error("void 表达式不能用于字符串插值（free 只能作语句）");
                        } else if matches!(t, Ty::Optional(_)) {
                            // T? 在 C 里是 { has, val } 结构体，插值会把它当
                            // 指针塞进 varargs —— 输出随机内容且不报错。
                            self.error_hint(
                                format!("可空值（{}）不能直接插值", t.name()),
                                "先判 .has 再用 .val，或用 ! 强制解包",
                            );
                        } else if let Ty::Struct(sname) = &t {
                            // 结构体进 printf 的 varargs 同样是 UB（读到的
                            // 是第一个字段）。有 op_str 就自动调它，没有就
                            // 明确拒绝，而不是让它静默打个 0。
                            self.require_op_str(sname);
                        }
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
                } else if let Some(ty) = self.consts.get(name) {
                    // 顶层 const：作用域里没有就是它（局部同名会先命中 lookup）
                    ty.clone()
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
                // 编译期除零：已知整数左操作数配**零常量**时拒编译。
                // 零既可以是裸字面量 `x / 0`，也可以是 `const Z = 0` 再 `x / Z`
                // —— 后者以前漏过去了，要到运行期才 abort。
                // 类型未知时不猜测整数，避免误拒泛型推导出的浮点表达式。
                // 浮点除零是 IEEE 语义（inf），合法；变量的情况运行时
                // 兜底（lume_checked_div_i64 / lume_checked_mod_i64）。
                if matches!(op, BinOp::Div | BinOp::Mod) {
                    if self.is_const_zero(r) {
                        if lt == Ty::Int {
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
                        // 结构体的取负走 op_neg（`-a` → `Big_op_neg(&a)`）
                        if let Ty::Struct(sname) = &et {
                            if self.structs.contains_key(sname) {
                                let sname = sname.clone();
                                if let Some(ret) = self.check_unop_overload(*op, &sname) {
                                    return ret;
                                }
                                self.error_hint(
                                    format!("结构体 '{}' 不支持取负（没有定义 op_neg）", sname),
                                    format!("在 struct {} 里加 `fn op_neg() -> {}`", sname, sname),
                                );
                                return Ty::Unknown;
                            }
                        }
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

// 数组高阶内置：map / filter / reduce。
            // 管道融合（`xs |> map(f) |> map(g)` 编译成单个 for 循环）
            // 的类型基础，见 codegen/pipe.rs。
            if let Expr::Ident(name) = callee.as_ref() {
                if let Some(ty) = self.check_array_hof(name, args) {
                    return ty;
                }
            }

            // free(x)：显式释放 owned 数组。类型层面只校验「一个数组参数」，
            // 真正的所有权检查在 codegen（它知道这个变量是不是函数/map/filter
            // 新建出来的堆数组）。
            if let Expr::Ident(name) = callee.as_ref() {
                if name == "free" {
                    if args.len() != 1 {
                        self.error("free 需要 1 个参数");
                    } else {
                        let ty = self.check_expr(&args[0]);
                        if !matches!(ty, Ty::Array(_) | Ty::Unknown) {
                            self.error("free 只接受数组");
                        }
                    }
                    return Ty::Void;
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
                    if name == "say" || name == "write"
                        || name == "eprint" || name == "ewrite"
                    {
                        for a in args {
                            let t = self.check_expr(a);
                            if matches!(t, Ty::Optional(_)) {
                                // 同插值：{ has, val } 结构体进 varargs 是 UB
                                self.error_hint(
                                    format!("{}() 不能直接打印可空值（{}）", name, t.name()),
                                    "先判 .has 再用 .val，或用 ! 强制解包",
                                );
                            } else if let Ty::Struct(sname) = &t {
                                // 有 op_str 就按字符串打印，没有就拒绝 ——
                                // 否则 printf 读到的是结构体第一个字段。
                                self.require_op_str(sname);
                            }
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
                    // 枚举静态方法 `Tag.name(v)`。必须在 check_expr(obj) 之前拦，
                    // `Tag` 是类型名不是变量，先查变量表只会得到「未定义的变量」。
                    if let Expr::Ident(tname) = obj.as_ref() {
                        if self.enums.contains_key(tname) {
                            if method != "name" {
                                self.error_hint(
                                    format!("枚举 '{}' 没有方法 '{}'", tname, method),
                                    format!("枚举只支持 {}.name(值)", tname),
                                );
                                return Ty::Unknown;
                            }
                            if args.len() != 1 {
                                self.error_hint(
                                    format!("{}.name() 需要 1 个参数，但传了 {} 个", tname, args.len()),
                                    format!("写成 {}.name(某个 {} 值)", tname, tname),
                                );
                                for a in args {
                                    let _ = self.check_expr(a);
                                }
                                return Ty::Str;
                            }
                            let at = self.check_expr(&args[0]);
                            let ok = match &at {
                                Ty::Unknown | Ty::Int => true,
                                Ty::Enum(n) => n == tname,
                                _ => false,
                            };
                            if !ok {
                                self.error_hint(
                                    format!(
                                        "{}.name() 的参数必须是 {}，但这里是 {}",
                                        tname, tname, at.name()
                                    ),
                                    "传一个该枚举的值（或整数）",
                                );
                            }
                            return Ty::Str;
                        }
                    }
                    let obj_ty = self.check_expr(obj);
                    // a.len() / v.len() → int
                    if method == "len" && args.is_empty() {
                        if let Ty::Array(_) | Ty::Vec(_) = &obj_ty {
                            return Ty::Int;
                        }
                    }
                    // Vec 的内置方法（不进 builtins 表：它们是接收者的类型
                    // 决定的方法，不是自由函数）。放在实参检查之前，
                    // 由它自己按参数位置检查，避免重复报告同一个错误。
                    if let Ty::Vec(elem) = &obj_ty {
                        let elem = elem.as_ref().clone();
                        return self.check_vec_method(&elem, method, args);
                    }
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
                // 枚举变体 `Tag.Number`。同样要在 check_expr(obj) 之前拦：
                // 左边是**类型名**，不是变量。
                if let Expr::Ident(tname) = obj.as_ref() {
                    if let Some(variants) = self.enums.get(tname).cloned() {
                        if !variants.iter().any(|v| v == name) {
                            let hint = if variants.len() <= 8 {
                                format!("可用变体：{}", variants.join(" / "))
                            } else {
                                "检查变体名拼写（见 enum 定义）".to_string()
                            };
                            self.error_hint(
                                format!("枚举 '{}' 没有变体 '{}'", tname, name),
                                hint,
                            );
                        }
                        return Ty::Enum(tname.clone());
                    }
                }
                let obj_ty = self.check_expr(obj);
                // 可空值的 .has / .val
                if name == "has" || name == "val" {
                    if let Ty::Optional(inner) = &obj_ty {
                        return if name == "has" { Ty::Bool } else { (**inner).clone() };
                    }
                }
                // a.len → int
                if name == "len" {
                    if let Ty::Array(_) | Ty::Vec(_) = &obj_ty {
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
                    Ty::Array(inner) | Ty::Vec(inner) => inner.as_ref().clone(),
                    _ => Ty::Int,
                }
            }

            // 切片 a[i..j] / a[i..=j] / a[..j] / a[..]：
            // 类型和被切的数组同型（元素类型不变），只是长度可变。
            // 长度是运行期值，所以这里只保证「切的是数组、边界是 int」。
            Expr::Slice(obj, start, end, _) => {
                let obj_ty = self.check_expr(obj);
                for bound in [start, end].into_iter().flatten() {
                    let bt = self.check_expr(bound);
                    if !bt.is_unknown() && bt != Ty::Int {
                        self.error(format!(
                            "切片边界必须是 int，但得到 {}",
                            bt.name()
                        ));
                    }
                }
                match &obj_ty {
                    Ty::Array(inner) => {
                        Ty::Array(Box::new(inner.as_ref().clone()))
                    },
                    _ => {
                        self.error(format!(
                            "只能切片数组，但被切对象的类型是 {}",
                            obj_ty.name()
                        ));
                        Ty::Array(Box::new(Ty::Int))
                    }
                }
            }

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

    // ── 运算符重载 ──
    //
    // 语义全部来自 `crate::ops`（和 codegen 共用一份解析）。这里只做
    // 类型检查：右侧类型对不对、返回类型对不对，以及「没定义」时的提示。

    /// 左操作数是结构体时，按 `op_*` 方法解析二元运算符。
    ///
    /// - `None` —— 左操作数不是已知结构体 / 运算符不可重载 → 交回内建规则；
    /// - `Some(Ty::Unknown)` —— 已经报过错，别再叠一条「算术运算类型不匹配」。
    fn check_struct_overload(&mut self, op: BinOp, lt: &Ty, rt: &Ty) -> Option<Ty> {
        let sname = match lt {
            Ty::Struct(n) if self.structs.contains_key(n) => n.clone(),
            _ => return None,
        };
        if !crate::ops::is_overloadable(op) {
            return None;
        }
        let methods = self.methods.get(&sname).cloned().unwrap_or_default();
        let call = match crate::ops::resolve_binop(op, |m| methods.contains_key(m)) {
            Some(c) => c,
            None => {
                let sym = crate::ops::symbol(op);
                let want = crate::ops::primary_method(op).unwrap_or("op_xxx");
                let ret = if crate::ops::is_comparison(op) {
                    "bool".to_string()
                } else {
                    sname.clone()
                };
                self.error_hint(
                    format!("结构体 '{}' 不支持 {} 运算（没有定义 {}）", sname, sym, want),
                    format!(
                        "在 struct {} 里加 `fn {}(o: {}) -> {}`",
                        sname, want, sname, ret
                    ),
                );
                return Some(Ty::Unknown);
            }
        };
        let sig = match methods.get(call.method) {
            Some(s) => s.clone(),
            None => return Some(Ty::Unknown),
        };
        if sig.params.len() != 1 {
            self.error_hint(
                format!(
                    "运算符方法 {}.{} 必须恰好有 1 个参数（self 是隐式的）",
                    sname, call.method
                ),
                format!("写成 `fn {}(o: {})`，右操作数由调用点传入", call.method, sname),
            );
            return Some(Ty::Unknown);
        }
        let want = &sig.params[0];
        if !rt.is_unknown() && !want.is_unknown() && rt != want {
            self.error_hint(
                format!(
                    "{} 的右操作数类型不匹配：{} 期望 {}，得到 {}",
                    crate::ops::symbol(op),
                    call.method,
                    want.name(),
                    rt.name()
                ),
                "运算符两边必须同类型，不做隐式转换",
            );
        }
        if crate::ops::is_comparison(op) {
            if !sig.ret.is_unknown() && sig.ret != Ty::Bool {
                self.error_hint(
                    format!(
                        "运算符方法 {}.{} 必须返回 bool（比较的结果是 bool）",
                        sname, call.method
                    ),
                    "把返回类型改成 bool",
                );
            }
            return Some(Ty::Bool);
        }
        Some(sig.ret)
    }

    /// 一元取负的重载。返回 None 表示这个结构体没有 op_neg（已报错）。
    fn check_unop_overload(&mut self, op: UnOp, sname: &str) -> Option<Ty> {
        let methods = self.methods.get(sname).cloned().unwrap_or_default();
        let call = crate::ops::resolve_unop(op, |m| methods.contains_key(m))?;
        let sig = methods.get(call.method)?.clone();
        if !sig.params.is_empty() {
            self.error_hint(
                format!("运算符方法 {}.{} 不能有参数", sname, call.method),
                "取负是一元的，写成 `fn op_neg() -> 同类型`",
            );
        }
        Some(sig.ret)
    }

    /// 这个结构体能不能在插值 / say 里自动转字符串。
    pub(crate) fn has_op_str(&self, sname: &str) -> bool {
        self.methods
            .get(sname)
            .and_then(|ms| ms.get(crate::ops::STR_METHOD))
            .map(|s| s.params.is_empty())
            .unwrap_or(false)
    }

    /// 插值 / say 里出现结构体时的统一检查。
    fn require_op_str(&mut self, sname: &str) {
        if self.has_op_str(sname) {
            return;
        }
        self.error_hint(
            format!("结构体 '{}' 不能直接打印或插值（没有定义 op_str）", sname),
            format!(
                "在 struct {} 里加 `fn op_str() -> str`，之后 \"{{x}}\" 和 say(x) 都会自动调它",
                sname
            ),
        );
    }

    pub(crate) fn check_binop(&mut self, op: &BinOp, lt: &Ty, rt: &Ty) -> Ty {
        // 结构体运算符重载优先于内建数值规则。
        if let Some(t) = self.check_struct_overload(*op, lt, rt) {
            return t;
        }
        use BinOp::*;
        match op {
            Add | Sub | Mul | Div | Mod => {
                if lt.is_unknown() || rt.is_unknown() {
                    return Ty::Unknown;
                }
                // 字符串拼接：只允许 `str + str`。
                // 不做 `int + str` 之类的隐式转换 —— 拼没拼上必须一眼看得出，
                // 想混类型就写插值 "{n}{s}"。
                if *op == Add && lt == &Ty::Str && rt == &Ty::Str {
                    return Ty::Str;
                }
                if lt == &Ty::Str || rt == &Ty::Str {
                    let hint = if *op == Add {
                        Some("字符串拼接要求两边都是 str；要和数字混拼请用插值 \"{x}{s}\"")
                    } else {
                        Some("str 只支持 + 运算（拼接）")
                    };
                    let msg = format!(
                        "算术运算类型不匹配：{} 和 {}",
                        lt.name(), rt.name()
                    );
                    match hint {
                        Some(h) => self.error_hint(msg, h.to_string()),
                        None => self.error(msg),
                    }
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
        if sig.variadic {
            // 变参：多传的部分是 varargs，个数下界是声明的参数个数
            if args.len() < sig.params.len() {
                self.error_hint(
                    format!(
                        "变参函数 '{}' 至少要 {} 个参数，但传了 {} 个",
                        fname, sig.params.len(), args.len()
                    ),
                    "`...` 之前声明的参数必须逐个传",
                );
            }
        } else if sig.required == sig.params.len() {
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
                    // 数组字面量塞给 Vec 参数是最常见的误用：两者在 C 里
                    // 是不同的东西（栈数组 vs 带 len 的句柄），不能隐式转。
                    let hint = match (expected, &arg_ty) {
                        (Ty::Vec(_), Ty::Array(_)) => Some(format!(
                            "固定数组和 Vec 是两种类型；先写 `tmp: {} = [...]` 建一个再传，\
                             或者把参数类型改成固定数组",
                            expected.name()
                        )),
                        (Ty::Array(_), Ty::Vec(_)) => {
                            Some("Vec 是运行期长度的句柄，不能当固定数组传".to_string())
                        }
                        _ => None,
                    };
                    let msg = format!(
                        "函数 '{}' 第 {} 个参数类型不匹配：期望 {}，得到 {}",
                        fname, i + 1, expected.name(), arg_ty.name()
                    );
                    match hint {
                        Some(h) => self.error_hint(msg, h),
                        None => self.error(msg),
                    }
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

    /// map / filter / reduce 的类型检查。不是这三个函数就返回 None。
    ///
    /// 语义：
    ///   map(xs, f)     → [T]，长度同 xs
    ///   filter(xs, f)  → [T]，长度 ≤ xs（运行期才知道）
    ///   reduce(xs, f, init) → f 的返回类型
    fn check_array_hof(&mut self, name: &str, args: &[Expr]) -> Option<Ty> {
        let want = match name {
            "map" => 2,
            "filter" => 2,
            "reduce" => 3,
            _ => return None,
        };
        if args.len() != want {
            self.error_hint(
                format!("{} 需要 {} 个参数，但传了 {} 个", name, want, args.len()),
                match name {
                    "reduce" => "reduce(数组, 函数, 初始值)".to_string(),
                    "filter" => "filter(数组, 谓词)".to_string(),
                    _ => "map(数组, 函数)".to_string(),
                },
            );
            for a in args {
                let _ = self.check_expr(a);
            }
            return Some(Ty::Unknown);
        }

        // 第 1 个参数必须是数组
        let arr_ty = self.check_expr(&args[0]);
        // 空数组字面量 `[]` 推不出元素类型，别让它走到 codegen
        // 生成 `map({}, f)` 这种非法 C
        if let Expr::List(items) = &args[0] {
            if items.is_empty() {
                self.error_hint(
                    format!("{} 的第 1 个参数是空数组，元素类型未知", name),
                    "空数组请先给元素类型，比如 `es: [int] = []`".to_string(),
                );
                return Some(Ty::Unknown);
            }
        }
        let elem = match &arr_ty {
            Ty::Array(inner) => (**inner).clone(),
            _ => {
                if !arr_ty.is_unknown() {
                    self.error_hint(
                        format!(
                            "{} 的第 1 个参数必须是数组，但得到 {}",
                            name,
                            arr_ty.name()
                        ),
                        "改成对数组调用".to_string(),
                    );
                }
                Ty::Int
            }
        };

        // 第 2 个参数必须是用户函数（map/reduce 取其返回类型，filter 取 bool）
        let fname = match &args[1] {
            Expr::Ident(f) => f.clone(),
            other => {
                self.error_hint(
                    format!(
                        "{} 的第 2 个参数必须是函数名，但得到 {}",
                        name,
                        describe_expr(other)
                    ),
                    "先定义这个函数，再传函数名（不带括号）".to_string(),
                );
                let _ = self.check_expr(other);
                return Some(match name {
                    "reduce" => Ty::Int,
                    _ => Ty::Array(Box::new(elem)),
                });
            }
        };
        let sig = match self.functions.get(&fname).cloned() {
            Some(s) => s,
            None => {
                self.error_hint(
                    format!("未定义的函数 '{}'", fname),
                    "先定义这个函数，或检查拼写".to_string(),
                );
                return Some(match name {
                    "reduce" => Ty::Int,
                    _ => Ty::Array(Box::new(elem)),
                });
            }
        };
        // map 的函数至少要 1 个参数（元素），reduce 要 2 个（累加器 + 元素）
        let want_params = if name == "reduce" { 2 } else { 1 };
        if sig.params.len() < want_params {
            self.error_hint(
                format!(
                    "{} 的函数 '{}' 至少需要 {} 个参数，但只有 {} 个",
                    name,
                    fname,
                    want_params,
                    sig.params.len()
                ),
                format!(
                    "{} 的第一个参数是{}",
                    fname,
                    if name == "reduce" { "累加器" } else { "元素" }
                ),
            );
        }
        if name == "filter" && sig.ret != Ty::Bool && sig.ret != Ty::Unknown {
            self.error_hint(
                format!(
                    "filter 的谓词 '{}' 必须返回 bool，但返回 {}",
                    fname,
                    sig.ret.name()
                ),
                "让谓词返回 true / false".to_string(),
            );
        }

        if name == "reduce" {
            // 初始值类型要对得上累加器参数
            let init_ty = self.check_expr(&args[2]);
            if let Some(first) = sig.params.first() {
                if !init_ty.is_unknown()
                    && !first.is_unknown()
                    && &init_ty != first
                {
                    self.error_hint(
                        format!(
                            "reduce 的初始值类型不匹配：累加器是 {}，初始值是 {}",
                            first.name(),
                            init_ty.name()
                        ),
                        format!("把初始值改成 {}", first.name()),
                    );
                }
            }
            return Some(sig.ret.clone());
        }

        // map 的返回类型 = 元素类型或函数的返回类型
        // （只做管道融合的场景下 f(T)->U；codegen 会检查得更细）
        Some(Ty::Array(Box::new(elem)))
    }
}

// ── Vec 的内置方法 ──
//
// 它们不进取 `builtins::TABLE`：那是「自由函数名 → C 函数」的映射，
// 而 Vec 的方法由接收者的**元素类型**决定（push 要校验实参类型，
// pop 要返回元素类型），只能按类型分派。
impl Checker {
    fn check_vec_method(&mut self, elem: &Ty, method: &str, args: &[Expr]) -> Ty {
        // 会改 buffer 的方法在 par for 里是数据竞争：多个 worker 同时
        // realloc 同一个 Vec，轻则丢元素重则踩坏堆。
        let mutating = matches!(method, "push" | "pop" | "clear" | "free" | "reserve");
        if mutating && self.par_depth > 0 {
            self.error_hint(
                format!("par for 里不能调用 Vec.{}()", method),
                "worker 之间共享同一个 Vec 会并发 realloc；把 push 挪到 par for 外面",
            );
            return Ty::Unknown;
        }

        match method {
            "len" | "length" if args.is_empty() => Ty::Int,
            "is_empty" if args.is_empty() => Ty::Bool,
            "clear" | "free" if args.is_empty() => Ty::Void,
            "pop" | "last" if args.is_empty() => elem.clone(),
            "reserve" if args.len() == 1 => {
                let t = self.check_expr(&args[0]);
                if !t.is_unknown() && t != Ty::Int {
                    self.error(format!("Vec.reserve 的参数必须是 int，但得到 {}", t.name()));
                }
                Ty::Void
            }
            "push" if args.len() == 1 => {
                let t = self.check_expr(&args[0]);
                if !t.is_unknown() && !elem.is_unknown() && &t != elem {
                    self.error_hint(
                        format!(
                            "Vec[{}].push 的实参类型不匹配：得到 {}",
                            elem.name(),
                            t.name()
                        ),
                        format!("往这个 Vec 里只能 push {}", elem.name()),
                    );
                }
                Ty::Void
            }
            _ => {
                let known = "len / is_empty / push / pop / last / clear / reserve / free";
                self.error_hint(
                    format!("Vec 没有方法 '{}'", method),
                    format!("可用的方法：{}", known),
                );
                Ty::Unknown
            }
        }
    }
}

fn describe_expr(e: &Expr) -> String {
    match e {
        Expr::Ident(n) => n.clone(),
        Expr::Call(c, _) => format!("函数调用 {}", describe_expr(c)),
        Expr::Int(n) => format!("整数 {}", n),
        Expr::Float(_) => "浮点数".to_string(),
        Expr::Str(_) => "字符串".to_string(),
        Expr::List(_) => "数组字面量".to_string(),
        _ => "该表达式".to_string(),
    }
}