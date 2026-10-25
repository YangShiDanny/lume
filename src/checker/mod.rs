mod collect;
mod expr;
mod stmt;

use crate::ast::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Str,
    Rune,
    Void,
    Unknown,
    Struct(String),
    /// 数组。长度在运行期才知道（局部字面量除外），所以这里只记元素类型，
    /// 长度由 codegen 的隐式伴随参数负责传递。
    Array(Box<Ty>),
    /// 可选值 T? / 错误值 T!。用户决定合并成同一种「可空」语义：
    /// 用 { has, val } 表示，T! 只是 T? 的语法别名。
    Optional(Box<Ty>),
}

impl Ty {
    pub fn name(&self) -> String {
        match self {
            Ty::Int => "int".to_string(),
            Ty::Float => "float".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Str => "str".to_string(),
            Ty::Rune => "rune".to_string(),
            Ty::Void => "void".to_string(),
            Ty::Unknown => "?".to_string(),
            Ty::Struct(s) => s.clone(),
            Ty::Array(inner) => format!("[{}]", inner.name()),
            Ty::Optional(inner) => format!("{}?", inner.name()),
        }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Ty::Unknown)
    }
}

#[derive(Debug, Clone)]
pub struct FuncSig {
    pub params: Vec<Ty>,
    /// 参数名。命名参数调用 `f(a: 1)` 的重排依据。
    pub param_names: Vec<String>,
    /// 无默认值的参数个数（默认参数必须在尾部，这就是最少要传的数）。
    pub required: usize,
    /// 每个参数的默认值表达式。在调用处求值（C 的展开语义），
    /// 所以默认值只能引用调用处可见的名字，不能引用其他参数。
    pub defaults: Vec<Option<Expr>>,
    pub ret: Ty,
}

#[derive(Debug)]
pub struct CheckError {
    pub msg: String,
    pub hint: Option<String>,
}

pub struct Checker {
    pub(crate) errors: Vec<CheckError>,
    pub(crate) functions: HashMap<String, FuncSig>,
    pub(crate) generic_fns: HashSet<String>,
    pub(crate) structs: HashMap<String, HashMap<String, Ty>>,
    pub(crate) methods: HashMap<String, HashMap<String, FuncSig>>,
    pub(crate) scopes: Vec<HashMap<String, Ty>>,
    pub(crate) current_ret: Ty,
    /// 顶层 `const` 名字 → 类型。作用域查找 `lookup` 找不到时兜底到这里，
    /// 这样 `x % M` 里的 M 不会被当成「未定义的变量」。
    pub(crate) consts: HashMap<String, Ty>,
    /// 顶层 `const NAME = <整数字面量>` 的值。
    /// 用来在**编译期**发现 `x / NAME` 这种「除数是零常量」的除零 ——
    /// 以前只认裸字面量 `x / 0`，包一层常量名就漏过去了。
    pub(crate) const_ints: HashMap<String, i64>,
}

impl Checker {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            functions: HashMap::new(),
            generic_fns: HashSet::new(),
            structs: HashMap::new(),
            methods: HashMap::new(),
            scopes: Vec::new(),
            current_ret: Ty::Void,
            consts: HashMap::new(),
            const_ints: HashMap::new(),
        }
    }

    pub fn check(&mut self, program: &Program) -> Result<(), Vec<CheckError>> {
        self.collect_items(program);

        for item in &program.items {
            match item {
                Item::Fn(f) => {
                    // 泛型函数体跳过（类型未具体化）
                    if f.generics.is_empty() {
                        self.check_fn(f, None);
                    }
                }
                Item::Struct(s) => {
                    for m in &s.methods {
                        self.check_fn(m, Some(&s.name));
                    }
                }
                _ => {}
            }
        }

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(std::mem::take(&mut self.errors))
        }
    }

    // ── 错误 ──

    pub(crate) fn error(&mut self, msg: impl Into<String>) {
        self.errors.push(CheckError {
            msg: msg.into(),
            hint: None,
        });
    }

    pub(crate) fn error_hint(&mut self, msg: impl Into<String>, hint: impl Into<String>) {
        self.errors.push(CheckError {
            msg: msg.into(),
            hint: Some(hint.into()),
        });
    }

    // ── 作用域 ──

    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(crate) fn define(&mut self, name: String, ty: Ty) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<Ty> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    /// `e` 是否可证明是编译期整数零。认 `0`、`-0` 和值为 0 的顶层常量。
    /// 局部变量同名时不算 —— 那时值是运行期的事。
    pub(crate) fn is_const_zero(&self, e: &Expr) -> bool {
        match literal_int(e) {
            Some(v) => v == 0,
            None => match e {
                Expr::Ident(n) => {
                    self.lookup(n).is_none()
                        && self.const_ints.get(n).copied() == Some(0)
                }
                _ => false,
            },
        }
    }
}

pub(crate) fn is_builtin(name: &str) -> bool {
    matches!(name,
        "str_len" | "str_upper" | "str_lower" | "str_trim"
        | "str_to_int" | "str_to_float"
        | "str_contains" | "str_starts_with" | "str_ends_with"
        | "str_replace" | "str_split"
        | "int_abs" | "int_min" | "int_max"
        | "float_sqrt" | "sqrt" | "float_abs"
        | "float_pow" | "pow"
        | "float_sin" | "sin" | "float_cos" | "cos"
        | "float_tan" | "tan"
        | "float_floor" | "floor" | "float_ceil" | "ceil"
        | "float_round" | "round"
    )
}

pub(crate) fn builtin_return_type(name: &str) -> Ty {
    match name {
        "str_len" | "str_to_int" | "int_abs" | "int_min" | "int_max" => Ty::Int,
        "str_upper" | "str_lower" | "str_trim" | "str_replace" => Ty::Str,
        "str_to_float" | "float_abs" | "sqrt" | "float_sqrt" | "pow"
        | "float_pow" | "sin" | "float_sin" | "cos" | "float_cos" | "tan"
        | "float_tan" | "floor" | "float_floor" | "ceil" | "float_ceil"
        | "round" | "float_round" => Ty::Float,
        "str_contains" | "str_starts_with" | "str_ends_with" => Ty::Bool,
        "str_split" => Ty::Array(Box::new(Ty::Str)),
        _ => Ty::Unknown,
    }
}