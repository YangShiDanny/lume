// src/codegen/types.rs
use super::Codegen;
use crate::ast::*;
use std::collections::HashMap;

use super::expr::single_expr;

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
            Type::F32 => "float".to_string(),
            Type::F64 => "double".to_string(),
            Type::Bool => "bool".to_string(),
            Type::Str => "const char*".to_string(),
            Type::Rune => "uint32_t".to_string(),
            Type::Named(s) => s.clone(),
            Type::List(inner) => format!("{}*", self.gen_type(inner)),
            Type::Array(inner, _) => format!("{}*", self.gen_type(inner)),
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
                BinOp::Pipe => self.infer_type(r),
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
            _ => "int64_t".to_string(),
        }
    }
}