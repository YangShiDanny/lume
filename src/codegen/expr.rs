// src/codegen/expr.rs
use super::Codegen;
use crate::ast::*;

impl Codegen {
    pub(crate) fn gen_repeat_array_init(
        &mut self,
        name: &str,
        ty: Option<&Type>,
        init: &Expr,
        fields: Option<&[String]>,
    ) -> Option<(String, usize)> {
        let (value, count) = match init {
            Expr::Repeat(value, count) => (value.as_ref(), *count),
            _ => return None,
        };
        let elem_ty = match ty {
            Some(Type::Array(inner, Some(_))) => self.gen_type(inner),
            _ => self.infer_type(value),
        };
        let value_code = match fields {
            Some(fields) => self.gen_expr_with_self(value, fields),
            None => self.gen_expr(value),
        };
        self.push_line(&format!("{} {}[{}];", elem_ty, name, count));

        let byte_value = match (elem_ty.as_str(), value) {
            ("int8_t" | "uint8_t", Expr::Int(0)) => Some(0),
            ("int8_t" | "uint8_t", Expr::Int(1)) => Some(1),
            _ => None,
        };
        if let Some(byte_value) = byte_value {
            self.push_line(&format!("memset({}, {}, sizeof({}));", name, byte_value, name));
        } else {
            let id = self.repeat_counter;
            self.repeat_counter += 1;
            let value_name = format!("_lume_repeat_value_{}", id);
            let index_name = format!("_lume_repeat_index_{}", id);
            self.push_line(&format!("const {} {} = {};", elem_ty, value_name, value_code));
            self.push_line(&format!(
                "for (int64_t {} = 0; {} < {}LL; {}++) {{ {}[{}] = {}; }}",
                index_name, index_name, count, index_name, name, index_name, value_name
            ));
        }
        Some((elem_ty, count))
    }

    pub(crate) fn gen_local_array_init(
        &mut self,
        ty: Option<&Type>,
        init: &Expr,
        fields: Option<&[String]>,
    ) -> Option<(String, usize, String)> {
        let (elem_ty, len) = match init {
            Expr::List(items) => {
                let declared = match ty {
                    Some(Type::Array(inner, Some(len))) => {
                        Some((self.gen_type(inner), *len))
                    }
                    _ => None,
                };
                declared.unwrap_or_else(|| {
                    let elem_ty = items.first()
                        .map(|item| self.infer_type(item))
                        .unwrap_or_else(|| "int64_t".to_string());
                    (elem_ty, items.len())
                })
            }
            Expr::Repeat(value, len) => {
                let elem_ty = match ty {
                    Some(Type::Array(inner, Some(_))) => self.gen_type(inner),
                    _ => self.infer_type(value),
                };
                (elem_ty, *len)
            }
            _ => return None,
        };
        let initializer = match fields {
            Some(fields) => self.gen_expr_with_self(init, fields),
            None => self.gen_expr(init),
        };
        Some((elem_ty, len, initializer))
    }

    pub(crate) fn gen_expr(&mut self, expr: &Expr) -> String {
        match expr {
            Expr::Int(n) => format!("{}LL", n),
            Expr::Float(f) => {
                let s = format!("{}", f);
                if s.contains('.') || s.contains('e') {
                    s
                } else {
                    format!("{}.0", s)
                }
            }
            Expr::Str(s) => {
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{}\"", escaped)
            }
            // 带插值的字符串出现在非 say 位置时，退化成纯文本拼接
            Expr::FmtStr(parts) => {
                let mut s = String::new();
                for p in parts {
                    if let StrPart::Text(t) = p {
                        s.push_str(t);
                    }
                }
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{}\"", escaped)
            }
            Expr::Rune(c) => format!("'{}'", c),
            Expr::Color(c) => format!("0x{:06X}", c),
            Expr::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
            Expr::Nothing => "NULL".to_string(),
            Expr::Ident(name) => {
                if name == "None" {
                    let tag = self.opt_tag("int64_t");
                    return format!("({}){{0, 0}}", tag);
                }
                name.clone()
            }
            Expr::It => "_it".to_string(),

            Expr::Binary(l, op, r) => {
                let ls = self.gen_expr(l);
                let rs = self.gen_expr(r);

                // 除以 2^n → 右移（有符号加修正）
                if let BinOp::Div = op {
                    if let Expr::Int(d) = r.as_ref() {
                        if *d > 1 && (*d as u64).is_power_of_two() {
                            let shift = (*d as u64).trailing_zeros();
                            let is_unsigned = match l.as_ref() {
                                Expr::Ident(name) => self.var_types.get(name)
                                    .map(|t| t.starts_with("uint"))
                                    .unwrap_or(false),
                                _ => false,
                            };
                            if is_unsigned {
                                return format!("({} >> {})", ls, shift);
                            }
                            return format!("lume_sdiv_pow2({}, {}LL)", ls, shift);
                        }
                    }
                }
                // 非负整数取模 2^n → 位与
                if let BinOp::Mod = op {
                    if let Expr::Int(b) = r.as_ref() {
                        if *b > 0 && (*b as u64).is_power_of_two() {
                            let is_non_neg = match l.as_ref() {
                                Expr::Ident(name) => {
                                    self.non_neg_vars.contains(name)
                                        || self.var_types.get(name)
                                            .map(|t| t.starts_with("uint"))
                                            .unwrap_or(false)
                                }
                                _ => false,
                            };
                            if is_non_neg {
                                let mask = (*b as u64) - 1;
                                return format!("({} & {}LL)", ls, mask as i64);
                            }
                        }
                    }
                }
                let op_str = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Mod => "%",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Lt => "<",
                    BinOp::Gt => ">",
                    BinOp::Le => "<=",
                    BinOp::Ge => ">=",
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                    BinOp::Pipe => {
                        // a |> f()   →  f(a)
                        // a |> f(b)  →  f(a, b)
                        // 走 gen_call_args：尾部默认值在这里展开，
                        // 数组实参的长度伴随参数也在这里补
                        // （以前手拼参数列表，这两样都会漏）。
                        let (callee, extra): (&Expr, &[Expr]) = match r.as_ref() {
                            Expr::Call(callee, args) => (callee.as_ref(), args.as_slice()),
                            other => (other, &[]),
                        };
                        let mut all = vec![l.as_ref().clone()];
                        all.extend_from_slice(extra);
                        let arg_strs = self.gen_call_args(callee, &all, None);
                        let callee_str = self.gen_expr(callee);
                        return format!("{}({})", callee_str, arg_strs.join(", "));
                    }
                };
                // 字符串是值语义：六个比较运算符都要比内容，不能比指针。
                // C 里 char* 直接 == 比的是地址，内容相同的两个字符串
                // 会判成不等，而且是静默错。
                if matches!(
                    op,
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge
                ) && self.is_str_expr(l)
                    && self.is_str_expr(r)
                {
                    let cmp_op = match op {
                        BinOp::Eq => "==",
                        BinOp::Ne => "!=",
                        BinOp::Lt => "<",
                        BinOp::Gt => ">",
                        BinOp::Le => "<=",
                        BinOp::Ge => ">=",
                        _ => "==",
                    };
                    return format!("(strcmp({}, {}) {} 0)", ls, rs, cmp_op);
                }
                // 整数除法/取模：除零和 INT64_MIN / -1 在 C 里是 UB。
                // 包一层运行时检查（double 是 IEEE，inf 有定义，不包）。
                // -O3 下 static inline 会被内联，常量除数直接消掉分支。
                if matches!(op, BinOp::Div | BinOp::Mod) {
                    let lt = self.infer_type(l);
                    if lt != "double" && lt != "float" {
                        let fname = match op {
                            BinOp::Div => "lume_checked_div_i64",
                            _ => "lume_checked_mod_i64",
                        };
                        return format!("{}({}, {})", fname, ls, rs);
                    }
                }
                format!("({} {} {})", ls, op_str, rs)
            }

            Expr::Unary(op, e) => {
                let es = self.gen_expr(e);
                let op_str = match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                };
                format!("({}{})", op_str, es)
            }

            // ! 强制解包：has==0 直接运行时报错（崩得明明白白）
            Expr::Force(e) => {
                let ec = self.gen_expr(e);
                format!(
                    "({{ __typeof__({}) _o = {}; if (!_o.has) {{ fprintf(stderr, \"lume: 对 None 用了 !\\n\"); abort(); }} _o.val; }})",
                    ec, ec
                )
            }

            // ? 传播：has==0 提前返回 None（要求当前函数返回 T?）
            Expr::Try(e) => {
                let ec = self.gen_expr(e);
                let ret = &self.cur_ret_c;
                format!(
                    "({{ __typeof__({}) _t = {}; if (!_t.has) {{ return ({}){{0, 0}}; }} _t.val; }})",
                    ec, ec, ret
                )
            }

            // 切片在表达式位置退化成指针值（长度走伴随参数那条路，
            // 见 gen_call_args / gen_slice_let）。直接内联用会丢长度，
            // 所以只允许出现在 let 和函数实参里，checker 会拦其他位置。
            Expr::Slice(..) => self.slice_ptr_expr(expr),

            Expr::Call(callee, args) => {
                // reduce 是单趟归约、返回标量，可以内联在表达式位置
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "reduce" {
                        if let Some(c) = self.gen_reduce_call(expr) {
                            return c;
                        }
                    }
                }
                // map/filter 产出两个值（指针+长度），只能在 let 位置
                // （见 gen_hof_let）。表达式位置走到这里说明用法不对，
                // 交给 checker 报错。
                let _ = args;

                // 内置构造：Some(x) / None
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "Some" {
                        if let Some(x) = args.first() {
                            let elem_c = self.infer_type(x);
                            let tag = self.opt_tag(&elem_c);
                            let xc = self.gen_expr(x);
                            return format!("(({}){{1, {}}})", tag, xc);
                        }
                        let tag = self.opt_tag("int64_t");
                        return format!("({}){{0, 0}}", tag);
                    }
                    if name == "None" {
                        let tag = self.opt_tag("int64_t");
                        return format!("({}){{0, 0}}", tag);
                    }
                }

                // 内置函数
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "say" {
                        return self.gen_say(args);
                    }
                    if name == "write" {
                        return self.gen_write(args);
                    }
                    if name == "flush" {
                        return "lume_log_flush()".to_string();
                    }
                    if name == "now" {
                        return "lume_now_ns()".to_string();
                    }
                    if let Some(c_name) = builtin_map(name) {
                        let args_str: Vec<String> = args.iter()
                            .map(|a| self.gen_expr(a))
                            .collect();
                        return format!("{}({})", c_name, args_str.join(", "));
                    }
                }

                // a.len() —— 数组长度。
                // 局部字面量是常量，数组参数是伴随参数 {name}_len
                if let Expr::Field(obj, method) = callee.as_ref() {
                    if method == "len" && args.is_empty() {
                        if let Some(len) = self.array_len_of(obj) {
                            return len;
                        }
                    }
                }

                // 泛型调用
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.generic_fns.contains_key(fname) {
                        return self.monomorphize_call(fname, args);
                    }
                }

                // 方法调用 p.len()
                if let Expr::Field(obj, method_name) = callee.as_ref() {
                    if let Expr::Ident(obj_name) = obj.as_ref() {
                        if let Some(struct_name) = self.var_types.get(obj_name).cloned() {
                            let key = (struct_name.clone(), method_name.clone());
                            if self.methods.contains_key(&key) {
                                let arr_flags = self
                                    .method_array_params
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or_default();
                                let mut all_args = vec![format!("&{}", obj_name)];
                                for (i, a) in args.iter().enumerate() {
                                    all_args.push(self.gen_arg(a));
                                    if arr_flags.get(i).copied().unwrap_or(false) {
                                        match self.array_len_of(a) {
                                            Some(len) => all_args.push(len),
                                            None => all_args.push(format!(
                                                "/* 无法确定第 {} 个实参的数组长度 */0LL",
                                                i + 1
                                            )),
                                        }
                                    }
                                }
                                return format!(
                                    "{}_{}({})",
                                    struct_name, method_name, all_args.join(", ")
                                );
                            }
                        }
                    }
                }

                // 普通调用。数组实参后面自动补一个长度实参，
                // 和被调用函数的伴随参数一一对应（见 gen_params）
                let callee_str = self.gen_expr(callee);
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.array_ret_fns.contains(fname) {
                        return format!(
                            "/* {}() 返回数组，请写成 `let r = {}(...)` 才能拿到长度 */",
                            fname, fname
                        );
                    }
                }
                let arg_strs = self.gen_call_args(callee, args, None);
                format!("{}({})", callee_str, arg_strs.join(", "))
            }

            Expr::Field(obj, name) => {
                // a.len —— 数组长度（局部字面量和数组参数都支持）
                if name == "len" {
                    if let Some(len) = self.array_len_of(obj) {
                        return len;
                    }
                }
                let o = self.gen_expr(obj);
                format!("{}.{}", o, name)
            }

            Expr::Index(obj, idx) => {
                let o = self.gen_expr(obj);
                let i = match gen_index(idx) {
                    Some(s) => s,
                    None => self.gen_expr(idx),
                };
                format!("{}[{}]", o, i)
            }

            Expr::If(cond, then, els) => self.gen_if_expr(cond, then, els, None),

            Expr::StructLit(name, fields) => {
                // 名字不是结构体 → 命名参数函数调用 f(a: 1)（checker 已放过）
                if let Some(call) = self.gen_named_call(name, fields, None) {
                    return call;
                }
                let field_strs: Vec<String> = fields.iter()
                    .map(|(fname, fval)| {
                        let v = self.gen_expr(fval);
                        format!(".{} = {}", fname, v)
                    })
                    .collect();
                let tag = self.struct_tags.get(name).copied().unwrap_or(0);
                format!("({}){{ ._tag = {}LL, {} }}", name, tag, field_strs.join(", "))
            }

            Expr::List(items) => {
                let s: Vec<String> = items.iter().map(|e| self.gen_expr(e)).collect();
                format!("{{{}}}", s.join(", "))
            }

            Expr::Repeat(value, count) => {
                let item = self.gen_expr(value);
                format!("{{{}}}", vec![item; *count].join(", "))
            }

            _ => "/* TODO */".to_string(),
        }
    }

    /// 表达式是不是字符串。用来把 `==` 从指针比较改成内容比较。
    pub(crate) fn is_str_expr(&self, e: &Expr) -> bool {
        self.infer_type(e) == "const char*"
    }

    /// `let s = a[i..j]`：切片要产出两个值（指针 + 长度），
    /// 和数组返回函数的 `let r = f()` 一样拆成三行，长度才有地方落。
    /// 返回 None 表示 init 不是切片，走普通 let。
    ///
    /// 切片是**借用**，不拷贝：s 和 a 指向同一块内存，a 改了 s 也跟着改。
    /// 零开销，和 Rust 的 `&a[i..j]` 同语义。
    pub(crate) fn gen_slice_let(
        &mut self,
        name: &str,
        init: &Expr,
        fields: Option<&[String]>,
    ) -> Option<Vec<String>> {
        let (obj, start, end, inclusive) = match init {
            Expr::Slice(o, s, e, inc) => (o.as_ref(), s, e, *inc),
            _ => return None,
        };
        // self 场景（结构体方法内）元素类型仍走 infer_type：
        // var_types 表在方法里登记过，self 的字段访问也走同一张表。
        //
        // infer_type 给数组变量的是 "int64_t*"，这里要**元素**类型
        // （声明指针时补 *；array_elem_types / var_types 也存元素类型）。
        let base_ty = self.infer_type(obj);
        let elem_ty = base_ty
            .strip_suffix('*')
            .unwrap_or(&base_ty)
            .to_string();
        let base = match fields {
            Some(f) => self.gen_expr_with_self(obj, f),
            None => self.gen_expr(obj),
        };
        let (s, e, base_len) = self.slice_bounds(obj, start, end, inclusive);
        let ptr = format!(
            "({} + lume_slice_off({}, {}, {}, {}))",
            base, base, s, e, base_len
        );
        let len = format!("lume_slice_len({}, {}, {}, {})", base, s, e, base_len);
        // 登记成长度和普通数组一样，后面 s.len / for v in s 都能用
        self.array_len_exprs.insert(name.to_string(), format!("{}_len", name));
        self.array_elem_types.insert(name.to_string(), elem_ty.clone());
        self.var_types.insert(name.to_string(), format!("{}*", elem_ty));
        Some(vec![
            format!("{}* {};", elem_ty, name),
            format!("int64_t {}_len;", name),
            format!("{} = {};", name, ptr),
            format!("{}_len = {};", name, len),
        ])
    }

/// 数组实参的长度表达式。
    /// 数组变量查 array_len_exprs；数组字面量直接用元素个数
    /// （C99 复合字面量的生命周期覆盖整次调用，作为实参是安全的）。
    pub(crate) fn array_len_of(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Ident(n) => self.array_len_exprs.get(n).cloned(),
            Expr::List(items) => Some(format!("{}LL", items.len())),
            Expr::Repeat(_, count) => Some(format!("{}LL", count)),
            _ => None,
        }
    }

    /// `&mut` 版 array_len_of：能算切片的伴生长度。
    /// 切片的原数组本身可能是切片（`xs[1..][0..2]`），
    /// 递归下去才能拿到长度，所以必须在 &mut self 上做。
    pub(crate) fn array_len_of_mut(&mut self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Slice(obj, s, e, inc) => self.slice_len_expr(obj, s, e, *inc),
            other => self.array_len_of(other),
        }
    }

    /// 切片的边界三元组（起点 / 终点 / 原长度）的 C 表达式。
    /// 缺省：起点 `0LL`，终点 = 原数组长度。`..=j` 的终点是 `j + 1`。
    /// 一律带运行期边界检查 —— 切片越界在 C 里是静默的越界读写。
    /// 和除零检查同一个思路：崩得明明白白好过静默算错。
    pub(crate) fn slice_bounds(
        &mut self,
        obj: &Expr,
        start: &Option<Box<Expr>>,
        end: &Option<Box<Expr>>,
        inclusive: bool,
    ) -> (String, String, String) {
        let base_len = self
            .array_len_of_mut(obj)
            .unwrap_or_else(|| "/* 未知原长度 */0LL".to_string());
        let s = match start {
            Some(e) => self.gen_expr(e),
            None => "0LL".to_string(),
        };
        let raw_end = match end {
            Some(e) => self.gen_expr(e),
            None => base_len.clone(),
        };
        let e = if inclusive {
            format!("({} + 1LL)", raw_end)
        } else {
            raw_end
        };
        (s, e, base_len)
    }

    /// 切片在表达式位置的指针值：`a + off`。
    /// 长度不在这里 —— 长度走伴随参数那条路（gen_slice_let / gen_call_args）。
    pub(crate) fn slice_ptr_expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::Slice(obj, start, end, inclusive) => {
                let base = self.gen_expr(obj);
                let (s, ev, len) =
                    self.slice_bounds(obj, start, end, *inclusive);
                format!(
                    "({} + lume_slice_off({}, {}, {}, {}))",
                    base, base, s, ev, len
                )
            }
            _ => self.gen_expr(e),
        }
    }

    /// 切片的长度 C 表达式，供函数实参的伴随长度参数用。
    /// 原数组的长度拿不到就返回 None（调用方发一条注释占位）。
    pub(crate) fn slice_len_expr(
        &mut self,
        obj: &Expr,
        start: &Option<Box<Expr>>,
        end: &Option<Box<Expr>>,
        inclusive: bool,
    ) -> Option<String> {
        if self.array_len_of(obj).is_none()
            && !matches!(obj, Expr::Slice(..))
        {
            return None;
        }
        let base = self.gen_expr(obj);
        let (s, ev, len) = self.slice_bounds(obj, start, end, inclusive);
        Some(format!(
            "lume_slice_len({}, {}, {}, {})",
            base, s, ev, len
        ))
    }

    /// `if c { a } else { b }` 作为表达式 → C 三元运算符。
    /// 两个分支都必须恰好一条表达式语句，否则三元装不下，
    /// 也不退化成 GNU 语句表达式（MSVC 不认）。
    pub(crate) fn gen_if_expr(
        &mut self,
        cond: &Expr,
        then: &Block,
        els: &Option<Block>,
        fields: Option<&[String]>,
    ) -> String {
        let te = match single_expr(then) {
            Some(e) => e,
            None => {
                return "/* if 表达式：then 分支只支持单条表达式 */".to_string()
            }
        };
        let ee = match els.as_ref().and_then(single_expr) {
            Some(e) => e,
            None => {
                return "/* if 表达式：需要 else 分支，且只支持单条表达式 */"
                    .to_string()
            }
        };
        let c = match fields {
            Some(f) => self.gen_expr_with_self(cond, f),
            None => self.gen_expr(cond),
        };
        let ts = match fields {
            Some(f) => self.gen_expr_with_self(te, f),
            None => self.gen_expr(te),
        };
        let es = match fields {
            Some(f) => self.gen_expr_with_self(ee, f),
            None => self.gen_expr(ee),
        };
        format!(
            "({}) ? ({}) : ({})",
            bare_parens(&c),
            bare_parens(&ts),
            bare_parens(&es)
        )
    }

    /// 实参专用生成器。数组字面量在实参位置必须写成 C99 复合字面量
    /// `(int64_t[]){1,2,3}` —— 裸的 `{1,2,3}` 在初始化位置合法，
    /// 但出现在 `f({1,2,3})` 里根本不是表达式，clang 直接报语法错。
    /// 复合字面量的生命周期覆盖它所在的整个块，作为实参是安全的。
    pub(crate) fn gen_arg(&mut self, e: &Expr) -> String {
        match e {
            Expr::List(items) if !items.is_empty() => {
                let elem_ty = self.infer_type(&items[0]);
                let strs: Vec<String> = items.iter().map(|x| self.gen_expr(x)).collect();
                return format!("({}[]){{{}}}", elem_ty, strs.join(", "));
            }
            Expr::Repeat(value, count) => {
                let elem_ty = self.infer_type(value);
                let item = self.gen_expr(value);
                return format!("({}[]){{{}}}", elem_ty, vec![item; *count].join(", "));
            }
            _ => {}
        }
        self.gen_expr(e)
    }

    pub(crate) fn gen_arg_with_self(&mut self, e: &Expr, fields: &[String]) -> String {
        match e {
            Expr::List(items) if !items.is_empty() => {
                let elem_ty = self.infer_type(&items[0]);
                let strs: Vec<String> = items
                    .iter()
                    .map(|x| self.gen_expr_with_self(x, fields))
                    .collect();
                return format!("({}[]){{{}}}", elem_ty, strs.join(", "));
            }
            Expr::Repeat(value, count) => {
                let elem_ty = self.infer_type(value);
                let item = self.gen_expr_with_self(value, fields);
                return format!("({}[]){{{}}}", elem_ty, vec![item; *count].join(", "));
            }
            _ => {}
        }
        self.gen_expr_with_self(e, fields)
    }

    /// 生成一次普通函数调用的实参列表，数组实参后面自动补长度。
    /// 声明处用 gen_params 展开伴随参数，两边规则必须一致。
    /// fields 非空表示在结构体方法里，表达式要走 with_self 版本。
    pub(crate) fn gen_call_args(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        fields: Option<&[String]>,
    ) -> Vec<String> {
        // 默认参数：位置参数从前往后填，缺的尾部参数在调用处展开默认值
        // （C 语义——默认值表达式在调用处求值，只能引用调用处可见的名字）。
        // checker 已保证个数在 [required, total]，这里只负责补尾。
        let defaults: Vec<Option<Expr>> = match callee {
            Expr::Ident(fname) => self
                .fn_param_defaults
                .get(fname)
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut args: Vec<Expr> = args.to_vec();
        if args.len() < defaults.len() {
            for d in defaults.iter().skip(args.len()) {
                match d {
                    Some(e) => args.push(e.clone()),
                    // 理论上 checker 已挡住（缺参且无默认值），防御
                    None => break,
                }
            }
        }
        let arr_flags: Option<Vec<bool>> = match callee {
            Expr::Ident(fname) => self.fn_array_params.get(fname).cloned(),
            _ => None,
        };
        let mut out: Vec<String> = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            // 切片实参：先取指针，长度由下面的 need_len 分支补。
            // 不能走 gen_arg —— 它会把 Slice 当普通表达式降级处理。
            let s = if matches!(a, Expr::Slice(..)) {
                self.slice_ptr_expr(a)
            } else {
                match fields {
                    Some(f) => self.gen_arg_with_self(a, f),
                    None => self.gen_arg(a),
                }
            };
            out.push(s);
            let need_len = arr_flags
                .as_ref()
                .and_then(|fl| fl.get(i))
                .copied()
                .unwrap_or(false);
            if need_len {
                match a {
                    Expr::Slice(obj, s, e, inc) => {
                        match self.slice_len_expr(obj, s, e, *inc) {
                            Some(len) => out.push(len),
                            None => out.push(format!(
                                "/* 无法确定第 {} 个实参的切片长度 */0LL",
                                i + 1
                            )),
                        }
                    }
                    other => match self.array_len_of_mut(other) {
                        Some(len) => out.push(len),
                        None => out.push(format!(
                            "/* 无法确定第 {} 个实参的数组长度 */0LL",
                            i + 1
                        )),
                    },
                }
            }
        }
        out
    }

    /// 命名参数函数调用 f(b: 2, a: 1) → C 的位置参数调用。
    ///
    /// parser 把 `f(a: 1)` 解析成 StructLit（和 Point(x: 1) 完全同形），
    /// 这里按语义分流：名字在 structs 表里 → 结构体构造，返回 None 交给
    /// 调用方按老路生成；否则查 fn_param_names 重排，None 槽位用默认值
    /// 表达式补，再走 gen_call_args（数组长度伴随参数一起补）。
    ///
    /// self_fields 非空表示在结构体方法里，实参要走 with_self 版本。
    /// 返回 None = 不是命名参数调用（结构体构造，或名字完全未知）。
    fn gen_named_call(
        &mut self,
        name: &str,
        fields: &[(String, Expr)],
        self_fields: Option<&[String]>,
    ) -> Option<String> {
        if self.structs.contains_key(name) {
            return None;
        }
        // 名字既不是结构体也不是用户函数：保持老行为（生成一个必然
        // 编译不过的结构体字面量），错误由 checker 报。
        let param_names = self.fn_param_names.get(name).cloned()?;
        let slots = match reorder_named_args(fields, &param_names) {
            Ok(s) => s,
            // checker 已经报过错了，这里兜个底别 panic
            Err(msg) => return Some(format!("/* 命名参数错误: {} */", msg)),
        };
        let defaults = self
            .fn_param_defaults
            .get(name)
            .cloned()
            .unwrap_or_default();
        let mut args: Vec<Expr> = Vec::with_capacity(slots.len());
        for (i, slot) in slots.iter().enumerate() {
            match slot {
                Some(e) => args.push(e.clone()),
                None => match defaults.get(i).and_then(|d| d.clone()) {
                    Some(d) => args.push(d),
                    // checker 已验证过有默认值，防御性填充
                    None => args.push(Expr::Int(0)),
                },
            }
        }
        let arg_strs = self.gen_call_args(&Expr::Ident(name.to_string()), &args, self_fields);
        Some(format!("{}({})", name, arg_strs.join(", ")))
    }

    /// `let r = f(...)` 且 f 返回数组：C 函数多一个长度出参，
    /// 必须拆成「声明指针 + 声明长度 + 赋值」三行，长度才有地方落。
    /// 返回 None 表示不是这种情况，走普通 let。
    pub(crate) fn gen_array_let(
        &mut self,
        name: &str,
        init: &Expr,
        fields: Option<&[String]>,
    ) -> Option<Vec<String>> {
        let (callee, args) = match init {
            Expr::Call(c, a) => (c.as_ref(), a),
            _ => return None,
        };
        let fname = match callee {
            Expr::Ident(f) => f.clone(),
            _ => return None,
        };
        if !self.array_ret_fns.contains(&fname) {
            return None;
        }
        // 只脱一层 *：`int64_t*` → `int64_t`，`const char**` → `const char*`
        // （字符串数组的元素是字符串，指针还要留一层）。
        // 别用 trim_end_matches('*') —— 它会把两个 * 全脱掉，
        // str_split 的返回值就变成 const char，声明和插值格式全错。
        let ret_ty = self
            .fn_ret_types
            .get(&fname)
            .cloned()
            .unwrap_or_else(|| "int64_t*".to_string());
        let elem_ty = ret_ty
            .strip_suffix('*')
            .unwrap_or(ret_ty.as_str())
            .to_string();
        let mut parts = self.gen_call_args(callee, args, fields);
        parts.push(format!("&{}_len", name));
        // 登记成长度和普通数组一样，后面 r.len / for v in r 都能用
        self.array_len_exprs
            .insert(name.to_string(), format!("{}_len", name));
        self.array_elem_types
            .insert(name.to_string(), elem_ty.clone());
        self.var_types
            .insert(name.to_string(), format!("{}*", elem_ty));
        // builtin（如 str_split）在 C 里叫 lume_str_split
        let c_name = builtin_map(&fname)
            .map(|s| s.to_string())
            .unwrap_or_else(|| fname.clone());
        Some(vec![
            format!("{}* {};", elem_ty, name),
            format!("int64_t {}_len;", name),
            format!("{} = {}({});", name, c_name, parts.join(", ")),
        ])
    }

    /// 返回数组的两种合法形式：
    ///   return [1,2,3]  → malloc 拷贝（栈上的复合字面量离开函数就悬空）
    ///   return a        → 把入参数组直接传出去，长度一起带出去
    /// 其他形式返回 None，走普通 return
    pub(crate) fn gen_array_return(
        &mut self,
        e: &Expr,
        fields: Option<&[String]>,
    ) -> Option<Vec<String>> {
        match e {
            Expr::List(items) => {
                let elem_ty = if let Some(first) = items.first() {
                    self.infer_type(first)
                } else {
                    "int64_t".to_string()
                };
                let n = items.len();
                let mut out = vec![format!(
                    "{}* _r = ({}*)malloc((size_t){} * sizeof({}));",
                    elem_ty, elem_ty, n, elem_ty
                )];
                for (i, it) in items.iter().enumerate() {
                    let v = match fields {
                        Some(f) => self.gen_expr_with_self(it, f),
                        None => self.gen_expr(it),
                    };
                    out.push(format!("_r[{}] = {};", i, v));
                }
                out.push(format!("*_ret_len = {}LL;", n));
                out.push("return _r;".to_string());
                Some(out)
            }
            Expr::Repeat(value, n) => {
                let elem_ty = self.infer_type(value);
                let initial = match fields {
                    Some(fields) => self.gen_expr_with_self(value, fields),
                    None => self.gen_expr(value),
                };
                let id = self.repeat_counter;
                self.repeat_counter += 1;
                let repeated_value = format!("_lume_return_repeat_{}", id);
                let result_name = format!("_lume_repeat_return_{}", id);
                let index_name = format!("_lume_repeat_index_{}", id);
                Some(vec![
                    format!(
                        "{}* {} = ({}*)malloc((size_t){} * sizeof({}));",
                        elem_ty, result_name, elem_ty, n, elem_ty
                    ),
                    format!("{} {} = {};", elem_ty, repeated_value, initial),
                    format!(
                        "for (int64_t {} = 0; {} < {}LL; {}++) {{ {}[{}] = {}; }}",
                        index_name, index_name, n, index_name,
                        result_name, index_name, repeated_value
                    ),
                    format!("*_ret_len = {}LL;", n),
                    format!("return {};", result_name),
                ])
            }
            Expr::Ident(src) => {
                let len = self.array_len_exprs.get(src).cloned()?;
                if self.array_params.contains(src) {
                    // 入参数组指向调用方的内存，活得过函数返回，直接传出去
                    Some(vec![
                        format!("*_ret_len = {};", len),
                        format!("return {};", src),
                    ])
                } else {
                    // 局部数组在 C 里是栈上的 `T x[N]`，返回它就是悬空指针。
                    // 必须 malloc 拷一份。
                    let elem_ty = self.array_elem_types.get(src).cloned()
                        .unwrap_or_else(|| "int64_t".to_string());
                    Some(vec![
                        format!(
                            "{}* _r = ({}*)malloc((size_t)({}) * sizeof({}));",
                            elem_ty, elem_ty, len, elem_ty
                        ),
                        format!(
                            "for (int64_t _i = 0; _i < ({}); _i++) {{ _r[_i] = {}[_i]; }}",
                            len, src
                        ),
                        format!("*_ret_len = {};", len),
                        "return _r;".to_string(),
                    ])
                }
            }
            _ => None,
        }
    }

    pub(crate) fn gen_expr_with_self(&mut self, expr: &Expr, fields: &[String]) -> String {
        match expr {
            // `self` 单独作为值出现时（f(self)、return self），
            // Lume 的 struct 是值类型，传的应该是结构体本身而不是指针。
            // _self 是 Point*，解引用成 Point 才和 `fn f(p: Point)` 对得上。
            // 注意 self.x / 裸字段名走的是别的分支，不受这里影响。
            Expr::Ident(name) if name == "self" => "(*_self)".to_string(),
            Expr::Ident(name) if fields.contains(name) => {
                format!("_self->{}", name)
            }
            // 带插值的字符串出现在非 say 位置时，退化成纯文本拼接
            Expr::FmtStr(parts) => {
                let mut s = String::new();
                for p in parts {
                    if let StrPart::Text(t) = p {
                        s.push_str(t);
                    }
                }
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{}\"", escaped)
            }

            Expr::Call(callee, args) => {
                // say（方法里的打印，插值要走 with_self 版本）
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "say" {
                        return self.gen_say_with(args, Some(fields));
                    }
                    if name == "write" {
                        return self.gen_write_with(args, Some(fields));
                    }
                    if name == "flush" {
                        return "lume_log_flush()".to_string();
                    }
                }
                // now
                if let Expr::Ident(name) = callee.as_ref() {
                    if name == "now" {
                        return "lume_now_ns()".to_string();
                    }
                    if let Some(c_name) = builtin_map(name) {
                        let args_str: Vec<String> = args.iter()
                            .map(|a| self.gen_expr_with_self(a, fields))
                            .collect();
                        return format!("{}({})", c_name, args_str.join(", "));
                    }
                }

                // a.len() —— 数组长度。
                // 局部字面量是常量，数组参数是伴随参数 {name}_len
                if let Expr::Field(obj, method) = callee.as_ref() {
                    if method == "len" && args.is_empty() {
                        if let Some(len) = self.array_len_of(obj) {
                            return len;
                        }
                    }
                }

                // 泛型调用
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.generic_fns.contains_key(fname) {
                        return self.monomorphize_call(fname, args);
                    }
                }

                // 方法调用
                if let Expr::Field(obj, method_name) = callee.as_ref() {
                    if let Expr::Ident(obj_name) = obj.as_ref() {
                        if let Some(struct_name) = self.var_types.get(obj_name).cloned() {
                            let key = (struct_name.clone(), method_name.clone());
                            if self.methods.contains_key(&key) {
                                let arr_flags = self
                                    .method_array_params
                                    .get(&key)
                                    .cloned()
                                    .unwrap_or_default();
                                let mut all_args = vec![format!("&{}", obj_name)];
                                for (i, a) in args.iter().enumerate() {
                                    all_args.push(self.gen_arg_with_self(a, fields));
                                    if arr_flags.get(i).copied().unwrap_or(false) {
                                        match self.array_len_of(a) {
                                            Some(len) => all_args.push(len),
                                            None => all_args.push(format!(
                                                "/* 无法确定第 {} 个实参的数组长度 */0LL",
                                                i + 1
                                            )),
                                        }
                                    }
                                }
                                return format!(
                                    "{}_{}({})",
                                    struct_name, method_name, all_args.join(", ")
                                );
                            }
                        }
                    }
                }

                let callee_str = self.gen_expr_with_self(callee, fields);
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.array_ret_fns.contains(fname) {
                        return format!(
                            "/* {}() 返回数组，请写成 `let r = {}(...)` 才能拿到长度 */",
                            fname, fname
                        );
                    }
                }
                let arg_strs = self.gen_call_args(callee, args, Some(fields));
                format!("{}({})", callee_str, arg_strs.join(", "))
            }

            Expr::Binary(l, op, r) => {
                let ls = self.gen_expr_with_self(l, fields);
                let rs = self.gen_expr_with_self(r, fields);

                // 除以 2^n → 右移（有符号加修正）
                if let BinOp::Div = op {
                    if let Expr::Int(d) = r.as_ref() {
                        if *d > 1 && (*d as u64).is_power_of_two() {
                            let shift = (*d as u64).trailing_zeros();
                            let is_unsigned = match l.as_ref() {
                                Expr::Ident(name) => self.var_types.get(name)
                                    .map(|t| t.starts_with("uint"))
                                    .unwrap_or(false),
                                _ => false,
                            };
                            if is_unsigned {
                                return format!("({} >> {})", ls, shift);
                            }
                            return format!("lume_sdiv_pow2({}, {}LL)", ls, shift);
                        }
                    }
                }
                
                // 非负整数取模 2^n → 位与
                if let BinOp::Mod = op {
                    if let Expr::Int(b) = r.as_ref() {
                        if *b > 0 && (*b as u64).is_power_of_two() {
                            let is_non_neg = match l.as_ref() {
                                Expr::Ident(name) => {
                                    self.non_neg_vars.contains(name)
                                        || self.var_types.get(name)
                                            .map(|t| t.starts_with("uint"))
                                            .unwrap_or(false)
                                }
                                _ => false,
                            };
                            if is_non_neg {
                                let mask = (*b as u64) - 1;
                                return format!("({} & {}LL)", ls, mask as i64);
                            }
                        }
                    }
                }
                let op_str = match op {
                    BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
                    BinOp::Div => "/", BinOp::Mod => "%",
                    BinOp::Eq => "==", BinOp::Ne => "!=",
                    BinOp::Lt => "<", BinOp::Gt => ">",
                    BinOp::Le => "<=", BinOp::Ge => ">=",
                    BinOp::And => "&&", BinOp::Or => "||",
                    BinOp::Pipe => {
                        let (callee, extra): (&Expr, &[Expr]) = match r.as_ref() {
                            Expr::Call(callee, args) => (callee.as_ref(), args.as_slice()),
                            other => (other, &[]),
                        };
                        let mut all = vec![l.as_ref().clone()];
                        all.extend_from_slice(extra);
                        let arg_strs = self.gen_call_args(callee, &all, Some(fields));
                        let callee_str = self.gen_expr_with_self(callee, fields);
                        return format!("{}({})", callee_str, arg_strs.join(", "));
                    }
                };
                // 同上：字符串比较走 strcmp
                if matches!(
                    op,
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge
                ) && self.is_str_expr(l)
                    && self.is_str_expr(r)
                {
                    let cmp_op = match op {
                        BinOp::Eq => "==",
                        BinOp::Ne => "!=",
                        BinOp::Lt => "<",
                        BinOp::Gt => ">",
                        BinOp::Le => "<=",
                        BinOp::Ge => ">=",
                        _ => "==",
                    };
                    return format!("(strcmp({}, {}) {} 0)", ls, rs, cmp_op);
                }
                // 整数除法/取模：除零和 INT64_MIN / -1 在 C 里是 UB。
                // 包一层运行时检查（double 是 IEEE，inf 有定义，不包）。
                // -O3 下 static inline 会被内联，常量除数直接消掉分支。
                if matches!(op, BinOp::Div | BinOp::Mod) {
                    let lt = self.infer_type(l);
                    if lt != "double" && lt != "float" {
                        let fname = match op {
                            BinOp::Div => "lume_checked_div_i64",
                            _ => "lume_checked_mod_i64",
                        };
                        return format!("{}({}, {})", fname, ls, rs);
                    }
                }
                format!("({} {} {})", ls, op_str, rs)
            }

            Expr::Unary(op, e) => {
                let es = self.gen_expr_with_self(e, fields);
                let op_str = match op {
                    UnOp::Neg => "-", UnOp::Not => "!",
                };
                format!("({}{})", op_str, es)
            }

            // self.x → _self->x（裸字段名由上面的 Ident 分支处理）
            Expr::Field(obj, name) => {
                if name == "len" {
                    if let Some(len) = self.array_len_of(obj) {
                        return len;
                    }
                }
                if let Expr::Ident(id) = obj.as_ref() {
                    if id == "self" && fields.contains(name) {
                        return format!("_self->{}", name);
                    }
                }
                let o = self.gen_expr_with_self(obj, fields);
                format!("{}.{}", o, name)
            }

            Expr::Index(obj, idx) => {
                let o = self.gen_expr_with_self(obj, fields);
                let i = match gen_index(idx) {
                    Some(s) => s,
                    None => self.gen_expr_with_self(idx, fields),
                };
                format!("{}[{}]", o, i)
            }

            Expr::If(cond, then, els) => self.gen_if_expr(cond, then, els, Some(fields)),

            Expr::StructLit(name, fields_list) => {
                if let Some(call) = self.gen_named_call(name, fields_list, Some(fields)) {
                    return call;
                }
                let field_strs: Vec<String> = fields_list.iter()
                    .map(|(fname, fval)| {
                        let v = self.gen_expr_with_self(fval, fields);
                        format!(".{} = {}", fname, v)
                    })
                    .collect();
                let tag = self.struct_tags.get(name).copied().unwrap_or(0);
                format!("({}){{ ._tag = {}LL, {} }}", name, tag, field_strs.join(", "))
            }

            Expr::Repeat(value, count) => {
                let item = self.gen_expr_with_self(value, fields);
                format!("{{{}}}", vec![item; *count].join(", "))
            }

            _ => self.gen_expr(expr),
        }
    }
}

/// 块里恰好一条表达式语句时返回它，否则 None。
/// if 表达式的两个分支都要过这个检查 —— 三元运算符只包得住单个表达式。
/// 剥掉一整串最外层的括号（仅当首尾确实是一对匹配括号时）。
/// if/while 的条件和三元的 cond 都用它：Binary 产出的 `(a == b)`
/// 再被外层包一层就成了 `((a == b))`，clang -Wparentheses-equality
/// 会为此报警。剥掉不影响优先级——本来就是最外层。
pub(crate) fn bare_parens(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'(' && bytes[bytes.len() - 1] == b')' {
        let mut depth: i32 = 0;
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'(' {
                depth += 1;
            } else if b == b')' {
                depth -= 1;
                if depth == 0 && i < bytes.len() - 1 {
                    // 提前闭合：说明括号不是全包，别剥
                    return s.to_string();
                }
            }
        }
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}

pub(crate) fn single_expr(b: &Block) -> Option<&Expr> {
    if b.stmts.len() == 1 {
        if let Stmt::Expr(e) = &b.stmts[0] {
            return Some(e);
        }
    }
    None
}

/// 下标位置的字面量整数不需要 `LL` 后缀：`a[0]` 比 `a[0LL]` 干净，
/// 对 clang 来说两者完全等价。返回 None 表示不是字面量，走常规生成。
fn gen_index(idx: &Expr) -> Option<String> {
    match idx {
        Expr::Int(n) => Some(format!("{}", n)),
        _ => None,
    }
}

fn builtin_map(name: &str) -> Option<&'static str> {
    Some(match name {
        // str
        "str_len" => "lume_str_len",
        "str_upper" => "lume_str_upper",
        "str_lower" => "lume_str_lower",
        "str_trim" => "lume_str_trim",
        "str_to_int" => "lume_str_to_int",
        "str_to_float" => "lume_str_to_float",
        "str_contains" => "lume_str_contains",
        "str_starts_with" => "lume_str_starts_with",
        "str_ends_with" => "lume_str_ends_with",
        "str_replace" => "lume_str_replace",
        "str_split" => "lume_str_split",
        // int
        "int_abs" => "lume_int_abs",
        "int_min" => "lume_int_min",
        "int_max" => "lume_int_max",
        // float
        "float_sqrt" | "sqrt" => "lume_float_sqrt",
        "float_abs" => "lume_float_abs",
        "float_pow" | "pow" => "lume_float_pow",
        "float_sin" | "sin" => "lume_float_sin",
        "float_cos" | "cos" => "lume_float_cos",
        "float_tan" | "tan" => "lume_float_tan",
        "float_floor" | "floor" => "lume_float_floor",
        "float_ceil" | "ceil" => "lume_float_ceil",
        "float_round" | "round" => "lume_float_round",
        // 内部（optimize 生成，用户写不出来）
        "__shl" => "lume_shl",
        "__bitand" => "lume_bitand",
        _ => return None,
    })
}