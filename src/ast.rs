#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>,
}

// ─────────────────────────────────────────────────────────────
// 下面这批 `#[allow(dead_code)]` 不是掩盖问题，而是**明确记录**
// 「语法已经解析、但 codegen 还没实现」的半成品特性。
// 消 warning 有两种做法：删掉字段，或者保留字段并标注待办。
// 这里选后者 —— 删掉就等于把语言设计从 AST 里抹掉，以后想接回来得重新推。
//
// 待办清单（按「离能用还有多远」排）：
//   FnDecl/StructDecl/
//   Field::is_pub       可见性。没有模块系统之前 `pub` 是空操作，
//                       等 import 真正生效才有意义。
//   ImportDecl          模块导入。现在 `import std.str` 是空操作，
//                       std 函数在 runtime header 里硬编码，不 import 也能用。
//   AppDecl             `app` 块。已解析，codegen 完全没接。
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Item {
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    #[allow(dead_code)]
    App(AppDecl),
    Const(ConstDecl),
    #[allow(dead_code)]
    Import(ImportDecl),
}

#[derive(Debug, Clone)]
pub struct Decorator {
    pub name: String,
    pub args: Vec<Expr>,
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub name: String,
    pub generics: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Block,
    /// `extern fn name(...) -> T`：只声明、不生成函数体。
    ///
    /// C 侧标识符会加 `lume_ext_` 前缀并用 `__asm__("name")` 指回真正的
    /// 符号名 —— 直接用原名声明会和 libc 头文件里的声明撞类型
    /// （`int puts(const char*)` vs `int64_t puts(...)` 是 conflicting types）。
    pub is_extern: bool,
    /// `extern fn printf(fmt: str, ...)`：参数表末尾的 `...`。
    /// 只对 extern 有意义，调用处多传的实参不做类型检查。
    pub variadic: bool,
    /// 待办：可见性，见文件头说明
    #[allow(dead_code)]
    pub is_pub: bool,
    pub decorators: Vec<Decorator>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Option<Type>,
    /// 默认值。在调用处展开（C 语义），必须排在参数列表尾部
    /// （collect.rs 的 check_defaults_trailing 校验）。
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<Field>,
    pub methods: Vec<FnDecl>,
    /// 待办：可见性，见文件头说明
    #[allow(dead_code)]
    pub is_pub: bool,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    /// 待办：可见性，见文件头说明
    #[allow(dead_code)]
    pub is_pub: bool,
}

/// 枚举：`enum Color { Red Green Blue }`。
///
/// 有意做成 C 风格的**无载荷**枚举（每个变体就是一个整数），不走
/// 「带数据的 tagged union」那条路。理由：语言现在还没有泛型载荷、
/// 没有解构绑定，先让「几十种 token 类型」这类场景能写。
/// 带载荷的版本等 `match` 能绑定载荷时再说，见 todo.md。
#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    /// 显式值：`Red = 10`。None 表示「上一个 + 1」，第一个从 0 起。
    pub value: Option<i64>,
}

impl EnumDecl {
    /// 变体名 → 整数值。显式值之后没写的继续 +1。
    pub fn resolved(&self) -> Vec<(String, i64)> {
        let mut out = Vec::new();
        let mut next = 0i64;
        for v in &self.variants {
            let val = v.value.unwrap_or(next);
            out.push((v.name.clone(), val));
            next = val + 1;
        }
        out
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AppDecl {
    pub name: String,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<Type>,
    pub value: Expr,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ImportDecl {
    pub path: Vec<String>,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int, I8, I16, I32, I64, I128,
    U1, U2, U4, U8, U16, U32, U64, U128,
    F32, F64, Bool, Str, Rune,
    Named(String),
    List(Box<Type>),
    Array(Box<Type>, Option<usize>),
    /// 待办：map 类型（`map[str]int`）。parser 未产出，codegen 未实现。
    #[allow(dead_code)]
    Map(Box<Type>, Box<Type>),
    Tuple(Vec<Type>),
    Optional(Box<Type>),
    Error(Box<Type>),
    Generic(Box<Type>, Vec<Type>),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

// ── 字符串插值 ──
// "a = {x}" 解析成 [Text("a = "), Expr(Ident("x"))]
#[derive(Debug, Clone)]
pub enum StrPart {
    Text(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let { name: String, ty: Option<Type>, init: Expr },
    Assign { target: Expr, value: Expr },
    Expr(Expr),
    If { cond: Expr, then: Block, els: Option<Box<Stmt>> },
    While { cond: Expr, body: Block },
    For { var: String, iter: Expr, body: Block },
    ParFor { var: String, iter: Expr, body: Block },
    Return(Option<Expr>),
    Break,
    Continue,
    Match { value: Expr, arms: Vec<MatchArm> },
    /// `scope { ... }`：块内派生的字符串在块尾一次性还给 arena。
    /// 变量作用域和普通块一样（块内声明在外面看不到）。
    Scope { body: Block },
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Ident(String),
    Literal(Expr),
    Tuple(Vec<Pattern>),
    /// 枚举变体模式 `Color.Red`：`(枚举名, 变体名)`。
    /// 必须带枚举名限定 —— 裸写一个 `Red` 是变量绑定，不是模式匹配。
    EnumVariant(String, String),
    /// 待办：结构体模式 `Point { x, y }`。字段名目前被丢弃（`define_pattern` 只绑定值）。
    #[allow(dead_code)]
    Struct(String, Vec<(String, Pattern)>),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    FmtStr(Vec<StrPart>),
    Rune(char),
    Color(u32),
    Bool(bool),
    Nothing,
    Ident(String),
    It,
    Binary(Box<Expr>, BinOp, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    /// 待办：切片 `a[1..3]` / `a[..n]`。AST 已定义，codegen 未实现。
    #[allow(dead_code)]
    Slice(Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>, bool),
    Lambda { params: Vec<Param>, body: Block },
    Tuple(Vec<Expr>),
    List(Vec<Expr>),
    /// 待办：map 字面量 `{"a": 1}`。parser 未产出，codegen 未实现。
    #[allow(dead_code)]
    Map(Vec<(Expr, Expr)>),
    StructLit(String, Vec<(String, Expr)>),
    Repeat(Box<Expr>, usize),
    Range(Box<Expr>, Box<Expr>, bool),
    If(Box<Expr>, Block, Option<Block>),
    /// 待办：match 表达式。语句形式的 `match` 已解析，表达式形式未接 codegen。
    #[allow(dead_code)]
    Match(Box<Expr>, Vec<MatchArm>),
    Await(Box<Expr>),
    /// 待办：`spawn` 并发块。已解析，codegen 未实现。
    #[allow(dead_code)]
    Spawn(Block),
    Try(Box<Expr>),
    Force(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
    Pipe,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

// ─────────────────────────────────────────────────────────────
// 命名参数重排（checker 和 codegen 共用，逻辑必须一份）
//
// `f(b: 1, a: 2)` 在语法上和结构体构造 `Point(x: 1)` 完全同形，
// parser 一律解析成 StructLit。区分靠语义：名字在 structs 表里 →
// 结构体构造；否则 → 命名参数函数调用，由两边各自 fallback。
//
// 返回按签名顺序排列的槽位，None = 调用处没传（等默认值来填，
// 没有默认值就是错误，由 checker 报）。槽位而不是紧凑 Vec，
// 是因为命名参数允许省略中间有默认值的参数。
/// 识别 `Vec[T]`，返回元素类型。
///
/// 语法上不需要为 Vec 加任何新分支：`Vec[T]` 落在 `parse_type` 的
/// 后缀方括号里，产出 `Type::Generic(Named("Vec"), [T])`。
/// checker 和 codegen 都从这里取元素类型，避免两边各写一份模式匹配
/// 而漏掉某一种形状。
pub fn vec_elem_type(t: &Type) -> Option<&Type> {
    match t {
        Type::Generic(base, args) if args.len() == 1 => match base.as_ref() {
            Type::Named(n) if n == "Vec" => Some(&args[0]),
            _ => None,
        },
        _ => None,
    }
}

/// 把「字面量整数表达式」解析成 i64。只认 `8` 和 `-8` 两种形状。
///
/// checker（编译期除零判定）和 codegen（常量除数强度削弱）共用一份，
/// 保证两边对「什么算常量」的判断不会走偏。
pub fn literal_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Int(v) => Some(*v),
        Expr::Unary(UnOp::Neg, inner) => match inner.as_ref() {
            Expr::Int(v) => v.checked_neg(),
            _ => None,
        },
        _ => None,
    }
}

pub fn reorder_named_args(
    fields: &[(String, Expr)],
    param_names: &[String],
) -> Result<Vec<Option<Expr>>, String> {
    let mut slots: Vec<Option<Expr>> = vec![None; param_names.len()];
    for (name, value) in fields {
        match param_names.iter().position(|p| p == name) {
            Some(idx) => {
                if slots[idx].is_some() {
                    return Err(format!("参数 '{}' 被传了两次", name));
                }
                slots[idx] = Some(value.clone());
            }
            None => {
                return Err(format!(
                    "没有名为 '{}' 的参数（可用参数：{}）",
                    name,
                    param_names.join(", ")
                ))
            }
        }
    }
    Ok(slots)
}