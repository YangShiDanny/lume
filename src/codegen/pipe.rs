// src/codegen/pipe.rs
//! 管道融合：把 `xs |> map(f) |> map(g) |> filter(p)` 编译成**单个** for 循环。
//!
//! 不融合时每段各自 malloc 一个中间数组、三趟内存遍历：
//! ```text
//! _t1 = malloc(N); for (...) _t1[i] = f(xs[i]);
//! _t2 = malloc(N); for (...) _t2[i] = g(_t1[i]);
//! _t3 = malloc(M); for (...) if (p(_t2[i])) _t3[k++] = _t2[i];
//! ```
//!
//! 融合后只有一个循环、一份输出：
//! ```c
//! for (int64_t i = 0; i < N; i++) {
//!     T _c = xs[i];      // 当前值
//!     _c = f(_c);        // map: 就地覆盖
//!     _c = g(_c);        // map
//!     if (p(_c)) _out[_pk++] = _c;   // filter: 条件保留
//! }
//! ```
//! 中间值全部落在**栈上一个标量**里，只有最终输出需要堆内存。
//! 内存 3N → 1 份输出，遍历 3 趟 → 1 趟。
//!
//! **只融合「每一段都是 map 或 filter」的链**。链尾是 reduce 时不融合
//! —— reduce 天然是单趟归约，本来就没有中间数组可省。

use super::Codegen;
use crate::ast::*;

/// 管道里的一段
enum Stage {
    /// 每个元素调 f，结果继续往下传
    Map(String),
    /// 谓词为真才保留（输出长度 ≤ 输入长度）
    Filter(String),
}

impl Stage {
    fn func(&self) -> &str {
        match self {
            Stage::Map(f) | Stage::Filter(f) => f,
        }
    }
}

/// 拆开管道链：`a |> map(f) |> map(g)` → (源表达式, [Map(f), Map(g)])
///
/// 返回 None 表示不是可融合的数组管道（普通标量管道走原来的路）。
fn parse_chain(e: &Expr) -> Option<(Expr, Vec<Stage>)> {
    let mut stages: Vec<Stage> = Vec::new();
    let mut cur = e;

    loop {
        let (l, r) = match cur {
            Expr::Binary(l, BinOp::Pipe, r) => (l.as_ref(), r.as_ref()),
            _ => break,
        };
        let (callee, args) = match r {
            Expr::Call(c, a) => (c.as_ref(), a),
            _ => return None,
        };
        let fname = match callee {
            Expr::Ident(f) => f.as_str(),
            _ => return None,
        };
        let stage = match (fname, args.len()) {
            ("map", 1) => Stage::Map(fn_name(&args[0])?),
            ("filter", 1) => Stage::Filter(fn_name(&args[0])?),
            // reduce 收尾 / 参数不对 → 不是融合形状
            _ => return None,
        };
        stages.push(stage);
        cur = l;
    }

    if stages.is_empty() {
        return None;
    }
    // 源必须是能算出长度的东西
    match cur {
        Expr::Ident(_) | Expr::List(_) | Expr::Call(..) | Expr::Slice(..) => {}
        _ => return None,
    }
    stages.reverse();
    Some((cur.clone(), stages))
}

/// 实参应该是一个裸函数名（`map(dbl)` / `map(dbl())` 都接受）
fn fn_name(e: &Expr) -> Option<String> {
    match e {
        Expr::Ident(n) => Some(n.clone()),
        Expr::Call(c, args) if args.is_empty() => fn_name(c),
        _ => None,
    }
}

impl Codegen {
    /// 临时变量序号。同作用域里出现两次的临时名必须带序号，
    /// 否则 C 报 redeclaration。
    pub(crate) fn next_tmp_id(&mut self) -> u32 {
        self.tmp_counter += 1;
        self.tmp_counter
    }

    /// 把「数组来源」变成能在 C 里反复取值的引用。
    ///
    /// 变量 / 切片 / 函数调用：直接就是可用的 C 表达式。
    /// 数组字面量 `[1, 2, 3]`：得先提到具名临时变量，否则 `{}[i]`
    /// 不是合法的左值。返回 `(前置声明行, 变量名)`。
    ///
    /// 空列表推不出元素类型，返回 None —— 交给 checker 报错。
    fn arr_ref(&mut self, arr: &Expr, elem_ty: &str) -> Option<(Vec<String>, String)> {
        match arr {
            Expr::List(items) if items.is_empty() => None,
            Expr::List(items) => {
                let vals: Vec<String> =
                    items.iter().map(|it| self.gen_expr(it)).collect();
                let uid = self.next_tmp_id();
                Some((
                    vec![format!(
                        "{} _lsrc{}[{}] = {{{}}};",
                        elem_ty,
                        uid,
                        items.len(),
                        vals.join(", ")
                    )],
                    format!("_lsrc{}", uid),
                ))
            }
            _ => Some((Vec::new(), self.gen_expr(arr))),
        }
    }

    /// 融合链条能否成立：每段的函数签名都要接得上。
    ///
    /// 返回 `(源元素类型, 输出元素类型)`。对不上返回 None，让调用方走
    /// 普通路（由 checker 报类型错误），而不是静默生成一段错的 C。
    ///
    /// 只读 &self —— `infer_type` 也是 &self，类型推断阶段要能调。
    fn type_plan(&self, src_elem: &str, stages: &[Stage]) -> Option<String> {
        let mut cur_ty = src_elem.to_string();
        for st in stages {
            let f = st.func();
            let in_ty = self.fn_param_types.get(f).and_then(|v| v.first().cloned());
            let out_ty = self.fn_ret_types.get(f).cloned();
            let (in_ty, out_ty) = match (in_ty, out_ty) {
                (Some(i), Some(o)) => (i, o),
                _ => return None,
            };
            // 输入类型必须接得上（filter 的谓词返回 bool，单独在下面判）
            if in_ty != cur_ty {
                return None;
            }
            if matches!(st, Stage::Filter(_)) {
                if out_ty != "bool" {
                    return None;
                }
                // filter 不改变元素类型
            } else {
                cur_ty = out_ty;
            }
        }
        Some(cur_ty)
    }

    /// 融合链条的元素类型（给 `let` 判断 / 类型推断用）
    pub(crate) fn fused_pipe_elem_ty(&self, e: &Expr) -> Option<String> {
        let (src, stages) = parse_chain(e)?;
        let src_ty = self.infer_type(&src);
        let src_elem = src_ty.strip_suffix('*').unwrap_or(&src_ty);
        self.type_plan(src_elem, &stages)
    }

    /// `let r = xs |> map(f) |> map(g)` → 分配 + 单循环 + 收尾。
    ///
    /// 返回 None 表示不是可融合的数组管道，走普通 let。
    pub(crate) fn gen_fused_pipe_let(
        &mut self,
        name: &str,
        init: &Expr,
        _fields: Option<&[String]>,
    ) -> Option<Vec<String>> {
        let (src, stages) = parse_chain(init)?;
        let src_len = self.array_len_of_mut(&src)?;
        let src_ty = self.infer_type(&src);
        let in_elem = src_ty.strip_suffix('*').unwrap_or(&src_ty).to_string();
        // 输出元素类型 = 链条末端类型（map 会改类型，filter 不会）
        let out_elem = self.type_plan(&in_elem, &stages)?;

        // 数组字面量不能当 C 表达式用（`{}[_pi]` 不是合法左值），先提到临时变量
        let (prelude, src_c) = self.arr_ref(&src, &in_elem)?;

        // 链里全是 map 时长度就是源长度，不必靠 _pk 累加（编译器更容易向量化）
        let all_map = stages.iter().all(|s| matches!(s, Stage::Map(_)));

        // 临时变量要按语句编号取名：同一作用域里两条 filter 管道
        // 都会声明 _pk，C 里直接 redeclaration 报错。
        let uid = self.next_tmp_id();

        // 循环体：所有中间值都落在栈上单个标量 _pc 里
        let vectorizable = stages.iter().all(|st| self.is_vectorizable_fn(st.func()));
        let mut body = String::new();
        if vectorizable {
            body.push_str(
                "#if defined(__clang__)\n\
                 #pragma clang loop vectorize(enable) interleave(enable)\n\
                 #endif\n",
            );
        }
        body.push_str(&format!(
            "for (int64_t _pi{} = 0; _pi{} < {}; _pi{}++) {{\n",
            uid, uid, src_len, uid
        ));
        body.push_str(&format!(
            "{} _pc{} = {}[_pi{}];\n",
            in_elem, uid, src_c, uid
        ));
        let mut has_filter = false;
        for st in &stages {
            match st {
                Stage::Map(f) => {
                    body.push_str(&format!("_pc{} = {}(_pc{});\n", uid, f, uid));
                }
                Stage::Filter(_) => {
                    // filter 只做「跳过」，不写输出。值留在 _pc 里继续往下传，
                    // 否则 filter 后面的 map 就没地方落了。
                    has_filter = true;
                    body.push_str(&format!(
                        "if (!{}(_pc{})) continue;\n",
                        st.func(),
                        uid
                    ));
                }
            }
        }
        // 收尾：一次写出。全 map 链用 _pi（长度已知），含 filter 用 _pk 紧凑写。
        if has_filter {
            body.push_str(&format!("{}[_pk{}++] = _pc{};\n", name, uid, uid));
        } else {
            body.push_str(&format!("{}[_pi{}] = _pc{};\n", name, uid, uid));
        }
        body.push_str("}");

        let mut lines = prelude;
        lines.push(format!(
            "{}* {} = ({}*)malloc(({} > 0 ? {} : 1) * sizeof({}));",
            out_elem, name, out_elem, src_len, src_len, out_elem
        ));
        if all_map {
            lines.push(format!("int64_t {}_len = {};", name, src_len));
        } else {
            lines.push(format!("int64_t {}_len = 0;", name));
            lines.push(format!("int64_t _pk{} = 0;", uid));
        }
        lines.push(body);
        if !all_map {
            lines.push(format!("{}_len = _pk{};", name, uid));
            // filter 的输出按输入长度分配，选择性很强时会浪费几个数量级。
            // 命中率不到一半才收缩（realloc 也不是免费的）。
            lines.push(format!(
                "if ({}_len * 2 < ({})) {{ {}* _sh{} = ({}*)realloc({}, (size_t)({}_len > 0 ? {}_len : 1) * sizeof({})); if (_sh{} != NULL) {} = _sh{}; }}",
                name, src_len, out_elem, uid, out_elem, name, name, name, out_elem, uid, name, uid
            ));
        }

        // 登记成普通数组，后面 r.len() / for v in r 都能用
        self.array_len_exprs.insert(name.to_string(), format!("{}_len", name));
        self.array_elem_types.insert(name.to_string(), out_elem.clone());
        self.var_types.insert(name.to_string(), format!("{}*", out_elem));
        Some(lines)
    }

    /// `let r = map(xs, f)` / `let r = filter(xs, p)`：拆成多行。
    ///
    /// map/filter 产出两个值（指针 + 长度），所以不能在表达式位置内联
    /// —— `|>` 管道存在的意义就是这个。只有 `let` 和管道两条路。
    pub(crate) fn gen_hof_let(
        &mut self,
        name: &str,
        init: &Expr,
    ) -> Option<Vec<String>> {
        let (callee, args) = match init {
            Expr::Call(c, a) => (c.as_ref(), a),
            _ => return None,
        };
        let kind = match callee {
            Expr::Ident(f) if f == "map" || f == "filter" => f.as_str(),
            _ => return None,
        };
        if args.len() != 2 {
            return None;
        }
        let f = fn_name(&args[1])?;
        let arr = &args[0];
        let len = self.array_len_of_mut(arr)?;
        let src_ty = self.infer_type(arr);
        let elem_ty = src_ty.strip_suffix('*').unwrap_or(&src_ty).to_string();
        let in_ty = self.fn_param_types.get(&f).and_then(|v| v.first().cloned())?;
        let out_ty = self.fn_ret_types.get(&f).cloned()?;
        if in_ty != elem_ty {
            return None;
        }
        if kind == "filter" {
            if out_ty != "bool" {
                return None;
            }
        }
        let out_elem = if kind == "map" { out_ty.clone() } else { elem_ty.clone() };

        // 数组字面量不能当 C 表达式用，先提到具名临时变量
        let (prelude, arr_c) = self.arr_ref(arr, &elem_ty)?;
        let mut lines = prelude;
        lines.push(format!(
            "{}* {} = ({}*)malloc(({} ? {} : 1) * sizeof({}));",
            out_elem, name, out_elem, len, len, out_elem
        ));
        match kind {
            // 长度恒等于源长度，编译器能直接向量化
            "map" => {
                let uid = self.next_tmp_id();
                lines.push(format!("int64_t {}_len = {};", name, len));
                if self.is_vectorizable_fn(&f) {
                    lines.push(format!(
                        "#if defined(__clang__)\n#pragma clang loop vectorize(enable) interleave(enable)\n#endif"
                    ));
                }
                lines.push(format!(
                    "for (int64_t _pi{} = 0; _pi{} < {}; _pi{}++) {{ {}[_pi{}] = {}({}[_pi{}]); }}",
                    uid, uid, len, uid, name, uid, f, arr_c, uid
                ));
            }
            // 谓词为真的前移写入（原地紧凑化，不用第二个数组）
            _ => {
                let uid = self.next_tmp_id();
                lines.push(format!("int64_t {}_len = 0;", name));
                lines.push(format!("int64_t _pk{} = 0;", uid));
                lines.push(format!(
                    "for (int64_t _pi{} = 0; _pi{} < {}; _pi{}++) {{ if ({}({}[_pi{}])) {{ {}[_pk{}++] = {}[_pi{}]; }} }}",
                    uid, uid, len, uid, f, arr_c, uid, name, uid, arr_c, uid
                ));
                lines.push(format!("{}_len = _pk{};", name, uid));
                lines.push(format!(
                    "if ({}_len * 2 < ({})) {{ {}* _sh{} = ({}*)realloc({}, (size_t)({}_len > 0 ? {}_len : 1) * sizeof({})); if (_sh{} != NULL) {} = _sh{}; }}",
                    name, len, out_elem, uid, out_elem, name, name, name, out_elem, uid, name, uid
                ));
            }
        }

        self.array_len_exprs.insert(name.to_string(), format!("{}_len", name));
        self.array_elem_types.insert(name.to_string(), out_elem.clone());
        self.var_types.insert(name.to_string(), format!("{}*", out_elem));
        Some(lines)
    }

    /// `reduce(xs, f, init)` → 单趟归约，返回标量，可以内联在表达式位置
    pub(crate) fn gen_reduce_call(&mut self, init: &Expr) -> Option<String> {
        let (callee, args) = match init {
            Expr::Call(c, a) => (c.as_ref(), a),
            _ => return None,
        };
        if !matches!(callee, Expr::Ident(f) if f == "reduce") {
            return None;
        }
        if args.len() != 3 {
            return None;
        }
        let f = fn_name(&args[1])?;
        let arr = &args[0];
        let arr_c = self.gen_expr(arr);
        let len = self.array_len_of_mut(arr)?;
        let src_ty = self.infer_type(arr);
        let elem_ty = src_ty.strip_suffix('*').unwrap_or(&src_ty).to_string();
        let params = self.fn_param_types.get(&f)?.clone();
        if params.len() < 2 || params[1] != elem_ty {
            return None;
        }
        let acc_ty = params[0].clone();
        let init_c = self.gen_expr(&args[2]);
        // 一句里可能有两个 reduce（say("...", reduce(a,f,0), reduce(b,g,0))），
        // 临时名必须唯一
        let uid = self.next_tmp_id();
        Some(format!(
            "({{ {acc_ty} _acc{uid} = ({acc_ty})({init_c}); \
for (int64_t _pi{uid} = 0; _pi{uid} < {len}; _pi{uid}++) {{ \
  _acc{uid} = {f}(_acc{uid}, {arr_c}[_pi{uid}]); \
}} \
_acc{uid}; }})",
        ))
    }
}