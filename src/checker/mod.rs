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