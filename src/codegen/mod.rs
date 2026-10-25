// src/codegen/mod.rs
mod expr;
mod items;
mod ops;
mod parallel;
mod pipe;
mod purity;
mod runtime;
mod say;
mod stmt;
mod types;

use crate::ast::*;
use std::cell::RefCell;
use types::collect_struct_deps;
use std::collections::{HashMap, HashSet};

/// 数组类型判定：`[T]` 解析成 List，`[T; N]` 解析成 Array。
/// 两者在 codegen 里一视同仁：指针 + 一个隐式长度伴随参数。
pub(crate) fn is_array_ty(t: &Option<Type>) -> bool {
    matches!(t, Some(Type::Array(_, _)) | Some(Type::List(_)))
}

pub struct Codegen {
    pub(crate) output: String,
    pub(crate) indent: usize,
    pub(crate) var_types: HashMap<String, String>,
    /// 当前 struct 方法的字段名 → C 类型。方法体里裸写字段名（`prefix`）
    /// 时，`infer_type` 要能查到它是 `const char*` —— 否则
    /// `prefix + suffix` 会退化成 C 的指针加法，`prefix == "x"` 会变成
    /// 比指针，两处都静默算错。局部变量优先级更高（`var_types` 先查），
    /// 所以同名局部变量仍能正常遮蔽字段。
    /// 生命周期由 `begin_fn_scope` 清空，不会漏进下一个函数。
    pub(crate) self_fields: HashMap<String, String>,
    pub(crate) structs: HashMap<String, Vec<(String, String)>>,
    /// 枚举名 → [(变体名, 值)]，按声明顺序。C 侧是 `typedef int64_t <名>;`
    /// 加每个变体一个 `#define`，所以这张表只用于：
    /// ① 认出 `Tag.Number` 里的 `Tag` 是类型名而不是变量；② 发定义。
    pub(crate) enums: HashMap<String, Vec<(String, i64)>>,
    // struct 名 → 类型标签编号。C 没有 RTTI，match 结构体解构靠它判断类型
    pub(crate) struct_tags: HashMap<String, i64>,
    pub(crate) methods: HashMap<(String, String), String>,
    // 数组名 → 该数组长度的 C 表达式。
    //   局部数组字面量 → "5LL"
    //   数组参数       → "a_len"（伴随参数）
    //   函数返回的数组 → "_r_len"
    // 这是「数组跨函数」的核心：长度跟着数组一起走，调用点自动补。
    pub(crate) array_len_exprs: HashMap<String, String>,
    pub(crate) array_elem_types: HashMap<String, String>,
    // ── 动态数组 Vec[T] ──
    // Vec 的 C 类型永远是 `LumeVec*`（元素大小存在句柄里），所以只看
    // var_types 分不出元素类型。取元素要按元素类型强转指针，因此这几张表
    // 把「哪个名字装的是 Vec[T] 的 T」记下来。
    /// 变量名 → 元素 C 类型
    pub(crate) vec_elems: HashMap<String, String>,
    /// (结构体名, 字段名) → 元素 C 类型。`s.toks[i]` / 方法体里的 `toks[i]`
    pub(crate) vec_field_elems: HashMap<(String, String), String>,
    /// 函数名 → 返回的 Vec 的元素 C 类型（`let v = make()` 用）
    pub(crate) fn_vec_elems: HashMap<String, String>,
    /// (结构体名, 方法名) → 返回的 Vec 的元素 C 类型（`let v = obj.parse()`）
    pub(crate) method_vec_elems: HashMap<(String, String), String>,
    /// 当前函数的返回类型是 Vec 时的元素 C 类型。`return []` 要据此
    /// 建一个空 Vec，`return v` 则直接返回句柄。
    pub(crate) cur_ret_vec_elem: Option<String>,
    /// 当前方法所属的结构体名（`gen_method` 设置）。方法体里裸写字段名
    /// 时，取 Vec 元素类型要知道是哪个 struct 的同名字段。
    pub(crate) cur_self_struct: Option<String>,
    /// 当前方法的字段名表。`gen_for` 生成循环体时要靠它把裸字段名映射成
    /// `_self->f`（gen_stmt_with_self 有 fields 参数，gen_for 没有）。
    pub(crate) cur_field_names: Vec<String>,
    /// push 的临时变量序号（见 gen_vec_method）
    pub(crate) vec_counter: usize,
    // 数组/列表参数名（长度编译期未知）
    pub(crate) array_params: HashSet<String>,
    // 已证明为纯函数、可以打 __attribute__((const)) 的函数名
    pub(crate) const_fns: HashSet<String>,
    // 当前函数里「归调用方所有」的数组变量：由函数返回 / map / filter
    // 新建出来的堆数组。只有这些能 free；字面量、参数、切片都不行。
    pub(crate) owned_arrays: HashSet<String>,
    // 返回**新分配**数组的函数（所有 return 都是 [..] / [v; n] / 局部数组）。
    // `let r = f()` 据此把 r 登记成 owned。
    pub(crate) owned_return_fns: HashSet<String>,
    // codegen 阶段的安全错误（目前只有 free 误用）。生成结束后以 #error 形式
    // 插到 C 文件开头，让 clang 带着可读信息失败。
    pub(crate) codegen_errors: Vec<String>,
    // a[i] 的运行期边界检查（默认开）。见 set_bounds_check。
    pub(crate) bounds_check: bool,
    // 会在 par for 里被调用到的 @cache 函数。这些必须用带锁的包装器，
    // 否则多个 worker 并发读写同一个全局哈希表（realloc 时直接崩）。
    pub(crate) thread_safe_cached: HashSet<String>,
    pub(crate) precomputed: HashMap<String, Vec<i64>>,
    pub(crate) generic_fns: HashMap<String, FnDecl>,
    // 普通函数的返回类型（供插值/类型推断用，见 generate 步骤 2.4）
    pub(crate) fn_ret_types: HashMap<String, String>,
    // 函数名 → 每个参数的 C 类型。管道融合校验 f 的入参类型时用
    // （见 codegen/pipe.rs 的 fusion_plan）。
    pub(crate) fn_param_types: HashMap<String, Vec<String>>,
    // 临时变量序号。同一作用域里出现两次的临时名（_pk / _pi / _pc / _acc）
    // 必须带序号，否则 C 报 redeclaration。见 codegen/pipe.rs。
    pub(crate) tmp_counter: u32,
    // 函数名 → 每个参数是不是数组（调用点据此补长度实参）
    pub(crate) fn_array_params: HashMap<String, Vec<bool>>,
    // 函数名 → 参数名表。命名参数调用 f(a: 1) 的重排依据
    // （parser 把这种语法解析成 StructLit，见 gen_named_call）
    pub(crate) fn_param_names: HashMap<String, Vec<String>>,
    // 函数名 → 参数默认值表达式（None = 无默认值）。
    // 默认值在调用处展开（C 语义），与 checker 的 FuncSig::defaults 同源。
    pub(crate) fn_param_defaults: HashMap<String, Vec<Option<Expr>>>,
    // (结构体名, 方法名) → 同上
    pub(crate) method_array_params: HashMap<(String, String), Vec<bool>>,
    // 返回数组的函数：C 签名多一个 int64_t* _ret_len 出参
    pub(crate) array_ret_fns: HashSet<String>,
    pub(crate) monomorphized: HashMap<String, String>,
    pub(crate) pending_monos: Vec<(FnDecl, String, HashMap<String, String>)>,
    pub(crate) par_counter: usize,
    // par for 的 args 结构体（必须早于调用点）和 worker 声明
    pub(crate) par_structs: String,
    pub(crate) par_worker_decls: String,
    // par for 的 worker 定义（放在所有函数原型之后，才能调用 @cache 等）
    pub(crate) par_workers: String,
    // 当前 par worker 里通过 _p-> 访问的只读外层标量（见 parallel.rs）
    pub(crate) par_scalars: Vec<String>,
    // 当前函数是否返回数组（决定 return 要不要 malloc 拷贝）
    pub(crate) cur_ret_is_array: bool,
    // 当前函数的 C 返回类型（供 ? 传播时构造提前返回的 None 字面量）
    pub(crate) cur_ret_c: String,
    // 可空类型 typedef：元素 C 类型 → tag（如 "int64_t" → "lume_opt_0"），反查 elem。
    // 用 RefCell 是因为 opt_tag 在 &self 的 gen_type / infer_type 里惰性分配。
    pub(crate) opt_names: RefCell<HashMap<String, String>>,
    pub(crate) opt_elems: RefCell<HashMap<String, String>>,
    // match 生成 union 时，值的真实类型对应的成员名（如 "Circle"）。
    // None 表示没走 union 路径。字面量/变量绑定臂要通过它访问 _match_val。
    pub(crate) match_val_member: Option<String>,

    pub(crate) non_neg_vars: std::collections::HashSet<String>,
    /// 顶层 `const NAME = <整数字面量>` 的值。
    /// 用途：让 `x % M` 在 codegen 阶段就能看到除数是常量，
    /// 从而走「常量除数无检查」+ 强度削弱，而不是等 clang 去猜。
    /// 只收字面量（含一元负号）—— 复杂常量表达式留给 C 编译器。
    pub(crate) const_ints: HashMap<String, i64>,
    pub(crate) repeat_counter: usize,
    /// 当前已打开的 `scope` 块：(水位变量名, 打开时的 loop_depth)。    /// `break` / `continue` 要按 loop_depth 判断哪些 scope 必须补一次
    /// 水位释放 —— 只释放本层循环里打开的，外层 scope 还在用。
    pub(crate) scope_marks: Vec<(String, usize)>,
    /// 当前嵌套的循环层数。0 = 不在任何循环体里。
    pub(crate) loop_depth: usize,
    pub(crate) scope_counter: usize,
}

impl Codegen {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            var_types: HashMap::new(),
            self_fields: HashMap::new(),
            structs: HashMap::new(),
            struct_tags: HashMap::new(),
            enums: HashMap::new(),
            methods: HashMap::new(),
            array_len_exprs: HashMap::new(),
            array_elem_types: HashMap::new(),
            vec_elems: HashMap::new(),
            vec_field_elems: HashMap::new(),
            fn_vec_elems: HashMap::new(),
            method_vec_elems: HashMap::new(),
            cur_ret_vec_elem: None,
            cur_self_struct: None,
            cur_field_names: Vec::new(),
            vec_counter: 0,
            array_params: HashSet::new(),
            const_fns: HashSet::new(),
            owned_arrays: HashSet::new(),
            owned_return_fns: HashSet::new(),
            codegen_errors: Vec::new(),
            bounds_check: true,
            thread_safe_cached: HashSet::new(),
            precomputed: HashMap::new(),
            generic_fns: HashMap::new(),
            fn_ret_types: HashMap::new(),
            tmp_counter: 0,
            fn_param_types: HashMap::new(),
            fn_array_params: HashMap::new(),
            fn_param_names: HashMap::new(),
            fn_param_defaults: HashMap::new(),
            method_array_params: HashMap::new(),
            array_ret_fns: HashSet::new(),
            monomorphized: HashMap::new(),
            pending_monos: Vec::new(),
            par_counter: 0,
            par_structs: String::new(),
            par_worker_decls: String::new(),
            par_workers: String::new(),
            par_scalars: Vec::new(),
            non_neg_vars: std::collections::HashSet::new(),
            const_ints: HashMap::new(),
            repeat_counter: 0,
            scope_marks: Vec::new(),
            loop_depth: 0,
            scope_counter: 0,
            cur_ret_is_array: false,
            cur_ret_c: String::new(),
            opt_names: RefCell::new(HashMap::new()),
            opt_elems: RefCell::new(HashMap::new()),
            match_val_member: None,
        }
    }

    pub fn set_precomputed(&mut self, name: String, table: Vec<i64>) {
        self.precomputed.insert(name, table);
    }

    /// a[i] 的运行期边界检查开关（默认开）。
    /// 关掉之后越界又会回到静默读写内存，只给手工验证过的热点内核用；
    /// 单个函数可以用 `@unchecked` 关，不必整份文件关。
    pub fn set_bounds_check(&mut self, on: bool) {
        self.bounds_check = on;
    }

    pub fn generate(&mut self, program: &Program) -> String {
        self.output.push_str(runtime::RUNTIME_HEADER);
        self.output.push('\n');

        // 可空 typedef 的插入点。opt_tag 在后面的 gen_type / infer_type 里
        // 惰性分配，所以真正要发哪些 typedef 只有跑完才知道 —— 和
        // par_workers 一样，最后再 insert 回这个位置。
        let opt_pos = self.output.len();

        // 0.9 枚举：类型名 → 变体表。必须早于 struct 收集 ——
        //     结构体字段的类型可以直接写枚举名，而 gen_type 只把名字原样
        //     交给 C（真正的 typedef 在第 2.8 步发）。
        for item in &program.items {
            if let Item::Enum(e) = item {
                self.enums.insert(e.name.clone(), e.resolved());
            }
        }

        // 1. 收集 struct 定义和方法签名
        let mut tag_counter: i64 = 0;
        for item in &program.items {
            if let Item::Struct(s) = item {
                let fields: Vec<(String, String)> = s.fields.iter()
                    .map(|f| (f.name.clone(), self.gen_type(&f.ty)))
                    .collect();
                self.structs.insert(s.name.clone(), fields);
                // Vec 字段的元素类型也要单独记：字段的 C 类型只有
                // "LumeVec*"，取元素时得知道按什么类型强转。
                for f in &s.fields {
                    if let Some(elem) = vec_elem_type(&f.ty) {
                        let c = self.gen_type(elem);
                        self.vec_field_elems
                            .insert((s.name.clone(), f.name.clone()), c);
                    }
                }
                // 按出现顺序分配类型标签
                self.struct_tags.insert(s.name.clone(), tag_counter);
                tag_counter += 1;

                for m in &s.methods {
                    let ret = match &m.ret {
                        None => "void".to_string(),
                        Some(t) => self.gen_type(t),
                    };
                    self.methods.insert((s.name.clone(), m.name.clone()), ret);
                    // 方法返回 Vec 时，调用点 `let v = obj.m()` 要能拿到
                    // 元素类型才能取元素。
                    if let Some(elem) = m.ret.as_ref().and_then(vec_elem_type) {
                        let c = self.gen_type(elem);
                        self.method_vec_elems
                            .insert((s.name.clone(), m.name.clone()), c);
                    }
                    self.method_array_params.insert(
                        (s.name.clone(), m.name.clone()),
                        m.params.iter().map(|p| is_array_ty(&p.ty)).collect(),
                    );
                }
            }
        }

        // 2. 收集泛型函数
        for item in &program.items {
            if let Item::Fn(f) = item {
                if !f.generics.is_empty() {
                    self.generic_fns.insert(f.name.clone(), f.clone());
                }
            }
        }

        // 2.4 收集普通函数的返回类型 + 数组参数标记。
        //     没有返回类型表，say("{f(x)}") 里 f 返回 float 也会被推断成
        //     int64_t，插值就用 %lld 打印，小数全丢。
        //     数组参数标记则是调用点自动补长度的依据。
        for item in &program.items {
            if let Item::Fn(f) = item {
                let ret = match &f.ret {
                    None => "void".to_string(),
                    Some(t) => self.gen_type(t),
                };
                self.fn_ret_types.insert(f.name.clone(), ret);
                // 参数类型表：管道融合要校验 f 的入参类型接不接得上
                self.fn_param_types.insert(
                    f.name.clone(),
                    f.params
                        .iter()
                        .map(|p| match &p.ty {
                            Some(t) => self.gen_type(t),
                            None => "int64_t".to_string(),
                        })
                        .collect(),
                );
                self.fn_array_params.insert(
                    f.name.clone(),
                    f.params.iter().map(|p| is_array_ty(&p.ty)).collect(),
                );
                // 默认参数 + 命名参数的依据。位置调用省略尾部参数时，
                // gen_call_args 从这里取默认值在调用处展开；命名参数
                // 调用则靠 fn_param_names 重排（见 gen_named_call）。
                self.fn_param_names.insert(
                    f.name.clone(),
                    f.params.iter().map(|p| p.name.clone()).collect(),
                );
                self.fn_param_defaults.insert(
                    f.name.clone(),
                    f.params.iter().map(|p| p.default.clone()).collect(),
                );
                if is_array_ty(&f.ret) {
                    self.array_ret_fns.insert(f.name.clone());
                }
                // 返回 Vec 的函数：Vec 自带 len，不需要 _ret_len 伴随参数，
                // 但调用点要能从元素类型推出怎么取元素。
                if let Some(elem) = f.ret.as_ref().and_then(vec_elem_type) {
                    let c = self.gen_type(elem);
                    self.fn_vec_elems.insert(f.name.clone(), c);
                }
            }
        }

        // 2.45 预置内置函数签名。
        //      infer_type / 数组长度推导都要查这些表，
        //      不预置的话 say("{str_contains(a,b)}") 会被推断成 int64_t。
        self.register_builtin_sigs();

        // 2.5 纯函数分析：必须在发前向声明之前跑完，
        //     因为 __attribute__((const)) 要写在声明上，调用点才能看到
        self.collect_const_fns(program);

        // 2.55 顶层整数常量表：`x % M` 里的 M 要在这里就能解析成字面量，
        //      才能在 codegen 阶段做常量除数安全化 + 强度削弱（见 expr.rs）。
        self.collect_const_ints(program);

        // 2.6 哪些 @cache 会被 par for 调到（含经由普通函数间接调用）。
        //     这些要用带锁的缓存包装器，其余保持无锁快路径。
        self.thread_safe_cached = collect_par_cached_fns(program);

        // 2.7 哪些函数返回的是**新分配**数组（而非入参别名）。
        //     `let r = f()` 只有对这类函数才把 r 标记成 owned，允许 free。
        self.owned_return_fns = collect_owned_return_fns(program);

        // 2.8 枚举定义。必须在 struct 定义之前：结构体字段可以是枚举类型。
        for item in &program.items {
            if let Item::Enum(e) = item {
                self.gen_enum_decl(e);
            }
        }

        // 3. 生成 struct 定义
        //
        // **按依赖排序**：字段类型引用了别的 struct，就先发被依赖的。
        // 多文件项目和库里，struct 的物理顺序和依赖顺序无关（按路径排序
        // 合并），照源顺序发会出现 `typedef struct Box { Dim dim; }` 排在
        // `typedef struct Dim` 前面 —— clang: unknown type name 'Dim'。
        let all_structs: HashSet<&str> = program
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Struct(s) => Some(s.name.as_str()),
                _ => None,
            })
            .collect();
        let mut pending: Vec<&StructDecl> = program
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Struct(s) => Some(s),
                _ => None,
            })
            .collect();
        let mut emitted: HashSet<String> = HashSet::new();
        while !pending.is_empty() {
            let mut deferred: Vec<&StructDecl> = Vec::new();
            let mut progressed = false;
            for s in pending {
                let mut deps: Vec<String> = Vec::new();
                for f in &s.fields {
                    collect_struct_deps(&f.ty, &mut deps);
                }
                // 未定义且不是自己 → 等下一轮。自引用（`kids: Vec[Node]`）
                // 的 C 类型是 LumeVec*，不需要前置定义。
                let ready = deps
                    .iter()
                    .all(|d| !all_structs.contains(d.as_str()) || emitted.contains(d) || *d == s.name);
                if ready {
                    self.gen_struct_decl(s);
                    emitted.insert(s.name.clone());
                    progressed = true;
                } else {
                    deferred.push(s);
                }
            }
            if !progressed {
                // 环状按值包含（A 有字段 B，B 有字段 A）：C 的 struct 不能
                // 互相按值嵌套，怎么排都编不过。按原顺序发出去让 clang 报，
                // 比在这里静默丢定义好。
                for s in deferred {
                    self.gen_struct_decl(s);
                }
                break;
            }
            pending = deferred;
        }

        // 3.2 顶层常量：必须早于所有函数体。C 里的 `const T x = v;` 是**定义**，
        //     没有「前向声明」的等效写法，函数体里引用时值必须已经出现过。
        //     （以前跟着第 9 步的函数一起按源文件顺序发，常量写在函数后面就编不过。）
        for item in &program.items {
            if let Item::Const(c) = item {
                self.gen_const(c);
            }
        }

        // par for 的 worker 插在这里：必须在所有函数定义之前，
        // 这样后面的函数才能直接调用它
        let worker_pos = self.output.len();

        // 3.5 前向声明（方法 + 普通函数）
        //     这样函数可以互相调用、也可以调用后面才定义的函数
        for item in &program.items {
            if let Item::Struct(s) = item {
                for m in &s.methods {
                    self.gen_method_decl(&s.name, m);
                }
            }
        }
        for item in &program.items {
            if let Item::Fn(f) = item {
                if !f.generics.is_empty() {
                    continue;
                }
                // cache/precompute 的包装器签名和普通函数不完全一样
                // （单参数、不带数组伴随参数），要单独发原型。
                // 以前整个跳过，结果 par worker 里调用 @cache 函数会生成
                // “call to undeclared function” 的非法 C。
                let is_cache = f.decorators.iter().any(|d| d.name == "cache");
                let is_pre = f.decorators.iter().any(|d| d.name == "precompute");
                if (is_cache || is_pre) && Self::cached_wrapper_shape(f, is_pre) {
                    self.gen_cached_wrapper_decl(f, is_pre);
                } else {
                    self.gen_fn_decl(f);
                }
            }
        }
        self.output.push('\n');

        // 4. 生成方法
        for item in &program.items {
            if let Item::Struct(s) = item {
                for m in &s.methods {
                    self.gen_method(&s.name, m);
                }
            }
        }

        // 5. 预扫描：收集所有泛型调用 → pending_monos
        self.collect_generic_calls(program);

        // 6. 循环收集，直到没有新的泛型调用
        let mut monos: Vec<(FnDecl, String, HashMap<String, String>)> = Vec::new();
        loop {
            self.collect_generic_calls(program);
            let batch: Vec<_> = self.pending_monos.drain(..).collect();
            if batch.is_empty() {
                break;
            }
            monos.extend(batch);
        }

        // 7. 生成所有单态化的前向声明
        for (f, name, tm) in &monos {
            self.gen_monomorphized_decl(f, name, tm);
        }
        if !monos.is_empty() {
            self.output.push('\n');
        }

        // 8. 单态化定义
        for (f, name, tm) in &monos {
            self.gen_monomorphized(f, name, tm);
        }

       // 9. 普通函数（顶层常量已在 3.2 步发完）
        for item in &program.items {
            if let Item::Fn(f) = item {
                self.gen_fn(f);
            }
        }

        let mut out = std::mem::take(&mut self.output);
        // par for 的 args 结构体 + worker 声明必须早于方法/函数定义；
        // worker **定义**则要等所有函数原型（含 @cache 包装器、泛型单态化）
        // 都发完之后再追加 —— 否则 worker 里调用它们会变成隐式声明。
        if !self.par_structs.is_empty() || !self.par_worker_decls.is_empty() {
            out.insert_str(
                worker_pos,
                &format!("{}{}", self.par_structs, self.par_worker_decls),
            );
        }
        // 可空 typedef 必须早于所有使用点（struct 字段/函数原型）
        out.insert_str(opt_pos, &self.gen_opt_typedefs());
        if !self.par_workers.is_empty() {
            out.push('\n');
            out.push_str(&self.par_workers);
        }

        // 日志线程懒启动：主函数里先放占位符，这里看生成代码里有没有
        // 真正的日志调用。没有就整行删掉 —— 空程序不再起 writer 线程。
        // 只扫 runtime header 之后的部分，避免把 header 自己的定义算进去。
        let body = &out[runtime::RUNTIME_HEADER.len()..];
        let uses_log = body.contains("lume_logf(")
            || body.contains("lume_log_write_literal(")
            || body.contains("lume_log_flush()");
        let placeholder = "    /*__LUME_LOG_INIT__*/\n";
        out = out.replace(
            placeholder,
            if uses_log { "    lume_log_init_main();\n" } else { "" },
        );

        // free 误用之类的安全错误：放在文件最前面，clang 会立刻带着
        // 可读信息停下，而不是生成一段会崩的 C。
        if !self.codegen_errors.is_empty() {
            let mut pre = String::new();
            for e in &self.codegen_errors {
                let escaped = e.replace('\\', "\\\\").replace('"', "\\\"");
                pre.push_str(&format!("#error \"{}\"\n", escaped));
            }
            pre.push_str(&out);
            out = pre;
        }
        out
    }

    /// 发出本次用到的全部 lume_opt_N 定义。
    fn gen_opt_typedefs(&self) -> String {
        // 按 tag 的编号排序，保证输出稳定（HashMap 迭代序随机）
        let mut pairs: Vec<(String, String)> = self
            .opt_elems
            .borrow()
            .iter()
            // 字符串可空值的 typedef 写在 runtime header 里（文件 IO 要用），
            // 这里再发一遍就是 redefinition。
            .filter(|(tag, _)| tag.as_str() != types::OPT_STR_TAG)
            .map(|(tag, elem)| (tag.clone(), elem.clone()))
            .collect();
        if pairs.is_empty() {
            return String::new();
        }
        pairs.sort_by(|a, b| {
            let na: usize = a.0.trim_start_matches("lume_opt_").parse().unwrap_or(0);
            let nb: usize = b.0.trim_start_matches("lume_opt_").parse().unwrap_or(0);
            na.cmp(&nb)
        });
        let mut s = String::from("/* ---- 可空类型 (T? / T!) ---- */\n");
        for (tag, elem) in pairs {
            s.push_str(&format!(
                "typedef struct {{ int8_t has; {} val; }} {};\n",
                elem, tag
            ));
        }
        s.push('\n');
        s
    }

    pub fn generate_precompute_program(
        &mut self,
        f: &FnDecl,
        start: i64,
        end: i64,
    ) -> String {
        self.output.push_str(runtime::RUNTIME_HEADER);
        self.output.push('\n');

        self.gen_thread_safe_cached_fn(f);

        let n = end - start + 1;
        self.push_line("int main(void) {");
        self.indent += 1;

        self.push_line(&format!("int64_t _N = {}LL;", n));
        self.push_line("int64_t* _results = malloc(_N * sizeof(int64_t));");
        self.push_line(&format!(
            "int64_t _start = {}LL, _end = {}LL;",
            start, end
        ));
        self.push_line("#pragma omp parallel for");
        self.push_line("for (int64_t i = _start; i <= _end; i++) {");
        self.indent += 1;
        self.push_line(&format!(
            "_results[i - _start] = {}(i);",
            f.name
        ));
        self.indent -= 1;
        self.push_line("}");
        self.push_line("for (int64_t i = 0; i < _N; i++) {");
        self.indent += 1;
        self.push_line(&format!("printf(\"%lld\\n\", (long long)_results[i]);"));
        self.indent -= 1;
        self.push_line("}");
        self.push_line("free(_results);");
        self.push_line("return 0;");

        self.indent -= 1;
        self.push_line("}");

        std::mem::take(&mut self.output)
    }

    /// 内置函数签名表。数据来自 `crate::builtins`，这里只负责登记。
    /// 返回 `const char**` 的（只有 str_split）同时登记成数组返回函数，
    /// 走 `let x = f(...)` 的三行拆开。
    fn register_builtin_sigs(&mut self) {
        for b in crate::builtins::TABLE {
            // `str?` 的 C 类型由 opt_tag 决定（固定为 LumeOptStr），
            // 静态表里给不出来，所以这里特判。
            let ret = match b.ret {
                crate::builtins::Ret::OptStr => self.opt_tag("const char*"),
                other => other.c_type().to_string(),
            };
            self.fn_ret_types.insert(b.name.to_string(), ret.to_string());
            if b.ret == crate::builtins::Ret::StrArray {
                self.array_ret_fns.insert(b.name.to_string());
            }
        }
    }

    // ── 函数局部作用域 ──
    //
    // var_types / array_len_exprs / array_elem_types / array_params
    // 描述的都是「当前函数内部」的符号。Codegen 是单个实例、跨函数复用的，
    // 不清空就会串味：`out` 在 dbl() 里是数组参数，在 scaled() 里是局部
    // 数组，混在一起就分不清它是入参还是栈上变量 —— 而这两者的返回值
    // 处理完全不同（入参能直接 return，栈数组必须 malloc 拷贝）。
    pub(crate) fn begin_fn_scope(&mut self) {
        self.var_types.clear();
        self.self_fields.clear();
        self.array_len_exprs.clear();
        self.array_elem_types.clear();
        self.array_params.clear();
        self.owned_arrays.clear();
        self.vec_elems.clear();
        self.cur_self_struct = None;
        self.cur_field_names.clear();
    }

    pub(crate) fn push_line(&mut self, s: &str) {
        self.output.push_str(&"    ".repeat(self.indent));
        self.output.push_str(s);
        self.output.push('\n');
    }

    // ── 泛型调用预扫描 ──

    fn collect_generic_calls(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Fn(f) => {
                    if !f.generics.is_empty() {
                        continue;
                    }
                    for stmt in &f.body.stmts {
                        self.walk_stmt_for_generics(stmt);
                    }
                }
                Item::Struct(s) => {
                    for m in &s.methods {
                        for stmt in &m.body.stmts {
                            self.walk_stmt_for_generics(stmt);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn walk_stmt_for_generics(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(e) => self.walk_expr_for_generics(e),
            Stmt::Return(Some(e)) => self.walk_expr_for_generics(e),
            Stmt::Let { init, .. } => self.walk_expr_for_generics(init),
            Stmt::Assign { target, value } => {
                self.walk_expr_for_generics(target);
                self.walk_expr_for_generics(value);
            }
            Stmt::If { cond, then, els } => {
                self.walk_expr_for_generics(cond);
                for s in &then.stmts {
                    self.walk_stmt_for_generics(s);
                }
                if let Some(e) = els {
                    self.walk_stmt_for_generics(e);
                }
            }
            Stmt::While { cond, body } => {
                self.walk_expr_for_generics(cond);
                for s in &body.stmts {
                    self.walk_stmt_for_generics(s);
                }
            }
            Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
                self.walk_expr_for_generics(iter);
                for s in &body.stmts {
                    self.walk_stmt_for_generics(s);
                }
            }
            _ => {}
        }
    }

    fn walk_expr_for_generics(&mut self, expr: &Expr) {
        match expr {
            Expr::Call(callee, args) => {
                // 泛型调用 → 入队
                if let Expr::Ident(fname) = callee.as_ref() {
                    if self.generic_fns.contains_key(fname) {
                        let fname = fname.clone();
                        let _ = self.monomorphize_call(&fname, args);
                    }
                }
                self.walk_expr_for_generics(callee);
                for a in args {
                    self.walk_expr_for_generics(a);
                }
            }
            Expr::Binary(l, _, r) => {
                self.walk_expr_for_generics(l);
                self.walk_expr_for_generics(r);
            }
            Expr::Unary(_, e) => self.walk_expr_for_generics(e),
            Expr::Field(o, _) => self.walk_expr_for_generics(o),
            Expr::Index(o, i) => {
                self.walk_expr_for_generics(o);
                self.walk_expr_for_generics(i);
            }
            Expr::StructLit(_, fields) => {
                for (_, v) in fields {
                    self.walk_expr_for_generics(v);
                }
            }
            Expr::List(items) => {
                for e in items {
                    self.walk_expr_for_generics(e);
                }
            }
            Expr::Tuple(items) => {
                for e in items {
                    self.walk_expr_for_generics(e);
                }
            }
            Expr::If(cond, then, els) => {
                self.walk_expr_for_generics(cond);
                for s in &then.stmts {
                    self.walk_stmt_for_generics(s);
                }
                if let Some(b) = els {
                    for s in &b.stmts {
                        self.walk_stmt_for_generics(s);
                    }
                }
            }
            Expr::Lambda { body, .. } => {
                for s in &body.stmts {
                    self.walk_stmt_for_generics(s);
                }
            }
            _ => {}
        }
    }
}


// ── @cache 在 par for 里的线程安全判定 ──
//
// 直接看 AST：把所有 par for 循环体里调到的函数名当种子，
// 再沿普通函数/方法的调用图传递闭包。落在 @cache 集合里的，
// 生成带锁包装器；其余保持无锁（单线程快路径 +2ns，不能白送 20ns）。
fn collect_par_cached_fns(program: &Program) -> HashSet<String> {
    let mut cache_fns: HashSet<String> = HashSet::new();
    let mut edges: HashMap<String, HashSet<String>> = HashMap::new();
    let mut seeds: HashSet<String> = HashSet::new();

    for item in &program.items {
        match item {
            Item::Fn(f) => {
                if f.decorators.iter().any(|d| d.name == "cache") {
                    cache_fns.insert(f.name.clone());
                }
                let mut calls = HashSet::new();
                collect_calls_block(&f.body, &mut calls);
                edges.insert(f.name.clone(), calls);
                collect_par_seeds_block(&f.body, &mut seeds);
            }
            Item::Struct(s) => {
                for m in &s.methods {
                    let mut calls = HashSet::new();
                    collect_calls_block(&m.body, &mut calls);
                    edges.insert(m.name.clone(), calls);
                    collect_par_seeds_block(&m.body, &mut seeds);
                }
            }
            _ => {}
        }
    }

    let mut reach = seeds;
    let mut work: Vec<String> = reach.iter().cloned().collect();
    while let Some(n) = work.pop() {
        if let Some(callees) = edges.get(&n) {
            for c in callees {
                if reach.insert(c.clone()) {
                    work.push(c.clone());
                }
            }
        }
    }
    reach.intersection(&cache_fns).cloned().collect()
}

fn collect_par_seeds_block(b: &Block, out: &mut HashSet<String>) {
    for s in &b.stmts {
        match s {
            Stmt::ParFor { body, .. } => collect_calls_block(body, out),
            Stmt::If { then, els, .. } => {
                collect_par_seeds_block(then, out);
                if let Some(e) = els {
                    collect_par_seeds_stmt(e, out);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                collect_par_seeds_block(body, out)
            }
            _ => {}
        }
    }
}

fn collect_par_seeds_stmt(s: &Stmt, out: &mut HashSet<String>) {
    match s {
        Stmt::ParFor { body, .. } => collect_calls_block(body, out),
        Stmt::If { then, els, .. } => {
            collect_par_seeds_block(then, out);
            if let Some(e) = els {
                collect_par_seeds_stmt(e, out);
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => collect_par_seeds_block(body, out),
        Stmt::Expr(Expr::If(_, then, els)) => {
            collect_par_seeds_block(then, out);
            if let Some(e) = els {
                collect_par_seeds_block(e, out);
            }
        }
        _ => {}
    }
}

fn collect_calls_block(b: &Block, out: &mut HashSet<String>) {
    for s in &b.stmts {
        collect_calls_stmt(s, out);
    }
}

fn collect_calls_stmt(s: &Stmt, out: &mut HashSet<String>) {
    match s {
        Stmt::Let { init, .. } => collect_calls_expr(init, out),
        Stmt::Assign { target, value } => {
            collect_calls_expr(target, out);
            collect_calls_expr(value, out);
        }
        Stmt::Expr(e) | Stmt::Return(Some(e)) => collect_calls_expr(e, out),
        Stmt::If { cond, then, els } => {
            collect_calls_expr(cond, out);
            collect_calls_block(then, out);
            if let Some(e) = els {
                collect_calls_stmt(e, out);
            }
        }
        Stmt::While { cond, body } => {
            collect_calls_expr(cond, out);
            collect_calls_block(body, out);
        }
        Stmt::For { iter, body, .. } | Stmt::ParFor { iter, body, .. } => {
            collect_calls_expr(iter, out);
            collect_calls_block(body, out);
        }
        Stmt::Match { value, arms } => {
            collect_calls_expr(value, out);
            for a in arms {
                collect_calls_expr(&a.body, out);
            }
        }
        Stmt::Scope { body } => collect_calls_block(body, out),
        _ => {}
    }
}

fn collect_calls_expr(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::FmtStr(parts) => {
            for p in parts {
                if let StrPart::Expr(x) = p {
                    collect_calls_expr(x, out);
                }
            }
        }
        Expr::Binary(l, _, r) => {
            collect_calls_expr(l, out);
            collect_calls_expr(r, out);
        }
        Expr::Unary(_, x) | Expr::Await(x) | Expr::Try(x) | Expr::Force(x) => {
            collect_calls_expr(x, out)
        }
        Expr::Call(callee, args) => {
            match callee.as_ref() {
                Expr::Ident(n) => {
                    out.insert(n.clone());
                }
                Expr::Field(_, m) => {
                    out.insert(m.clone());
                }
                other => collect_calls_expr(other, out),
            }
            for a in args {
                collect_calls_expr(a, out);
            }
        }
        Expr::Field(o, _) => collect_calls_expr(o, out),
        Expr::Index(o, i) => {
            collect_calls_expr(o, out);
            collect_calls_expr(i, out);
        }
        Expr::Slice(o, a, b, _) => {
            collect_calls_expr(o, out);
            if let Some(x) = a {
                collect_calls_expr(x, out);
            }
            if let Some(x) = b {
                collect_calls_expr(x, out);
            }
        }
        Expr::Lambda { body, .. } | Expr::Spawn(body) => collect_calls_block(body, out),
        Expr::Tuple(v) | Expr::List(v) => {
            for x in v {
                collect_calls_expr(x, out);
            }
        }
        Expr::Map(pairs) => {
            for (k, v) in pairs {
                collect_calls_expr(k, out);
                collect_calls_expr(v, out);
            }
        }
        Expr::StructLit(_, fields) => {
            for (_, v) in fields {
                collect_calls_expr(v, out);
            }
        }
        Expr::Repeat(x, _) => collect_calls_expr(x, out),
        Expr::Range(a, b, _) => {
            collect_calls_expr(a, out);
            collect_calls_expr(b, out);
        }
        Expr::If(c, t, e) => {
            collect_calls_expr(c, out);
            collect_calls_block(t, out);
            if let Some(b) = e {
                collect_calls_block(b, out);
            }
        }
        Expr::Match(v, arms) => {
            collect_calls_expr(v, out);
            for a in arms {
                collect_calls_expr(&a.body, out);
            }
        }
        _ => {}
    }
}

// ── 数组返回值所有权分析 ──
//
// 只认「一定新分配」的返回：
//   return [1, 2, 3]      → codegen malloc
//   return [0; N]         → codegen malloc
//   return local_array    → 局部数组是栈上的，codegen 会 malloc 拷贝
// 入参（array param）直接返回的是调用方内存，绝不能算 owned。
// 其他形式（字段、调用、切片表达式……）一律保守地当作「不是新分配」，
// 这样 free 会在编译期被拒绝，而不是运行期 free 到栈/别名。

fn collect_owned_return_fns(program: &Program) -> HashSet<String> {
    let mut out = HashSet::new();
    for item in &program.items {
        if let Item::Fn(f) = item {
            if !f.generics.is_empty() || !is_array_ty(&f.ret) {
                continue;
            }
            let params: HashSet<String> =
                f.params.iter().map(|p| p.name.clone()).collect();
            let mut rets: Vec<&Expr> = Vec::new();
            collect_returns_block(&f.body, &mut rets);
            if !rets.is_empty()
                && rets.iter().all(|e| array_return_is_fresh(e, &params))
            {
                out.insert(f.name.clone());
            }
        }
    }
    out
}

fn array_return_is_fresh(e: &Expr, params: &HashSet<String>) -> bool {
    match e {
        Expr::List(_) | Expr::Repeat(..) => true,
        Expr::Ident(n) => !params.contains(n),
        _ => false,
    }
}

fn collect_returns_block<'a>(b: &'a Block, out: &mut Vec<&'a Expr>) {
    for s in &b.stmts {
        collect_returns_stmt(s, out);
    }
}

fn collect_returns_stmt<'a>(s: &'a Stmt, out: &mut Vec<&'a Expr>) {
    match s {
        Stmt::Return(Some(e)) => out.push(e),
        Stmt::If { then, els, .. } => {
            collect_returns_block(then, out);
            if let Some(e) = els {
                collect_returns_stmt(e, out);
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ParFor { body, .. } => {
            collect_returns_block(body, out)
        }
        // `else { ... }` 被 parser 包成 Expr::If(true, block)
        Stmt::Expr(Expr::If(_, then, els)) => {
            collect_returns_block(then, out);
            if let Some(b) = els {
                collect_returns_block(b, out);
            }
        }
        _ => {}
    }
}
