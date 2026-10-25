use super::Codegen;
use super::expr::bare_parens;
use crate::ast::*;

// par for 的 reduction：sum += a[i] 这类"每轮往同一个变量上累加"的写法。
// 没有 reduction 时多线程写同一个变量是数据竞争，结果随线程数变；
// 有了 reduction 每个线程各自累加，最后合并，和 OpenMP 的 reduction(+:sum)
// 语义一致，这才是 par for 求和能用的前提。
struct Reduction {
    var: String,
    ty: String,
    op: BinOp,
}

impl Reduction {
    fn local(&self) -> String {
        format!("_r_{}", self.var)
    }
    fn field(&self) -> String {
        format!("_red_{}", self.var)
    }
    fn init(&self) -> &'static str {
        match self.op {
            BinOp::Mul => "1",
            _ => "0",
        }
    }
    // 线程内累加：减法也先按加法收集。
    // 私有累加器里存的是"这一块贡献的量和"，符号在合并时统一处理 ——
    // 如果累加时就用 -=，合并时又 -=，负号会被减两次，结果变正。
    // （用隔离探针验证过：累加 -= + 合并 -= 得到 +55，正确值是 -55。）
    fn acc_op(&self) -> &'static str {
        match self.op {
            BinOp::Mul => "*=",
            _ => "+=",
        }
    }
    fn merge_op(&self) -> &'static str {
        match self.op {
            BinOp::Add => "+=",
            BinOp::Sub => "-=",
            BinOp::Mul => "*=",
            _ => "+=",
        }
    }
}

impl Codegen {
    pub(crate) fn gen_par_for(&mut self, var: &str, iter: &Expr, body: &Block) {
        match iter {
            Expr::Range(start, end, inclusive) => {
                self.gen_par_for_range(var, start, end, *inclusive, body);
            }
            // 不能静默跳过：那会让循环一次都不执行，结果错得悄无声息
            _ => {
                self.push_line(
                    "#error \"Lume: par for 只支持区间迭代器（for i in 0..n）\""
                );
            }
        }
    }

    fn gen_par_for_range(
        &mut self,
        var: &str,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        body: &Block,
    ) {
        let start_s = self.gen_expr(start);
        let end_s = self.gen_expr(end);

        let used_arrays = self.collect_used_arrays(body);
        let reductions = self.detect_reductions(body, var);

        // worker 被提升到文件顶层，看不见函数的局部变量。
        // 循环体里引用的外层标量只有两条路：
        //   只读  → 塞进 args 结构体按值传进去（下面处理）
        //   被写  → 多线程写同一个变量是数据竞争，直接报错
        // 以前这里什么都不做，生成的 C 引用未声明符号，编译期才炸，
        // 而且报错信息指向 worker 而不是用户的 Lume 代码。
        let mut scalars: Vec<(String, String)> = Vec::new();
        let mut written: Vec<String> = Vec::new();
        {
            let mut ids: Vec<String> = Vec::new();
            Self::collect_idents_in_block(body, &mut ids);
            ids.sort();
            ids.dedup();
            for n in &ids {
                if n == var { continue; }
                if used_arrays.iter().any(|a| a == n) { continue; }
                if reductions.iter().any(|r| &r.var == n) { continue; }
                // 不在 var_types 里说明是函数名/内建，不是外层局部变量
                if !self.var_types.contains_key(n) { continue; }
                if Self::is_written_in_block(body, n) {
                    written.push(n.clone());
                } else {
                    let ty = self.var_types.get(n).cloned().unwrap();
                    scalars.push((n.clone(), ty));
                }
            }
        }
        if !written.is_empty() {
            self.push_line(&format!(
                "#error \"Lume: par for 循环体写了外层变量 {}（只有归约变量能跨线程累加）\"",
                written.join(", ")
            ));
            return;
        }
        let saved_scalars = std::mem::replace(
            &mut self.par_scalars,
            scalars.iter().map(|(n, _)| n.clone()).collect(),
        );

        let id = self.par_counter;
        self.par_counter += 1;
        let args_struct = format!("_par_args_{}", id);

        // worker 定义在文件顶层，不嵌在函数里。
        // 嵌套函数是 GCC 扩展，clang / MSVC 都不认；而且提到外面之后
        // 它就是个普通静态函数，clang 能正常内联和优化。
        // 它不捕获任何东西，所有数据都通过 _p 传进去。
        let saved_out = std::mem::take(&mut self.output);
        let saved_indent = self.indent;
        self.indent = 0;

        // args 结构体
        self.push_line("typedef struct {");
        self.indent += 1;
        self.push_line("int64_t start;");
        self.push_line("int64_t end;");
        for arr in &used_arrays {
            // Vec 自带 len，不需要伴生长度参数
            if self.vec_elems.contains_key(arr) {
                self.push_line(&format!("LumeVec* {};", arr));
                continue;
            }
            let elem_ty = self.array_elem_types.get(arr)
                .cloned().unwrap_or_else(|| "int64_t".to_string());
            self.push_line(&format!("{}* {};", elem_ty, arr));
            // 数组长度一起传进 worker，循环体里 a.len 才用得了
            self.push_line(&format!("int64_t {}_len;", arr));
        }
        for r in &reductions {
            self.push_line(&format!("{} {};", r.ty, r.field()));
        }
        for (name, ty) in &scalars {
            self.push_line(&format!("{} {};", ty, name));
        }
        self.indent -= 1;
        self.push_line(&format!("}} {};", args_struct));
        // 结构体必须在调用点（方法/函数体）之前定义，先单独收走
        let struct_text = std::mem::take(&mut self.output);
        self.par_structs.push_str(&struct_text);

        // worker 函数（跨平台宏）
        self.push_line(&format!(
            "static LUME_WORKER_RET _par_worker_{}(void* arg) {{",
            id
        ));
        self.indent += 1;
        self.push_line(&format!("{}* _p = ({}*)arg;", args_struct, args_struct));

        // 数组指针拷成局部变量。
        // 不能加 restrict：调用方完全可能把同一个数组传进多个参数，
        // 或者传的是切片。clang 在需要时会自己做运行时别名检查。
        for arr in &used_arrays {
            if self.vec_elems.contains_key(arr) {
                self.push_line(&format!("LumeVec* {} = _p->{};", arr, arr));
                continue;
            }
            let elem_ty = self.array_elem_types.get(arr)
                .cloned().unwrap_or_else(|| "int64_t".to_string());
            self.push_line(&format!(
                "{}* {} = _p->{};",
                elem_ty, arr, arr
            ));
        }
        // 每个线程一份私有累加器，避免竞争
        for r in &reductions {
            self.push_line(&format!("{} {} = {};", r.ty, r.local(), r.init()));
        }

        // 上界统一按排他处理：0..N 和 0..=N 都化成 [start, stop)，
        // 这样切块不会在边界上重复执行元素。
        // worker 里 `let` 声明的名字要临时登记进 var_types（类型推断要用）。
        // 必须在下面的 body_has_unsafe_call **之前**登记好：它靠 is_str_expr
        // 判断循环体里有没有 strcmp / lume_str_concat，看不到类型就会误发
        // vectorize(enable)，clang 只能回一条「loop not vectorized」。
        // 生成结束统一清掉，不污染外层函数。
        let vars_before: Vec<String> = self.var_types.keys().cloned().collect();
        self.predeclare_par_lets(body);

        // start/end 必须先拷成 const 局部变量再进循环条件：
        // _p 是指向堆上 args 的指针，clang 无法证明循环体内没有别的线程
        // 或代码改写 *_p，于是不敢把 `i < _p->end` 提出去，向量化直接报废。
        // 拷成 const 局部量之后，这个别名可能性就消失了。
        self.push_line("const int64_t _par_lo = _p->start;");
        self.push_line("const int64_t _par_hi = _p->end;");
        // 循环体里有 say / 未证明纯的函数 → 发 vectorize(enable) 只会
        // 让 clang 报 -Wpass-failed，没有收益。
        if !self.body_has_unsafe_call(body) {
            self.push_line("#if defined(__clang__)");
            self.push_line("#pragma clang loop vectorize(enable) interleave(enable)");
            self.push_line("#endif");
        }
        self.push_line(&format!(
            "for (int64_t {} = _par_lo; {} < _par_hi; {}++) {{",
            var, var, var
        ));
        self.indent += 1;

        // 和普通 for 一样：起点是非负字面量时，循环变量一定 >= 0。
        // 不登记的话循环体里的 i % 2^n 会走 lume_checked_mod_i64，
        // 登记后能直接发 (i & (2^n-1))。
        let non_neg = matches!(start, Expr::Int(n) if *n >= 0);
        let was_non_neg = self.non_neg_vars.contains(var);
        if non_neg {
            self.non_neg_vars.insert(var.to_string());
        }

        // worker 里数组的长度在 _p->{name}_len，不在外层的 {name}_len。
        // 循环体调用收数组的函数时，gen_call_args 会来查这张表补伴随参数，
        // 不临时改映射就会生成 `f(a, a_len)` —— worker 里 a_len 不存在。
        let saved_lens: Vec<(String, Option<String>)> = used_arrays
            .iter()
            .map(|a| {
                let old = self
                    .array_len_exprs
                    .insert(a.clone(), format!("_p->{}_len", a));
                (a.clone(), old)
            })
            .collect();

        for stmt in &body.stmts {
            self.gen_par_body_stmt(stmt, &used_arrays, &reductions);
        }

        // 清掉 worker 内部 let 登记进 var_types 的名字。
        // worker 是提到文件顶层的独立函数，它的局部变量不该被外层函数看见；
        // 留着会让同名变量在别处被推断成 worker 里的类型。
        for k in self.var_types.keys().cloned().collect::<Vec<_>>() {
            if !vars_before.contains(&k) {
                self.var_types.remove(&k);
            }
        }

        for (a, old) in saved_lens {
            match old {
                Some(v) => {
                    self.array_len_exprs.insert(a, v);
                }
                None => {
                    self.array_len_exprs.remove(&a);
                }
            }
        }

        if non_neg && !was_non_neg {
            self.non_neg_vars.remove(var);
        }

        self.indent -= 1;
        self.push_line("}");

        for r in &reductions {
            self.push_line(&format!("_p->{} = {};", r.field(), r.local()));
        }
        self.push_line("LUME_WORKER_RETURN;");
        self.indent -= 1;
        self.push_line("}");
        self.output.push('\n');
        self.par_worker_decls.push_str(&format!(
            "static LUME_WORKER_RET _par_worker_{}(void* arg);\n",
            id
        ));

        // 把 worker 收进顶层缓冲区，恢复原来的输出位置
        let hoisted = std::mem::replace(&mut self.output, saved_out);
        self.indent = saved_indent;
        self.par_workers.push_str(&hoisted);
        self.par_scalars = saved_scalars;

        // 调用处（跨平台宏）
        self.push_line("{");
        self.indent += 1;
        self.push_line(&format!("int64_t _total_start = {};", start_s));
        self.push_line(&format!(
            "int64_t _total_stop = {}{};",
            end_s,
            if inclusive { " + 1" } else { "" }
        ));
        self.push_line("int64_t _total = _total_stop - _total_start;");
        self.push_line("if (_total < 1) _total = 1;");
        // 只提交本线程缓冲，不等 writer 写完：顺序靠 FIFO 保证，
        // 以前这里是全量 flush，par for 会卡在上一批输出上。
        self.push_line("lume_log_flush_thread();");

        // 小循环直接串行跑。
        // 线程池虽然把固定成本从 ~100 µs 降到几 µs，一次广播 + 唤醒
        // 仍然比几十个元素的加法贵。这里不复制循环体，而是直接调用
        // hoisted worker，代码量不涨。
        self.push_line(&format!(
            "if (_total < LUME_PAR_SERIAL_THRESHOLD) {{"
        ));
        self.indent += 1;
        self.push_line(&format!("{} _a;", args_struct));
        self.push_line("_a.start = _total_start;");
        self.push_line("_a.end = _total_stop;");
        for arr in &used_arrays {
            self.push_line(&format!("_a.{} = {};", arr, arr));
            if self.vec_elems.contains_key(arr) {
                continue; // Vec 自带 len
            }
            let len_expr = self.array_len_exprs.get(arr).cloned()
                .unwrap_or_else(|| "0LL".to_string());
            self.push_line(&format!("_a.{}_len = {};", arr, len_expr));
        }
        for r in &reductions {
            self.push_line(&format!("_a.{} = {};", r.field(), r.init()));
        }
        for (name, _) in &scalars {
            self.push_line(&format!("_a.{} = {};", name, name));
        }
        self.push_line(&format!("_par_worker_{}(&_a);", id));
        for r in &reductions {
            self.push_line(&format!(
                "{} {} _a.{};",
                r.var,
                r.merge_op(),
                r.field()
            ));
        }
        self.indent -= 1;
        self.push_line("} else {");
        self.indent += 1;

        self.push_line("int _n_threads = lume_cpu_count();");
        self.push_line("if (_n_threads > 64) _n_threads = 64;");
        // 小循环只叫醒前几个 worker：每个参与线程至少要分到
        // LUME_PAR_CHUNK_ELEMS 个元素，否则唤醒成本比并行收益大。
        self.push_line("int _n_active = (int)(_total / LUME_PAR_CHUNK_ELEMS) + 1;");
        self.push_line("if (_n_active > _n_threads) _n_active = _n_threads;");
        self.push_line("if (_n_active < 1) _n_active = 1;");
        self.push_line(&format!("{} _args[64];", args_struct));
        self.push_line("int64_t _chunk = (_total + _n_threads - 1) / _n_threads;");
        self.push_line("for (int _t = 0; _t < _n_threads; _t++) {");
        self.indent += 1;
        self.push_line("_args[_t].start = _total_start + _t * _chunk;");
        self.push_line("_args[_t].end = _args[_t].start + _chunk;");
        self.push_line("if (_args[_t].end > _total_stop) _args[_t].end = _total_stop;");
        for arr in &used_arrays {
            self.push_line(&format!("_args[_t].{} = {};", arr, arr));
            if self.vec_elems.contains_key(arr) {
                continue; // Vec 自带 len
            }
            let len_expr = self.array_len_exprs.get(arr).cloned()
                .unwrap_or_else(|| "0LL".to_string());
            self.push_line(&format!("_args[_t].{}_len = {};", arr, len_expr));
        }
        for r in &reductions {
            self.push_line(&format!(
                "_args[_t].{} = {};",
                r.field(),
                r.init()
            ));
        }
        for (name, _) in &scalars {
            self.push_line(&format!("_args[_t].{} = {};", name, name));
        }
        self.indent -= 1;
        self.push_line("}");
        // 线程池：worker 是常驻的，这里只做一次代际广播。
        // 主线程也参与抢任务（池里是 N-1 个 worker）。
        self.push_line(&format!(
            "lume_par_run(_par_worker_{}, _args, sizeof(_args[0]), _n_threads, _n_active);",
            id
        ));
        // 合并各线程的私有累加器
        if !reductions.is_empty() {
            self.push_line("for (int _t = 0; _t < _n_threads; _t++) {");
            self.indent += 1;
            for r in &reductions {
                self.push_line(&format!(
                    "{} {} _args[_t].{};",
                    r.var,
                    r.merge_op(),
                    r.field()
                ));
            }
            self.indent -= 1;
            self.push_line("}");
        }
        self.indent -= 1;
        self.push_line("}");
        self.indent -= 1;
        self.push_line("}");
    }

    // ── reduction 识别 ──

    fn detect_reductions(&self, body: &Block, loop_var: &str) -> Vec<Reduction> {
        let mut stmts: Vec<&Stmt> = Vec::new();
        Self::flatten_stmts(body, &mut stmts);

        // 候选变量：v = v OP rhs，且 v 不是循环变量
        let mut cands: Vec<String> = Vec::new();
        for s in &stmts {
            if let Some((v, _)) = Self::as_reduction(s) {
                if v != loop_var && !cands.contains(v) {
                    cands.push(v.clone());
                }
            }
        }

        let mut out = Vec::new();
        for v in cands {
            // 循环体里重新声明了同名变量 → 不是外层累加
            if stmts.iter().any(|s| matches!(s, Stmt::Let { name, .. } if name == &v)) {
                continue;
            }
            // 必须是"只在这个累加式里出现"：
            // 每处累加贡献 2 次引用（赋值目标 + 右值左侧），多一次就说明
            // 循环体还读了它，归约后语义会变，宁可不优化
            let n = stmts.iter()
                .filter(|s| matches!(Self::as_reduction(s), Some((x, _)) if x == &v))
                .count();
            if Self::count_ident_in_block(body, &v) != 2 * n {
                continue;
            }
            let ty = match self.var_types.get(&v) {
                Some(t) => t.clone(),
                None => continue,
            };
            // 同一个变量上混用 + / - / * 时，合并顺序会影响浮点结果，
            // 而且私有累加器的初值该取 0 还是 1 说不清。直接放弃。
            let mut ops = stmts.iter().filter_map(|s| match Self::as_reduction(s) {
                Some((x, o)) if x == &v => Some(o),
                _ => None,
            });
            let first = ops.next().unwrap();
            if ops.any(|o| o != first) {
                continue;
            }
            out.push(Reduction { var: v, ty, op: first });
        }
        out
    }

    // 识别 `v = v OP rhs`，OP ∈ {+, -, *}
    fn as_reduction(s: &Stmt) -> Option<(&String, BinOp)> {
        let Stmt::Assign { target, value } = s else { return None };
        let Expr::Ident(v) = target else { return None };
        let Expr::Binary(lhs, op, _) = value else { return None };
        if !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) {
            return None;
        }
        match lhs.as_ref() {
            Expr::Ident(v2) if v2 == v => Some((v, *op)),
            _ => None,
        }
    }

    fn flatten_stmts<'a>(b: &'a Block, out: &mut Vec<&'a Stmt>) {
        for s in &b.stmts {
            out.push(s);
            match s {
                Stmt::If { then, els, .. } => {
                    Self::flatten_stmts(then, out);
                    if let Some(e) = els {
                        match e.as_ref() {
                            Stmt::Expr(Expr::If(_, block, _)) => {
                                Self::flatten_stmts(block, out)
                            }
                            other => out.push(other),
                        }
                    }
                }
                Stmt::While { body, .. }
                | Stmt::For { body, .. } => Self::flatten_stmts(body, out),
                // 嵌套 par for 是另一个作用域（而且本来就要报错），
                // 不能把它内部的累加算成外层循环的归约候选
                Stmt::ParFor { .. } => {}
                _ => {}
            }
        }
    }

    fn count_ident_in_block(b: &Block, name: &str) -> usize {
        let mut n = 0;
        for s in &b.stmts {
            Self::count_ident_in_stmt(s, name, &mut n);
        }
        n
    }

    fn count_ident_in_stmt(s: &Stmt, name: &str, n: &mut usize) {
        match s {
            Stmt::Let { init, .. } => Self::count_ident_in_expr(init, name, n),
            Stmt::Assign { target, value } => {
                Self::count_ident_in_expr(target, name, n);
                Self::count_ident_in_expr(value, name, n);
            }
            Stmt::Expr(e) => Self::count_ident_in_expr(e, name, n),
            Stmt::Return(Some(e)) => Self::count_ident_in_expr(e, name, n),
            Stmt::If { cond, then, els } => {
                Self::count_ident_in_expr(cond, name, n);
                *n += Self::count_ident_in_block(then, name);
                if let Some(e) = els {
                    Self::count_ident_in_stmt(e, name, n);
                }
            }
            Stmt::While { cond, body } => {
                Self::count_ident_in_expr(cond, name, n);
                *n += Self::count_ident_in_block(body, name);
            }
            Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
                Self::count_ident_in_expr(iter, name, n);
                *n += Self::count_ident_in_block(body, name);
            }
            _ => {}
        }
    }

    fn count_ident_in_expr(e: &Expr, name: &str, n: &mut usize) {
        match e {
            Expr::Ident(v) => {
                if v == name {
                    *n += 1;
                }
            }
            Expr::Binary(l, _, r) => {
                Self::count_ident_in_expr(l, name, n);
                Self::count_ident_in_expr(r, name, n);
            }
            Expr::Unary(_, e) => Self::count_ident_in_expr(e, name, n),
            Expr::Index(o, i) => {
                Self::count_ident_in_expr(o, name, n);
                Self::count_ident_in_expr(i, name, n);
            }
            Expr::Range(a, b, _) => {
                Self::count_ident_in_expr(a, name, n);
                Self::count_ident_in_expr(b, name, n);
            }
            Expr::Call(c, args) => {
                Self::count_ident_in_expr(c, name, n);
                for a in args {
                    Self::count_ident_in_expr(a, name, n);
                }
            }
            Expr::Field(o, _) => Self::count_ident_in_expr(o, name, n),
            Expr::If(cond, then, els) => {
                Self::count_ident_in_expr(cond, name, n);
                *n += Self::count_ident_in_block(then, name);
                if let Some(b) = els {
                    *n += Self::count_ident_in_block(b, name);
                }
            }
            Expr::List(items) => {
                for i in items {
                    Self::count_ident_in_expr(i, name, n);
                }
            }
            _ => {}
        }
    }

    // ── 循环体生成 ──

    /// 预扫描 worker 体里的 `let`，把「名字 → C 类型」登记进 var_types。
    ///
    /// 平时 var_types 是边生成边登记的，但 gen_par_for_range 要在生成
    /// **之前**判断「循环体里有没有不可向量化的东西」，那个判断走
    /// is_str_expr → infer_type → var_types。所以先扫一遍。
    fn predeclare_par_lets(&mut self, body: &Block) {
        for s in &body.stmts {
            self.predeclare_par_let_stmt(s);
        }
    }

    fn predeclare_par_let_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, ty, init } => {
                let c_ty = match ty {
                    Some(t) => self.gen_type(t),
                    None => self.infer_type(init),
                };
                self.var_types.insert(name.clone(), c_ty);
            }
            Stmt::If { then, els, .. } => {
                self.predeclare_par_lets(then);
                if let Some(e) = els {
                    self.predeclare_par_let_stmt(e);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ParFor { body, .. } => {
                self.predeclare_par_lets(body);
            }
            Stmt::Expr(Expr::If(_, t, e)) => {
                self.predeclare_par_lets(t);
                if let Some(b) = e {
                    self.predeclare_par_lets(b);
                }
            }
            _ => {}
        }
    }

    fn gen_par_body_stmt(
        &mut self,
        stmt: &Stmt,
        arrays: &[String],
        reds: &[Reduction],
    ) {
        match stmt {
            Stmt::Assign { target, value } => {
                // reduction：写私有累加器，不碰共享变量
                if let Expr::Ident(v) = target {
                    if let Some(r) = reds.iter().find(|r| &r.var == v) {
                        if let Expr::Binary(_, _, rhs) = value {
                            let rs = self.gen_par_expr(rhs, arrays);
                            self.push_line(&format!(
                                "{} {} {};",
                                r.local(),
                                r.acc_op(),
                                rs
                            ));
                            return;
                        }
                    }
                }
                let t = self.gen_par_expr(target, arrays);
                let v = self.gen_par_expr(value, arrays);
                self.push_line(&format!("{} = {};", t, v));
            }
            Stmt::Expr(e) => {
                let s = self.gen_par_expr(e, arrays);
                self.push_line(&format!("{};", s));
            }
            Stmt::Let { name, ty, init } => {
                let c_ty = match ty {
                    Some(t) => self.gen_type(t),
                    None => self.infer_type(init),
                };
                let v = self.gen_par_expr(init, arrays);
                self.push_line(&format!("{} {} = {};", c_ty, name, v));
                // 必须登记进 var_types，否则 worker 里后面引用这个变量时
                // infer_type 查不到、退化成 int64_t —— `tag == "even"`
                // 于是变成 C 的指针比较（clang 只给个 warning）。
                // 生成结束由 gen_par_for_range 统一清掉，不污染外层函数。
                self.var_types.insert(name.clone(), c_ty);
            }
            Stmt::If { cond, then, els } => {
                let c = self.gen_par_expr(cond, arrays);
                self.push_line(&format!("if ({}) {{", bare_parens(&c)));
                self.indent += 1;
                for s in &then.stmts {
                    self.gen_par_body_stmt(s, arrays, reds);
                }
                self.indent -= 1;
                if let Some(e) = els {
                    self.push_line("} else {");
                    self.indent += 1;
                    match e.as_ref() {
                        Stmt::Expr(Expr::If(cond, block, _))
                            if matches!(cond.as_ref(), Expr::Bool(true)) =>
                        {
                            for stmt in &block.stmts {
                                self.gen_par_body_stmt(stmt, arrays, reds);
                            }
                        }
                        other => self.gen_par_body_stmt(other, arrays, reds),
                    }
                    self.indent -= 1;
                }
                self.push_line("}");
            }
            // 嵌套 par for：worker 里再开线程，线程数会变成核数的平方。
            // 而且 worker 是被提升到文件顶层的，提升逻辑还没处理嵌套，
            // 静默跳过会让循环一次都不执行、结果错得悄无声息 —— 直接报错。
            Stmt::ParFor { .. } => {
                self.push_line(
                    "#error \"Lume: par for 暂不支持嵌套（worker 里不能再 par for）\""
                );
            }
            // scope 靠「记水位 + 之后回退」归还 arena 空间，而 par for 的
            // 多个 worker 是并发从同一个 arena 切的。worker A 回退水位会把
            // worker B 刚分配的字符串一起回收 —— 直接踩坏别人的内存。
            // 这个做不到「局部」，所以只能拒绝。
            Stmt::Scope { .. } => {
                self.push_line(
                    "#error \"Lume: par for 的循环体里不能用 scope（多个 worker 共享 arena，回退水位会回收别人的字符串）\""
                );
            }
            _ => self.push_line("/* par body: 暂不支持 */"),
        }
    }

    fn gen_par_expr(&mut self, expr: &Expr, arrays: &[String]) -> String {
        match expr {
            // 数组已经在 worker 顶部拷成局部指针，直接用名字
            Expr::Ident(name) if arrays.contains(name) => name.clone(),
            // 只读外层标量走 args 结构体（worker 是顶层函数，看不见它们）
            Expr::Ident(name) if self.par_scalars.contains(name) => {
                format!("_p->{}", name)
            }
            Expr::Binary(l, op, r) => {
                let ls = self.gen_par_expr(l, arrays);
                let rs = self.gen_par_expr(r, arrays);
                // 结构体运算符重载（和 gen_expr 共用 gen_op_overload）
                if let Some(c) = self.gen_op_overload(l, *op, r, &ls, &rs) {
                    return c;
                }
                // 字符串拼接近 arena、比较走 strcmp —— 和 gen_expr 共用
                // 同一份判断（以前这里没有，par for 里的 "v" + int_to_str(i)
                // 会生成 C 的指针加法，clang 报错才发现）。
                if let Some(c) = self.gen_str_binop(l, *op, r, &ls, &rs) {
                    return c;
                }
                // 除法/取模：常量除数安全化 + 强度削弱，和 gen_expr 共用
                // gen_divmod。以前这里是手抄的一份，只做了 % 2^n 的位与，
                // 常量除数仍然走 lume_checked_*，par for 里白丢一截性能。
                if matches!(op, BinOp::Div | BinOp::Mod) {
                    let lt = self.infer_type(l);
                    if lt != "double" && lt != "float" {
                        if let Some(c) = self.gen_divmod(&ls, l, *op, r) {
                            return c;
                        }
                        let fname = match op {
                            BinOp::Div => "lume_checked_div_i64",
                            _ => "lume_checked_mod_i64",
                        };
                        return format!("{}({}, {})", fname, ls, rs);
                    }
                }
                let op_str = match op {
                    BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
                    BinOp::Div => "/", BinOp::Mod => "%",
                    BinOp::Eq => "==", BinOp::Ne => "!=",
                    BinOp::Lt => "<", BinOp::Gt => ">",
                    BinOp::Le => "<=", BinOp::Ge => ">=",
                    BinOp::And => "&&", BinOp::Or => "||",
                    BinOp::Pipe => "/* pipe */",
                };
                format!("({} {} {})", ls, op_str, rs)
            }
            Expr::Index(o, i) => {
                // Vec 的下标：和 gen_vec_index 同一套语义，只是接收者要走
                // par 的表达式生成（外层变量可能被收集进 _p 结构体）。
                if self.infer_type(o) == "LumeVec*" {
                    if let Some(elem) = self.inferred_vec_elem(o) {
                        let os = self.gen_par_expr(o, arrays);
                        let is = self.gen_par_expr(i, arrays);
                        let ptr = if self.bounds_check {
                            let label = match o.as_ref() {
                                Expr::Ident(n) => format!("\"{}\"", n),
                                _ => "\"\"".to_string(),
                            };
                            format!("lume_vec_at({}, {}, {})", os, is, label)
                        } else {
                            format!("lume_vec_at_raw({}, {})", os, is)
                        };
                        return format!("(*({}*)({}))", elem, ptr);
                    }
                }
                let os = self.gen_par_expr(o, arrays);
                let is = self.gen_par_expr(i, arrays);
                format!("{}[{}]", os, is)
            }
            Expr::Unary(op, e) => {
                let es = self.gen_par_expr(e, arrays);
                if let Some(c) = self.gen_unop_overload(*op, e, &es) {
                    return c;
                }
                let op_str = match op { UnOp::Neg => "-", UnOp::Not => "!" };
                format!("({}{})", op_str, es)
            }
            Expr::Int(n) => format!("{}LL", n),
            Expr::Float(f) => {
                let s = format!("{}", f);
                if s.contains('.') || s.contains('e') {
                    s
                } else {
                    format!("{}.0", s)
                }
            }
            // a.len → _p->a_len（长度在 args 结构体里）
            Expr::Field(o, name) if name == "len" => {
                if let Expr::Ident(arr) = o.as_ref() {
                    if arrays.contains(arr) {
                        return format!("_p->{}_len", arr);
                    }
                }
                self.gen_expr(expr)
            }
            // Vec 的只读方法。push 之类会改 buffer 的在 checker 就被拒了
            // （多 worker 并发 realloc 是数据竞争），这里只处理读操作。
            Expr::Call(c, args) if matches!(c.as_ref(), Expr::Field(..)) => {
                if let Expr::Field(obj, m) = c.as_ref() {
                    if self.infer_type(obj) == "LumeVec*" {
                        if let Some(elem) = self.inferred_vec_elem(obj) {
                            let v = self.gen_par_expr(obj, arrays);
                            match m.as_str() {
                                "len" | "length" if args.is_empty() => {
                                    return format!("(({})->len)", v)
                                }
                                "is_empty" if args.is_empty() => {
                                    return format!("(({})->len == 0LL)", v)
                                }
                                "pop" | "last" if args.is_empty() => {
                                    return format!(
                                        "(*({}*)(lume_vec_{}({})))",
                                        elem, m, v
                                    )
                                }
                                _ => {}
                            }
                        }
                    }
                }
                self.gen_expr(expr)
            }
            Expr::Ident(name) => name.clone(),
            // par worker 里禁止 free：多个 worker 可能拿到同一个 owned 数组
            Expr::Call(c, _) if matches!(c.as_ref(), Expr::Ident(n) if n == "free") => {
                self.codegen_errors.push(
                    "par for 循环体里不能 free（多个 worker 会重复释放）".to_string()
                );
                "((void)0)".to_string()
            }
            _ => self.gen_expr(expr),
        }
    }

    fn collect_used_arrays(&self, block: &Block) -> Vec<String> {
        let mut result = Vec::new();
        for stmt in &block.stmts {
            self.collect_arrays_in_stmt(stmt, &mut result);
        }
        result.sort();
        result.dedup();
        result
    }

    fn collect_arrays_in_stmt(&self, stmt: &Stmt, out: &mut Vec<String>) {
        match stmt {
            Stmt::Expr(e) => self.collect_arrays_in_expr(e, out),
            Stmt::Assign { target, value } => {
                self.collect_arrays_in_expr(target, out);
                self.collect_arrays_in_expr(value, out);
            }
            Stmt::Let { init, .. } => self.collect_arrays_in_expr(init, out),
            Stmt::If { cond, then, els } => {
                self.collect_arrays_in_expr(cond, out);
                for s in &then.stmts {
                    self.collect_arrays_in_stmt(s, out);
                }
                if let Some(e) = els {
                    self.collect_arrays_in_stmt(e, out);
                }
            }
            Stmt::While { cond, body } => {
                self.collect_arrays_in_expr(cond, out);
                for s in &body.stmts {
                    self.collect_arrays_in_stmt(s, out);
                }
            }
            Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
                self.collect_arrays_in_expr(iter, out);
                for s in &body.stmts {
                    self.collect_arrays_in_stmt(s, out);
                }
            }
            Stmt::Return(Some(e)) => self.collect_arrays_in_expr(e, out),
            _ => {}
        }
    }

    fn collect_arrays_in_expr(&self, expr: &Expr, out: &mut Vec<String>) {
        match expr {
            Expr::Ident(name) => {
                // 字面量数组（编译期知道长度）和数组参数都算；
                // Vec 是句柄，按「引用」传进 worker 就够了（只读；
                // 会改 buffer 的方法 checker 已经拒了）。
                if !out.contains(name)
                    && (self.array_len_exprs.contains_key(name)
                        || self.array_params.contains(name)
                        || self.vec_elems.contains_key(name))
                {
                    out.push(name.clone());
                }
            }
            Expr::Binary(l, _, r) => {
                self.collect_arrays_in_expr(l, out);
                self.collect_arrays_in_expr(r, out);
            }
            Expr::Unary(_, e) => self.collect_arrays_in_expr(e, out),
            Expr::Call(c, args) => {
                self.collect_arrays_in_expr(c, out);
                for a in args {
                    self.collect_arrays_in_expr(a, out);
                }
            }
            Expr::Field(o, _) => self.collect_arrays_in_expr(o, out),
            // 插值里的表达式也算「用到了」。漏了这一支的话，
            // `par for { say("{a.len()}") }` 不会把 a 带进 worker，
            // 生成 C 引用未声明符号。
            Expr::FmtStr(parts) => {
                for p in parts {
                    if let StrPart::Expr(e) = p {
                        self.collect_arrays_in_expr(e, out);
                    }
                }
            }
            Expr::Index(o, i) => {
                self.collect_arrays_in_expr(o, out);
                self.collect_arrays_in_expr(i, out);
            }
            _ => {}
        }
    }

    // ── 外层标量收集 / 写入检测 ──
    // worker 提到文件顶层后就看不见函数局部变量了，这两组 walker
    // 用来决定哪些标量要按值传进去、哪些必须报错。

    fn collect_idents_in_block(b: &Block, out: &mut Vec<String>) {
        for s in &b.stmts {
            Self::collect_idents_in_stmt(s, out);
        }
    }

    fn collect_idents_in_stmt(s: &Stmt, out: &mut Vec<String>) {
        match s {
            Stmt::Let { init, .. } => Self::collect_idents_in_expr(init, out),
            Stmt::Assign { target, value } => {
                Self::collect_idents_in_expr(target, out);
                Self::collect_idents_in_expr(value, out);
            }
            Stmt::Expr(e) => Self::collect_idents_in_expr(e, out),
            Stmt::Return(Some(e)) => Self::collect_idents_in_expr(e, out),
            Stmt::If { cond, then, els } => {
                Self::collect_idents_in_expr(cond, out);
                Self::collect_idents_in_block(then, out);
                if let Some(e) = els {
                    Self::collect_idents_in_stmt(e, out);
                }
            }
            Stmt::While { cond, body } => {
                Self::collect_idents_in_expr(cond, out);
                Self::collect_idents_in_block(body, out);
            }
            Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
                Self::collect_idents_in_expr(iter, out);
                Self::collect_idents_in_block(body, out);
            }
            _ => {}
        }
    }

    fn collect_idents_in_expr(e: &Expr, out: &mut Vec<String>) {
        match e {
            Expr::Ident(n) => out.push(n.clone()),
            Expr::Binary(l, _, r) => {
                Self::collect_idents_in_expr(l, out);
                Self::collect_idents_in_expr(r, out);
            }
            Expr::Unary(_, e) => Self::collect_idents_in_expr(e, out),
            Expr::Index(o, i) => {
                Self::collect_idents_in_expr(o, out);
                Self::collect_idents_in_expr(i, out);
            }
            Expr::Range(a, b, _) => {
                Self::collect_idents_in_expr(a, out);
                Self::collect_idents_in_expr(b, out);
            }
            Expr::Call(c, args) => {
                Self::collect_idents_in_expr(c, out);
                for a in args {
                    Self::collect_idents_in_expr(a, out);
                }
            }
            Expr::Field(o, _) => Self::collect_idents_in_expr(o, out),
            // 插值里的标识符也要收集（否则 `say("{outer}")` 里的 outer
            // 不会被当成外层变量传进 worker）
            Expr::FmtStr(parts) => {
                for p in parts {
                    if let StrPart::Expr(e) = p {
                        Self::collect_idents_in_expr(e, out);
                    }
                }
            }
            Expr::If(cond, then, els) => {
                Self::collect_idents_in_expr(cond, out);
                Self::collect_idents_in_block(then, out);
                if let Some(b) = els {
                    Self::collect_idents_in_block(b, out);
                }
            }
            Expr::List(items) => {
                for i in items {
                    Self::collect_idents_in_expr(i, out);
                }
            }
            _ => {}
        }
    }

    // 变量在循环体里是否被赋值（`x = ...`）。
    // 注意 Stmt::Let 同名算遮蔽，不算写外层变量。
    fn is_written_in_block(b: &Block, name: &str) -> bool {
        b.stmts.iter().any(|s| Self::is_written_in_stmt(s, name))
    }

    fn is_written_in_stmt(s: &Stmt, name: &str) -> bool {
        match s {
            Stmt::Assign { target, .. } => matches!(target, Expr::Ident(n) if n == name),
            Stmt::If { then, els, .. } => {
                Self::is_written_in_block(then, name)
                    || els.as_ref().map(|e| Self::is_written_in_stmt(e, name)).unwrap_or(false)
            }
            Stmt::While { body, .. } => Self::is_written_in_block(body, name),
            Stmt::For { body, .. } | Stmt::ParFor { body, .. } => {
                Self::is_written_in_block(body, name)
            }
            _ => false,
        }
    }
}
