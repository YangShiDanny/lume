use super::{Checker, FuncSig, Ty};
use crate::ast::*;
use std::collections::HashMap;

impl Checker {
    pub(crate) fn collect_items(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Fn(f) => {
                    // @precompute 参数检查
                    if let Some(d) = f.decorators.iter().find(|d| d.name == "precompute") {
                        if f.params.len() != 1 {
                            self.error_hint(
                                format!(
                                    "@precompute 函数 '{}' 必须恰好有 1 个参数",
                                    f.name
                                ),
                                "改成单参数形式",
                            );
                        }
                        if d.args.len() != 2 {
                            self.error_hint(
                                format!(
                                    "@precompute 需要 2 个参数 (start, end): {}",
                                    f.name
                                ),
                                "例如 @precompute(0, 100)",
                            );
                        } else {
                            for arg in &d.args {
                                if !matches!(arg, Expr::Int(_)) {
                                    self.error(format!(
                                        "@precompute 参数必须是整数常量: {}",
                                        f.name
                                    ));
                                }
                            }
                        }
                    }

                    // 默认参数必须连续排在尾部，见 check_defaults_trailing
                    self.check_defaults_trailing(f, &f.name);

                    let sig = FuncSig {
                        params: f.params.iter()
                            .map(|p| self.ty_from_ast(p.ty.as_ref()))
                            .collect(),
                        param_names: f.params.iter().map(|p| p.name.clone()).collect(),
                        required: f.params.iter().take_while(|p| p.default.is_none()).count(),
                        defaults: f.params.iter().map(|p| p.default.clone()).collect(),
                        ret: self.ty_from_ast(f.ret.as_ref()),
                    };

                    if !f.generics.is_empty() {
                        self.generic_fns.insert(f.name.clone());
                    }

                    self.functions.insert(f.name.clone(), sig);
                }
                Item::Struct(s) => {
                    let mut fields = HashMap::new();
                    for f in &s.fields {
                        fields.insert(f.name.clone(), self.ty_from_ast(Some(&f.ty)));
                    }
                    self.structs.insert(s.name.clone(), fields);

                    let mut methods = HashMap::new();
                    for m in &s.methods {
                        let label = format!("{}.{}", s.name, m.name);
                        self.check_defaults_trailing(m, &label);
                        // 运算符方法的签名在**定义处**校验一次。留到使用处
                        // 再报的话，错误指向 `a + b`，而不是写错的那个 fn。
                        if crate::ops::is_op_method(&m.name) {
                            self.check_op_method_signature(s, m);
                        }
                        let sig = FuncSig {
                            params: m.params.iter()
                                .map(|p| self.ty_from_ast(p.ty.as_ref()))
                                .collect(),
                            param_names: m.params.iter().map(|p| p.name.clone()).collect(),
                            required: m.params.iter().take_while(|p| p.default.is_none()).count(),
                            defaults: m.params.iter().map(|p| p.default.clone()).collect(),
                            ret: self.ty_from_ast(m.ret.as_ref()),
                        };
                        methods.insert(m.name.clone(), sig);
                    }
                    self.methods.insert(s.name.clone(), methods);
                }
                Item::Enum(e) => {
                    if self.enums.contains_key(&e.name) {
                        self.error(format!("枚举 '{}' 重复定义", e.name));
                        continue;
                    }
                    let names: Vec<String> =
                        e.variants.iter().map(|v| v.name.clone()).collect();
                    self.enums.insert(e.name.clone(), names);

                    // 值重复会让 C 里的 switch 出现 duplicate case，
                    // 而且 match 上根本分不开这两个变体，直接在编译期拒掉。
                    let mut seen_vals: Vec<i64> = Vec::new();
                    for (vname, val) in e.resolved() {
                        if seen_vals.contains(&val) {
                            self.error_hint(
                                format!(
                                    "枚举 '{}' 的变体 '{}' 的值 {} 和前面的变体重复",
                                    e.name, vname, val
                                ),
                                "每个变体的值必须唯一，显式值之后没写的会依次 +1",
                            );
                        } else {
                            seen_vals.push(val);
                        }
                    }
                }
                // 顶层常量：先登记名字 → 类型，函数体里的引用才不会报
                // 「未定义的变量」。类型声明优先，没写就按字面量推断。
                Item::Const(c) => {
                    let ty = match &c.ty {
                        Some(t) => self.ty_from_ast(Some(t)),
                        None => match &c.value {
                            Expr::Int(_) | Expr::Color(_) => Ty::Int,
                            Expr::Float(_) => Ty::Float,
                            Expr::Bool(_) => Ty::Bool,
                            Expr::Str(_) => Ty::Str,
                            Expr::Rune(_) => Ty::Rune,
                            // 表达式形式的常量值：类型推不出来就交给
                            // codegen 的 C 常量传播，checker 不猜。
                            _ => Ty::Unknown,
                        },
                    };
                    self.consts.insert(c.name.clone(), ty);
                    if let Some(v) = literal_int(&c.value) {
                        self.const_ints.insert(c.name.clone(), v);
                    }
                }
                _ => {}
            }
        }
    }

    /// 运算符方法的签名约束。名字表在 `crate::ops`，规则很窄：
    ///
    /// - `op_str` / `op_neg` 是零元（self 隐式，不算参数）；
    /// - 其余都是二元（右操作数一个参数）；
    /// - `op_str` 必须返回 str —— 它要直接进 printf 的 `%s`。
    fn check_op_method_signature(&mut self, s: &StructDecl, m: &FnDecl) {
        let unary = m.name == crate::ops::STR_METHOD || m.name == "op_neg";
        let want = if unary { 0 } else { 1 };
        if m.params.len() != want {
            self.error_hint(
                format!(
                    "运算符方法 {}.{} 必须有 {} 个参数（self 是隐式的，不算参数）",
                    s.name, m.name, want
                ),
                format!(
                    "运算符方法一共这些：{}",
                    crate::ops::method_names().join(" / ")
                ),
            );
        }
        if m.name == crate::ops::STR_METHOD {
            let ret = self.ty_from_ast(m.ret.as_ref());
            if !ret.is_unknown() && ret != Ty::Str {
                self.error_hint(
                    format!("{}.{} 必须返回 str", s.name, m.name),
                    "插值 / say 会把它当 %s 直接打印",
                );
            }
        }
    }

    /// 默认参数必须连续排在参数列表尾部。
    /// C 的默认参数是调用处展开：`f(a, b = 1)` 调 `f(x)` 时补的是
    /// 最后一个参数。要是中间夹一个没默认值的（`f(a = 1, b)`），
    /// 调用处根本不知道该把实参往哪个槽位填，这个洞补不上。
    fn check_defaults_trailing(&mut self, f: &FnDecl, label: &str) {
        let mut seen_default = false;
        for p in &f.params {
            if p.default.is_some() {
                seen_default = true;
            } else if seen_default {
                self.error_hint(
                    format!(
                        "'{}' 的参数 '{}' 没有默认值，却排在带默认值的参数后面",
                        label, p.name
                    ),
                    "把有默认值的参数统一挪到参数列表尾部",
                );
                return;
            }
        }
    }

    pub(crate) fn ty_from_ast(&self, t: Option<&Type>) -> Ty {
        match t {
            None => Ty::Void,
            Some(t) => match t {
                Type::Int | Type::I8 | Type::I16 | Type::I32 | Type::I64
                | Type::U1 | Type::U2 | Type::U4 | Type::U8
                | Type::U16 | Type::U32 | Type::U64
                | Type::I128 | Type::U128 => Ty::Int,
                Type::F32 | Type::F64 => Ty::Float,
                Type::Bool => Ty::Bool,
                Type::Str => Ty::Str,
                Type::Rune => Ty::Rune,
                Type::Named(s) => {
                    // 枚举和结构体在语法上都是「一个名字」，靠登记表区分。
                    if self.enums.contains_key(s) {
                        Ty::Enum(s.clone())
                    } else {
                        Ty::Struct(s.clone())
                    }
                }
                Type::List(inner) | Type::Array(inner, _) => {
                    Ty::Array(Box::new(self.ty_from_ast(Some(inner))))
                }
                // Vec[T] 走 Generic 形状，见 ast::vec_elem_type
                Type::Generic(..) => match vec_elem_type(t) {
                    Some(inner) => Ty::Vec(Box::new(self.ty_from_ast(Some(inner)))),
                    None => Ty::Unknown,
                },
                // T? / T! 合并成同一种可空类型（见 Ty::Optional 注释）
                Type::Optional(inner) | Type::Error(inner) => {
                    Ty::Optional(Box::new(self.ty_from_ast(Some(inner))))
                }
                _ => Ty::Unknown,
            },
        }
    }
}