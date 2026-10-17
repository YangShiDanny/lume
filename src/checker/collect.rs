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

                    let sig = FuncSig {
                        params: f.params.iter()
                            .map(|p| self.ty_from_ast(p.ty.as_ref()))
                            .collect(),
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
                        let sig = FuncSig {
                            params: m.params.iter()
                                .map(|p| self.ty_from_ast(p.ty.as_ref()))
                                .collect(),
                            ret: self.ty_from_ast(m.ret.as_ref()),
                        };
                        methods.insert(m.name.clone(), sig);
                    }
                    self.methods.insert(s.name.clone(), methods);
                }
                _ => {}
            }
        }
    }

    pub(crate) fn ty_from_ast(&self, t: Option<&Type>) -> Ty {
        match t {
            None => Ty::Void,
            Some(t) => match t {
                Type::Int | Type::I8 | Type::I16 | Type::I32 | Type::I64
                | Type::U1 | Type::U2 | Type::U4 | Type::U8
                | Type::U16 | Type::U32 | Type::U64 => Ty::Int,
                Type::F32 | Type::F64 => Ty::Float,
                Type::Bool => Ty::Bool,
                Type::Str => Ty::Str,
                Type::Rune => Ty::Rune,
                Type::Named(s) => Ty::Struct(s.clone()),
                Type::List(inner) | Type::Array(inner, _) => {
                    Ty::Array(Box::new(self.ty_from_ast(Some(inner))))
                }
                _ => Ty::Unknown,
            },
        }
    }
}