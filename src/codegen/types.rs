// src/codegen/types.rs
use super::Codegen;
use crate::ast::*;
use std::collections::HashMap;

use super::expr::single_expr;

/// 字符串可空值的 C 类型名。///
/// 其余元素走 `lume_opt_N` 的惰性编号，字符串是唯一的例外：runtime 里的
/// `lume_file_read()` 必须**直接**返回它，而 runtime header 出现在生成的
/// C 最前面，那时编译器还没分配过任何 tag。所以给它一个固定名字，写在
/// runtime 里，`opt_tag("const char*")` 永远返回这个字符串。
///
/// 布局必须和 `gen_opt_typedefs()` 生成的完全一致：
/// `{ int8_t has; const char* val; }`。
pub(crate) const OPT_STR_TAG: &str = "LumeOptStr";

/// 可空类型的 C 表示：`typedef struct { int8_t has; <elem> val; } lume_opt_N;`
/// has=1 表示有值（Some），has=0 表示无值（None）。
///
/// 为什么不用匿名 struct：前向声明和定义各自写一遍匿名结构时，
/// tcc 判定为 "incompatible types for redefinition"（不套用 C11 的
/// 结构兼容规则）。命名 typedef 在任何编译器下都稳。
impl Codegen {
    /// 元素 C 类型 → 可空 typedef 名（惰性分配，顺序即首次出现顺序）。
    pub(crate) fn opt_tag(&self, elem_c: &str) -> String {
        // 字符串有固定 tag，见 OPT_STR_TAG 的注释。
        if elem_c == "const char*" {
            self.opt_elems
                .borrow_mut()
                .insert(OPT_STR_TAG.to_string(), elem_c.to_string());
            return OPT_STR_TAG.to_string();
        }
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
            // Vec[T] → 统一是 `LumeVec*`。元素类型不进 C 类型：
            // C 没有泛型，元素大小存在句柄的 esz 里（见 runtime 的 LumeVec）。
            Type::Generic(..) if vec_elem_type(t).is_some() => "LumeVec*".to_string(),
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
            // 非 say 位置的插值会被拼成一个新字符串（见 gen_fmtstr_parts），
            // 所以类型是 const char*，不是文本。
            Expr::FmtStr(_) => "const char*".to_string(),
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
                // 枚举变体 `Tag.Number`：C 侧是 int64_t 的 #define。
                // 左边是类型名，不是变量 —— 和 gen_expr 的 Field 分支
                // 保持同一套判断顺序（枚举名优先）。
                if let Expr::Ident(tname) = obj.as_ref() {
                    if self.enums.contains_key(tname) {
                        return "int64_t".to_string();
                    }
                }
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
                // 字段访问：拿基表达式的 C 类型，是已知 struct 就按声明查。
                // 用 infer_type(obj) 而不是「只认 Ident」，字段链才走得通：
                // `w.text`（w 是方法里的裸字段）、`r.w.text`（两级）。
                // 以前只认 Ident + var_types，`w.text` 与 `r.w.text` 都退化成
                // int64_t —— 拼接变成 C 指针加法，插值打成指针值。
                if let Some(fields) = self.structs.get(&oc) {
                    if let Some((_, fty)) = fields.iter().find(|(n, _)| n == name) {
                        return fty.clone();
                    }
                }
                // v[i].field：元素是结构体时，按结构体字段声明类型
                if let Expr::Index(base, _) = obj.as_ref() {
                    if let Some(elem) = self.inferred_vec_elem(base) {
                        if let Some(fields) = self.structs.get(&elem) {
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
                if let Some(e) = self.inferred_vec_elem(obj) {
                    return e;
                }
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
                // 方法体里的 `self`：C 侧生成 `(*_self)`，类型是所属结构体。
                // 不认这一条，`self + o` 和 `-self` 的运算符重载判定就会
                // 看到 "int64_t" 而直接失效。
                if name == "self" {
                    if let Some(s) = &self.cur_self_struct {
                        return s.clone();
                    }
                }
                self.var_types.get(name).cloned()
                    .or_else(|| self.self_fields.get(name).cloned())
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

                // 枚举静态方法 `Tag.name(v)` → str。不在这里登记的话，
                // `"x" + Tag.name(t)` 会被当成 str + int 生成指针加法。
                if let Expr::Field(obj, method) = callee.as_ref() {
                    if method == "name" {
                        if let Expr::Ident(tname) = obj.as_ref() {
                            if self.enums.contains_key(tname) {
                                return "const char*".to_string();
                            }
                        }
                    }
                }

                // Vec 的方法：len → int，is_empty → bool，pop/last → 元素类型。
                // 插值 `say("{v.pop()}")` 靠这个选对 printf 格式。
                if let Expr::Field(obj, method) = callee.as_ref() {
                    if self.infer_type(obj) == "LumeVec*" {
                        match method.as_str() {
                            "len" | "length" | "reserve" => return "int64_t".to_string(),
                            "is_empty" => return "bool".to_string(),
                            "pop" | "last" | "push" | "clear" | "free" => {
                                if let Some(e) = self.inferred_vec_elem(obj) {
                                    return e;
                                }
                            }
                            _ => {}
                        }
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

                // 方法调用。用 infer_type(基) 而不是「基必须是局部变量 Ident」——
                // 否则 `self.divmod(o).q` 里 obj 是 `self`，`x.divmod(y).q` 里
                // obj 是 Call，都会退化成 int64_t（声明成 int 再赋结构体，C 报错）。
                if let Expr::Field(obj, method_name) = callee.as_ref() {
                    let sname = self.infer_type(obj);
                    if let Some(ret) = self
                        .methods
                        .get(&(sname, method_name.clone()))
                    {
                        return ret.clone();
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

    /// 表达式如果是 Vec[T]（的句柄），返回元素 C 类型 T。
    ///
    /// Vec 的 C 类型统一是 `LumeVec*`，元素类型只记在编译器的几张表里：
    /// 变量（vec_elems）、结构体字段（vec_field_elems）、函数/方法的返回
    /// （fn_vec_elems / method_vec_elems）。取元素、push、pop 都要靠它
    /// 决定强转类型。
    pub(crate) fn inferred_vec_elem(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Ident(n) => self.vec_elems.get(n).cloned().or_else(|| {
                // 方法体里的裸字段名（toks[i]）
                let sname = self.cur_self_struct.as_ref()?;
                self.vec_field_elems
                    .get(&(sname.clone(), n.clone()))
                    .cloned()
            }),
            Expr::Field(base, fname) => {
                let sname = match base.as_ref() {
                    Expr::Ident(b) if b == "self" => self.cur_self_struct.clone(),
                    Expr::Ident(b) => self.var_types.get(b).cloned(),
                    _ => None,
                }?;
                self.vec_field_elems.get(&(sname, fname.clone())).cloned()
            }
            Expr::Call(callee, _) => match callee.as_ref() {
                Expr::Ident(fname) => self.fn_vec_elems.get(fname).cloned(),
                Expr::Field(obj, m) => {
                    let sname = self.infer_type(obj);
                    self.method_vec_elems.get(&(sname, m.clone())).cloned()
                }
                _ => None,
            },
            _ => None,
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
/// 从字段类型里收出「可能指向别的 struct」的名字。
///
/// 为什么要递归：`Dim?` / `Vec[Dim]` / `[Dim; 3]` 的 C 类型都含 `Dim`，
/// 只要前者需要 Dim 先定义（`Vec` 除外 —— 它是 `LumeVec*`，但多收一个
/// 名字只会让排序更保守，不会出错）。
pub(crate) fn collect_struct_deps(t: &Type, out: &mut Vec<String>) {
    match t {
        Type::Named(n) => out.push(n.clone()),
        Type::List(i) | Type::Optional(i) | Type::Error(i) => collect_struct_deps(i, out),
        Type::Array(i, _) => collect_struct_deps(i, out),
        Type::Map(k, v) => {
            collect_struct_deps(k, out);
            collect_struct_deps(v, out);
        }
        Type::Tuple(v) => {
            for x in v {
                collect_struct_deps(x, out);
            }
        }
        Type::Generic(base, args) => {
            collect_struct_deps(base, out);
            for a in args {
                collect_struct_deps(a, out);
            }
        }
        _ => {}
    }
}
