// src/codegen/types.rs
use super::Codegen;
use crate::ast::*;
use std::collections::HashMap;

use super::expr::single_expr;

/// 可空类型的 C 表示：`typedef struct { int8_t has; <elem> val; } lume_opt_N;`
/// has=1 表示有值（Some），has=0 表示无值（None）。
///
/// 为什么不用匿名 struct：前向声明和定义各自写一遍匿名结构时，
/// tcc 判定为 "incompatible types for redefinition"（不套用 C11 的
/// 结构兼容规则）。命名 typedef 在任何编译器下都稳。
impl Codegen {
    /// 元素 C 类型 → 可空 typedef 名（惰性分配，顺序即首次出现顺序）。
    pub(crate) fn opt_tag(&self, elem_c: &str) -> String {
        if let Some(tag) = self.opt_names.borrow().get(elem_c) {
            return tag.clone();
        }
        let tag = {
            let mut names = self.opt_names.borrow_mut();
            let tag = format!("lume_opt_{}", names.len());
            names.insert(elem_c.to_string(), tag.clone());
            tag
        };
        self.opt_elems
            .borrow_mut()
            .insert(tag.clone(), elem_c.to_string());
        tag
    }

    /// 可空 typedef 名 → 元素 C 类型。非可空类型返回 int64_t（保守默认）。
    pub(crate) fn opt_elem_of(&self, tag: &str) -> String {
        self.opt_elems
            .borrow()
            .get(tag)
            .cloned()
            .unwrap_or_else(|| "int64_t".to_string())
    }

    pub(crate) fn is_opt_tag(&self, c_ty: &str) -> bool {
        self.opt_elems.borrow().contains_key(c_ty)
    }
}

impl Codegen {
    pub(crate) fn gen_type(&self, t: &Type) -> String {
        match t {
            Type::Int | Type::I64 => "int64_t".to_string(),
            Type::I8 => "int8_t".to_string(),
            Type::I16 => "int16_t".to_string(),
            Type::I32 => "int32_t".to_string(),
            Type::U1 | Type::U2 | Type::U4 | Type::U8 => "uint8_t".to_string(),
            Type::U16 => "uint16_t".to_string(),
            Type::U32 => "uint32_t".to_string(),
            Type::U64 => "uint64_t".to_string(),
            Type::I128 => "__int128".to_string(),
            Type::U128 => "unsigned __int128".to_string(),
            Type::F32 => "float".to_string(),
            Type::F64 => "double".to_string(),
            Type::Bool => "bool".to_string(),
            Type::Str => "const char*".to_string(),
            Type::Rune => "uint32_t".to_string(),
            Type::Named(s) => s.clone(),
            Type::List(inner) => format!("{}*", self.gen_type(inner)),
            Type::Array(inner, _) => format!("{}*", self.gen_type(inner)),
            // T? / T! 合并成同一种可空类型（见 checker Ty::Optional）
            Type::Optional(inner) | Type::Error(inner) => {
                let elem = self.gen_type(inner);
                self.opt_tag(&elem)
            }
            _ => "void*".to_string(),
        }
    }

    // 泛型替换：T → int64_t
    pub(crate) fn gen_type_subst(
        &self,
        t: &Type,
        type_map: &HashMap<String, String>,
    ) -> String {
        if let Type::Named(name) = t {
            if let Some(concrete) = type_map.get(name) {
                return concrete.clone();
            }
        }
        self.gen_type(t)
    }

    pub(crate) fn infer_type(&self, expr: &Expr) -> String {
        match expr {
            Expr::Int(_) => "int64_t".to_string(),
            Expr::Float(_) => "double".to_string(),
            Expr::Str(_) => "const char*".to_string(),
            Expr::Bool(_) => "bool".to_string(),
            Expr::Rune(_) => "uint32_t".to_string(),
            Expr::Color(_) => "uint32_t".to_string(),
            Expr::List(items) => {
                if let Some(first) = items.first() {
                    self.infer_type(first)
                } else {
                    "int64_t".to_string()
                }
            }
            Expr::Repeat(value, _) => self.infer_type(value),
            // 比较 / 逻辑 → bool；算术 → 看操作数，不能一律 int64_t
            Expr::Binary(l, op, r) => match op {
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt |
                BinOp::Le | BinOp::Ge | BinOp::And | BinOp::Or => "bool".to_string(),
                // 管道：数组融合管道（xs |> map(f) |> filter(p)）的元素类型
                // 由链条末端决定；普通标量管道照旧取右侧类型。
                BinOp::Pipe => match self.fused_pipe_elem_ty(expr) {
                    Some(t) => format!("{}*", t),
                    None => self.infer_type(r),
                },
                _ => {
                    let lt = self.infer_type(l);
                    let rt = self.infer_type(r);
                    if lt == "double" || rt == "double" {
                        "double".to_string()
                    } else if lt == "float" || rt == "float" {
                        "float".to_string()
                    } else {
                        lt
                    }
                }
            },
            // 结构体字段访问：按字段声明类型
            Expr::Field(obj, name) => {
                // 可空值的 .has / .val
                let oc = self.infer_type(obj);
                if self.is_opt_tag(&oc) {
                    if name == "has" {
                        return "int8_t".to_string();
                    }
                    if name == "val" {
                        return self.opt_elem_of(&oc);
                    }
                }
                if let Expr::Ident(oid) = obj.as_ref() {
                    if let Some(sname) = self.var_types.get(oid) {
                        if let Some(fields) = self.structs.get(sname) {
                            if let Some((_, fty)) =
                                fields.iter().find(|(n, _)| n == name)
                            {
                                return fty.clone();
                            }
                        }
                    }
                }
                "int64_t".to_string()
            }
            // 数组元素：按数组元素类型
            Expr::Index(obj, _) => {
                if let Expr::Ident(oid) = obj.as_ref() {
                    if let Some(t) = self.array_elem_types.get(oid) {
                        return t.clone();
                    }
                }
                "int64_t".to_string()
            }
            Expr::Ident(name) => {
                if name == "None" {
                    return self.opt_tag("int64_t");
                }
                self.var_types.get(name).cloned()
                    .unwrap_or_else(|| "int64_t".to_string())
            }
            // if 表达式的类型就是 then 分支的类型（两个分支必须一致）
            Expr::If(_, then, _) => match single_expr(then) {
                Some(e) => self.infer_type(e),
                None => "int64_t".to_string(),
            },
            // StructLit 有两种语义：结构体构造（名字是结构体）和
            // 命名参数函数调用 f(a: 1)（parser 同形，见 gen_named_call）。
            // 后者的类型 = 函数返回类型，不能把函数名当类型返回——
            // 否则 say("{add3(c: 5)}") 的插值格式选择直接抓瞎。
            Expr::StructLit(name, _) => {
                if !self.structs.contains_key(name) {
                    if let Some(ret) = self.fn_ret_types.get(name) {
                        return ret.clone();
                    }
                }
                name.clone()
            }
            Expr::Call(callee, args) => {
                // 内置构造：Some(x) / None
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "Some" {
                        if let Some(x) = args.first() {
                            let e = self.infer_type(x);
                            return self.opt_tag(&e);
                        }
                        return self.opt_tag("int64_t");
                    }
                    if name == "None" {
                        return self.opt_tag("int64_t");
                    }
                }

                // 泛型调用：按返回类型推断
                if let Expr::Ident(fname) = callee.as_ref() {
                    if let Some(gf) = self.generic_fns.get(fname) {
                        let mut type_map: HashMap<String, String> = HashMap::new();
                        for (i, p) in gf.params.iter().enumerate() {
                            if let Some(arg) = args.get(i) {
                                let at = self.infer_type(arg);
                                if let Some(Type::Named(tname)) = &p.ty {
                                    if gf.generics.contains(tname) {
                                        type_map.insert(tname.clone(), at);
                                    }
                                }
                            }
                        }
                        for g in &gf.generics {
                            type_map.entry(g.clone())
                                .or_insert_with(|| "int64_t".to_string());
                        }
                        if let Some(ret) = &gf.ret {
                            return self.gen_type_subst(ret, &type_map);
                        }
                    }
                }

                // 普通函数：查返回类型表（generate 步骤 2.4 收集的）。
                // 查不到才退回 int64_t —— 内置函数（sqrt 之类）没有签名。
                if let Expr::Ident(fname) = callee.as_ref() {
                    if let Some(ret) = self.fn_ret_types.get(fname) {
                        return ret.clone();
                    }
                }

                // 方法调用
                if let Expr::Field(obj, method_name) = callee.as_ref() {
                    if let Expr::Ident(obj_name) = obj.as_ref() {
                        if let Some(struct_name) = self.var_types.get(obj_name) {
                            if let Some(ret) = self.methods.get(
                                &(struct_name.clone(), method_name.clone())
                            ) {
                                return ret.clone();
                            }
                        }
                    }
                }

                "int64_t".to_string()
            }
            // 一元负号/取反：类型跟操作数走（否则 i128 的负值和取反会被当成 int64_t 截断）
            Expr::Unary(_, e) => self.infer_type(e),
            // 切片：元素类型不变。infer_type(数组变量) 给的是 "int64_t*"
            // （数组变量的 C 类型本身就是指针），这里要的是**元素**类型，
            // 否则 `let s = a[1..3]` 会声明成 `int64_t** s`。
            Expr::Slice(obj, _, _, _) => {
                let t = self.infer_type(obj);
                t.strip_suffix('*').unwrap_or(&t).to_string()
            }
            Expr::Force(e) => self.infer_opt_elem(e),
            Expr::Try(e) => self.infer_opt_elem(e),
            _ => "int64_t".to_string(),
        }
    }

    /// 一个求值为可空的表达式，解包后元素的 C 类型。
    /// 供 `!` / `?` 的类型推断用。
    pub(crate) fn infer_opt_elem(&self, e: &Expr) -> String {
        match e {
            Expr::Call(callee, args) => {
                if let Expr::Ident(fname) = callee.as_ref() {
                    if fname == "Some" {
                        if let Some(x) = args.first() {
                            return self.infer_type(x);
                        }
                    }
                    if let Some(ret) = self.fn_ret_types.get(fname) {
                        if self.is_opt_tag(ret) {
                            return self.opt_elem_of(ret);
                        }
                        return "int64_t".to_string();
                    }
                }
                self.opt_elem_of(&self.infer_type(e))
            }
            Expr::Ident(name) => {
                if let Some(c) = self.var_types.get(name) {
                    return self.opt_elem_of(c);
                }
                "int64_t".to_string()
            }
            _ => self.opt_elem_of(&self.infer_type(e)),
        }
    }
}