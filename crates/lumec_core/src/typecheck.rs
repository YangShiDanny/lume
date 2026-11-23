use crate::ffi::{FfiSignature, FfiType};
use crate::manifest::lume_c_type;
use crate::parser::{Expr, FuncDef, Pattern, Stmt};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Type {
    Any,
    Int,
    I8,
    I16,
    I32,
    U8,
    U16,
    U32,
    U64,
    Float,
    Bool,
    String,
    Array,
    Struct(String),
    Function(Vec<Type>, Box<Type>),
    Unit,
}

fn ffi_ty_to_type(ty: FfiType) -> Type {
    match ty {
        FfiType::Void => Type::Unit,
        FfiType::I8 => Type::I8,
        FfiType::I16 => Type::I16,
        FfiType::I32 => Type::I32,
        FfiType::I64 => Type::Int,
        FfiType::U8 => Type::U8,
        FfiType::U16 => Type::U16,
        FfiType::U32 => Type::U32,
        FfiType::U64 => Type::U64,
        FfiType::F32 | FfiType::F64 => Type::Float,
        FfiType::Pointer => Type::Any,
        FfiType::Struct(_) | FfiType::Union(_) => Type::Any,
    }
}

impl Type {
    fn from_annotation(annotation: &str) -> Self {
        match annotation {
            "int" | "i64" => Self::Int,
            "i8" => Self::I8,
            "i16" => Self::I16,
            "i32" => Self::I32,
            "u8" => Self::U8,
            "u16" => Self::U16,
            "u32" => Self::U32,
            "u64" => Self::U64,
            "float" | "f32" | "f64" => Self::Float,
            "bool" => Self::Bool,
            "string" | "str" => Self::String,
            "void" | "unit" => Self::Unit,
            _ => Self::Any,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Int => "int",
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::String => "string",
            Self::Array => "array",
            Self::Struct(_) => "struct",
            Self::Function(_, _) => "function",
            Self::Unit => "unit",
        }
    }

    fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::Int
                | Self::I8
                | Self::I16
                | Self::I32
                | Self::U8
                | Self::U16
                | Self::U32
                | Self::U64
        )
    }

    fn integer_rank(&self) -> u8 {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 => 3,
            Self::U64 | Self::Int => 4,
            _ => 0,
        }
    }

    fn promotes_to(&self, other: &Self) -> bool {
        if !self.is_integer() || !other.is_integer() {
            return false;
        }
        self.integer_rank() <= other.integer_rank()
    }

    fn compatible_with(&self, other: &Self) -> bool {
        self == other
            || matches!(self, Self::Any)
            || matches!(other, Self::Any)
            || other.promotes_to(self)
    }
}

fn common_numeric_type(lhs: &Type, rhs: &Type) -> Type {
    if lhs == &Type::Float || rhs == &Type::Float {
        return Type::Float;
    }
    if lhs == &Type::Any || rhs == &Type::Any {
        return Type::Any;
    }
    if !lhs.is_integer() || !rhs.is_integer() {
        return Type::Any;
    }
    if lhs.integer_rank() > rhs.integer_rank() {
        return lhs.clone();
    }
    if rhs.integer_rank() > lhs.integer_rank() {
        return rhs.clone();
    }
    if lhs == rhs { lhs.clone() } else { Type::Int }
}

#[derive(Debug, Clone)]
struct FunctionType {
    params: Vec<Type>,
    result: Type,
}

pub fn check_program(functions: &[FuncDef]) -> Result<(), String> {
    check_program_with_externs(functions, &HashMap::new())
}

pub fn check_program_with_externs(
    functions: &[FuncDef],
    externs: &HashMap<String, FfiSignature>,
) -> Result<(), String> {
    let mut signatures = HashMap::new();
    for (name, signature) in externs {
        signatures.insert(
            name.clone(),
            FunctionType {
                params: signature.args.iter().cloned().map(ffi_ty_to_type).collect(),
                result: ffi_ty_to_type(signature.result.clone()),
            },
        );
    }
    for function in functions {
        if signatures.contains_key(&function.name) {
            return Err(format!(
                "function `{}` is defined more than once",
                function.name
            ));
        }
        if function.attrs.iter().any(|attr| attr == "ffi_export") {
            for param in &function.params {
                lume_c_type(param.ty.as_deref()).map_err(|error| {
                    format!(
                        "function `{}` cannot be exported to C: {error}",
                        function.name
                    )
                })?;
            }
            lume_c_type(function.ret_ty.as_deref()).map_err(|error| {
                format!(
                    "function `{}` cannot be exported to C: {error}",
                    function.name
                )
            })?;
        }
        signatures.insert(
            function.name.clone(),
            FunctionType {
                params: function
                    .params
                    .iter()
                    .map(|param| {
                        param
                            .ty
                            .as_deref()
                            .map(Type::from_annotation)
                            .unwrap_or(Type::Any)
                    })
                    .collect(),
                result: function
                    .ret_ty
                    .as_deref()
                    .map(Type::from_annotation)
                    .unwrap_or(Type::Any),
            },
        );
    }
    for function in functions {
        let locals = function
            .params
            .iter()
            .map(|param| {
                (
                    param.name.clone(),
                    param
                        .ty
                        .as_deref()
                        .map(Type::from_annotation)
                        .unwrap_or(Type::Any),
                )
            })
            .collect();
        Checker {
            signatures: &signatures,
            locals,
            return_type: function
                .ret_ty
                .as_deref()
                .map(Type::from_annotation)
                .unwrap_or(Type::Any),
            function: &function.name,
        }
        .check_block(&function.body)?;
    }
    Ok(())
}

struct Checker<'a> {
    signatures: &'a HashMap<String, FunctionType>,
    locals: HashMap<String, Type>,
    return_type: Type,
    function: &'a str,
}

impl Checker<'_> {
    fn check_block(&mut self, statements: &[Stmt]) -> Result<Type, String> {
        let mut last_type = Type::Unit;
        for statement in statements {
            match statement {
                Stmt::VarDecl { name, ty, init } => {
                    if self.locals.contains_key(name) {
                        return Err(self.error(format!("variable `{name}` is already declared")));
                    }
                    let actual = self.check_expr(init)?;
                    let declared = ty
                        .as_deref()
                        .map(Type::from_annotation)
                        .unwrap_or_else(|| actual.clone());
                    let declared = if declared == Type::Any {
                        actual.clone()
                    } else if actual == Type::Array && declared == Type::Int {
                        Type::Array
                    } else {
                        self.require_compatible(
                            &declared,
                            &actual,
                            &format!("initializer for `{name}`"),
                        )?;
                        declared
                    };
                    self.locals.insert(name.clone(), declared);
                    last_type = Type::Unit;
                }
                Stmt::Assign { name, expr } | Stmt::Walrus { name, expr } => {
                    let expected = self.locals.get(name).cloned().ok_or_else(|| {
                        self.error(format!("assignment to undeclared variable `{name}`"))
                    })?;
                    let actual = self.check_expr(expr)?;
                    self.require_compatible(
                        &expected,
                        &actual,
                        &format!("assignment to `{name}`"),
                    )?;
                    last_type = actual;
                }
                Stmt::IndexAssign { name, index, expr } => {
                    let target = self.locals.get(name).cloned().ok_or_else(|| {
                        self.error(format!("assignment to undeclared variable `{name}`"))
                    })?;
                    if target != Type::Array && target != Type::Any {
                        return Err(self.error(format!("`{name}` is not an array")));
                    }
                    let index_type = self.check_expr(index)?;
                    self.require_compatible(&Type::Int, &index_type, "array index")?;
                    self.check_expr(expr)?;
                    last_type = Type::Unit;
                }
                Stmt::FieldAssign {
                    name,
                    field: _,
                    expr,
                } => {
                    let target = self.locals.get(name).cloned().ok_or_else(|| {
                        self.error(format!("assignment to undeclared variable `{name}`"))
                    })?;
                    if !matches!(target, Type::Any | Type::Struct(_)) {
                        return Err(self.error(format!("`{name}` is not a struct")));
                    }
                    self.check_expr(expr)?;
                    last_type = Type::Unit;
                }
                Stmt::Ret(expr) => {
                    let actual = match expr {
                        Some(expr) => self.check_expr(expr)?,
                        None => Type::Unit,
                    };
                    self.require_compatible(&self.return_type, &actual, "return value")?;
                    last_type = actual;
                }
                Stmt::Expr(expr) => last_type = self.check_expr(expr)?,
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    self.check_condition(cond)?;
                    self.check_block(then_body)?;
                    self.check_block(else_body)?;
                    last_type = Type::Unit;
                }
                Stmt::While { cond, body } => {
                    self.check_condition(cond)?;
                    self.check_block(body)?;
                    last_type = Type::Unit;
                }
            }
        }
        Ok(last_type)
    }

    fn check_condition(&mut self, expr: &Expr) -> Result<(), String> {
        let actual = self.check_expr(expr)?;
        if matches!(actual, Type::Unit) {
            return Err(self.error("condition cannot be a unit value"));
        }
        Ok(())
    }

    fn check_expr(&mut self, expr: &Expr) -> Result<Type, String> {
        match expr {
            Expr::Int(_) => Ok(Type::Int),
            Expr::Float(_) => Ok(Type::Float),
            Expr::String(_) => Ok(Type::String),
            Expr::Bool(_) => Ok(Type::Bool),
            Expr::Ident(name) => self
                .locals
                .get(name)
                .cloned()
                .ok_or_else(|| self.error(format!("use of undeclared variable `{name}`"))),
            Expr::Unary(op, inner) => {
                let inner = self.check_expr(inner)?;
                if op == "not" {
                    if inner == Type::Unit {
                        return Err(self.error("operand of `not` cannot be unit"));
                    }
                    Ok(Type::Bool)
                } else {
                    if !is_numeric(&inner) {
                        return Err(self.error("unary `-` requires a number"));
                    }
                    Ok(inner)
                }
            }
            Expr::Binary(op, lhs, rhs) => {
                let lhs = self.check_expr(lhs)?;
                let rhs = self.check_expr(rhs)?;
                match op.as_str() {
                    "and" | "or" => {
                        self.require_compatible(&Type::Bool, &lhs, "logical operand")?;
                        self.require_compatible(&Type::Bool, &rhs, "logical operand")?;
                        Ok(Type::Bool)
                    }
                    "==" | "!=" | "is" => Ok(Type::Bool),
                    "<" | "<=" | ">" | ">=" => {
                        if !is_numeric(&lhs) || !is_numeric(&rhs) {
                            return Err(self.error("ordered comparison requires numbers"));
                        }
                        Ok(Type::Bool)
                    }
                    "+" if lhs == Type::String && rhs == Type::String => Ok(Type::String),
                    "+" if lhs == Type::Array && rhs == Type::Array => Ok(Type::Array),
                    "+" | "-" | "*" | "/" | "%" => {
                        if !is_numeric(&lhs) || !is_numeric(&rhs) {
                            return Err(self.error(format!("operator `{op}` requires numbers")));
                        }
                        Ok(common_numeric_type(&lhs, &rhs))
                    }
                    _ => Err(self.error(format!("unsupported operator `{op}`"))),
                }
            }
            Expr::Call(name, args) => {
                let argument_types = args
                    .iter()
                    .map(|arg| self.check_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(Type::Function(params, result)) = self.locals.get(name).cloned() {
                    if argument_types.len() != params.len() {
                        return Err(self.error(format!(
                            "closure `{name}` expects {} arguments, received {}",
                            params.len(),
                            argument_types.len()
                        )));
                    }
                    for (expected, actual) in params.iter().zip(argument_types) {
                        self.require_compatible(expected, &actual, "closure argument")?;
                    }
                    Ok(*result)
                } else if let Some(signature) = self.signatures.get(name) {
                    if argument_types.len() != signature.params.len() {
                        return Err(self.error(format!(
                            "function `{name}` expects {} arguments, received {}",
                            signature.params.len(),
                            argument_types.len()
                        )));
                    }
                    for (expected, actual) in signature.params.iter().zip(argument_types) {
                        self.require_compatible(
                            expected,
                            &actual,
                            &format!("argument to `{name}`"),
                        )?;
                    }
                    Ok(signature.result.clone())
                } else {
                    check_builtin(name, &argument_types).map_err(|message| self.error(message))
                }
            }
            Expr::Apply(callee, args) => {
                let callee_type = self.check_expr(callee)?;
                let argument_types = args
                    .iter()
                    .map(|arg| self.check_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                match callee_type {
                    Type::Function(params, result) => {
                        if argument_types.len() != params.len() {
                            return Err(self.error(format!(
                                "closure expects {} arguments, received {}",
                                params.len(),
                                argument_types.len()
                            )));
                        }
                        for (expected, actual) in params.iter().zip(argument_types) {
                            self.require_compatible(expected, &actual, "closure argument")?;
                        }
                        Ok(*result)
                    }
                    Type::Any => Ok(Type::Any),
                    _ => Err(self.error("value is not callable")),
                }
            }
            Expr::Lambda(params, return_annotation, body) => {
                let original_locals = self.locals.clone();
                let parameter_types = params
                    .iter()
                    .map(|param| {
                        param
                            .ty
                            .as_deref()
                            .map(Type::from_annotation)
                            .unwrap_or(Type::Any)
                    })
                    .collect::<Vec<_>>();
                for (param, ty) in params.iter().zip(&parameter_types) {
                    self.locals.insert(param.name.clone(), ty.clone());
                }
                let result = self.check_expr(body)?;
                self.locals = original_locals;
                let result = if let Some(annotation) = return_annotation {
                    let expected = Type::from_annotation(annotation);
                    self.require_compatible(&expected, &result, "lambda return value")?;
                    expected
                } else {
                    result
                };
                Ok(Type::Function(parameter_types, Box::new(result)))
            }
            Expr::MethodCall(receiver, method, args) => {
                let receiver_type = self.check_expr(receiver)?;
                let argument_types = args
                    .iter()
                    .map(|arg| self.check_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Type::Struct(name) = receiver_type {
                    let function_name = format!("{name}.{method}");
                    let signature = self.signatures.get(&function_name).ok_or_else(|| {
                        self.error(format!("undefined method `{method}` for `{name}`"))
                    })?;
                    let mut all_arguments = vec![Type::Struct(name)];
                    all_arguments.extend(argument_types);
                    if all_arguments.len() != signature.params.len() {
                        return Err(self.error(format!(
                            "method `{method}` expects {} arguments, received {}",
                            signature.params.len().saturating_sub(1),
                            all_arguments.len().saturating_sub(1)
                        )));
                    }
                    for (expected, actual) in signature.params.iter().zip(all_arguments) {
                        self.require_compatible(
                            expected,
                            &actual,
                            &format!("argument to method `{method}`"),
                        )?;
                    }
                    Ok(signature.result.clone())
                } else {
                    Ok(Type::Any)
                }
            }
            Expr::Field(receiver, _) => {
                let receiver_type = self.check_expr(receiver)?;
                if !matches!(receiver_type, Type::Struct(_) | Type::Any) {
                    return Err(self.error("field access requires a struct value"));
                }
                Ok(Type::Any)
            }
            Expr::StructLiteral(name, fields) => {
                for (_, expr) in fields {
                    self.check_expr(expr)?;
                }
                Ok(Type::Struct(name.clone()))
            }
            Expr::Match(value, arms) => {
                let value_type = self.check_expr(value)?;
                let mut result_type: Option<Type> = None;
                for (pattern, body) in arms {
                    let original_locals = self.locals.clone();
                    match pattern {
                        Pattern::Wildcard => {}
                        Pattern::Bind(name) => {
                            self.locals.insert(name.clone(), value_type.clone());
                        }
                        Pattern::Literal(literal) => {
                            let literal_type = self.check_expr(literal)?;
                            self.require_compatible(&value_type, &literal_type, "match pattern")?;
                        }
                    }
                    let arm_type = self.check_block(body)?;
                    self.locals = original_locals;
                    if let Some(expected) = &result_type {
                        self.require_compatible(expected, &arm_type, "match arms")?;
                    } else {
                        result_type = Some(arm_type);
                    }
                }
                Ok(result_type.unwrap_or(Type::Unit))
            }
            Expr::Index(array, index) => {
                let array_type = self.check_expr(array)?;
                let index_type = self.check_expr(index)?;
                self.require_compatible(&Type::Int, &index_type, "array index")?;
                match array_type {
                    Type::Array | Type::Any => Ok(Type::Any),
                    Type::String => Ok(Type::String),
                    _ => Err(self.error("indexing requires an array or string")),
                }
            }
            Expr::Array(items) => {
                let mut element_type = Type::Any;
                for item in items {
                    let item_type = self.check_expr(item)?;
                    if element_type == Type::Any {
                        element_type = item_type;
                    } else {
                        self.require_compatible(&element_type, &item_type, "array element")?;
                    }
                }
                Ok(Type::Array)
            }
            Expr::Walrus(name, expr) => {
                let expected = self.locals.get(name).cloned().ok_or_else(|| {
                    self.error(format!("assignment to undeclared variable `{name}`"))
                })?;
                let actual = self.check_expr(expr)?;
                self.require_compatible(&expected, &actual, &format!("assignment to `{name}`"))?;
                Ok(actual)
            }
            Expr::If(cond, then_body, else_body) => {
                self.check_condition(cond)?;
                let then_type = self.check_block(then_body)?;
                let else_type = self.check_block(else_body)?;
                if !then_type.compatible_with(&else_type) {
                    return Err(self.error(format!(
                        "if expression branches have incompatible types: {} and {}",
                        then_type.name(),
                        else_type.name()
                    )));
                }
                Ok(then_type)
            }
            Expr::While(cond, body) => {
                self.check_condition(cond)?;
                self.check_block(body)
            }
        }
    }

    fn require_compatible(
        &self,
        expected: &Type,
        actual: &Type,
        context: &str,
    ) -> Result<(), String> {
        if expected.compatible_with(actual) {
            Ok(())
        } else {
            Err(self.error(format!(
                "{context} expects {}, got {}",
                expected.name(),
                actual.name()
            )))
        }
    }

    fn error(&self, message: impl Into<String>) -> String {
        format!("{}: {}", self.function, message.into())
    }
}

fn check_builtin(name: &str, args: &[Type]) -> Result<Type, String> {
    match name {
        "print" => {
            require_builtin_arity(name, args, 1)?;
            Ok(Type::Unit)
        }
        "is_alpha"
        | "is_digit"
        | "is_alphanumeric"
        | "is_whitespace"
        | "is_newline"
        | "is_identifier_start"
        | "is_identifier_continue" => {
            require_builtin_arity(name, args, 1)?;
            if !matches!(args[0], Type::String | Type::Any) {
                return Err(format!("builtin `{name}` expects a string"));
            }
            Ok(Type::Bool)
        }
        "len" => {
            require_builtin_arity(name, args, 1)?;
            if !matches!(args[0], Type::String | Type::Array | Type::Any) {
                return Err("len expects a string or array".into());
            }
            Ok(Type::Int)
        }
        "now_ms" | "mono_ms" => {
            require_builtin_arity(name, args, 0)?;
            Ok(Type::Int)
        }
        "u8" | "u32" => {
            require_builtin_arity(name, args, 1)?;
            if !args[0].is_integer() && args[0] != Type::Any {
                return Err(format!("builtin `{name}` expects an integer"));
            }
            Ok(if name == "u8" { Type::U8 } else { Type::U32 })
        }
        "buf_new" => {
            require_builtin_arity(name, args, 1)?;
            if !args[0].is_integer() && args[0] != Type::Any {
                return Err("buf_new expects an integer length".into());
            }
            Ok(Type::Any)
        }
        "buf_len" => {
            require_builtin_arity(name, args, 1)?;
            Ok(Type::Int)
        }
        "buf_get_u8" | "buf_get_u32" => {
            require_builtin_arity(name, args, 2)?;
            Ok(if name == "buf_get_u8" {
                Type::U8
            } else {
                Type::U32
            })
        }
        "buf_set_u8" | "buf_set_u32" => {
            require_builtin_arity(name, args, 3)?;
            Ok(Type::Unit)
        }
        "memcpy" | "memmove" => {
            if args.len() != 3 && args.len() != 5 {
                return Err(format!(
                    "builtin `{name}` expects 3 or 5 arguments, received {}",
                    args.len()
                ));
            }
            Ok(Type::Unit)
        }
        "str_to_buf" => {
            require_builtin_arity(name, args, 1)?;
            if !matches!(args[0], Type::String | Type::Any) {
                return Err("str_to_buf expects a string".into());
            }
            Ok(Type::Any)
        }
        "buf_to_str" => {
            require_builtin_arity(name, args, 1)?;
            Ok(Type::String)
        }
        "ffi_unsafe_load" => {
            require_builtin_arity(name, args, 1)?;
            if !matches!(args[0], Type::String | Type::Any) {
                return Err("ffi_unsafe_load expects a library path string".into());
            }
            Ok(Type::Any)
        }
        "ffi_symbol" => {
            require_builtin_arity(name, args, 2)?;
            if !matches!(args[1], Type::String | Type::Any) {
                return Err("ffi_symbol expects a symbol name string".into());
            }
            Ok(Type::Any)
        }
        "ffi_unsafe_call" => {
            require_builtin_arity(name, args, 3)?;
            if !matches!(args[1], Type::String | Type::Any)
                || !matches!(args[2], Type::Array | Type::Any)
            {
                return Err("ffi_unsafe_call expects a signature string and argument array".into());
            }
            Ok(Type::Any)
        }
        "ffi_callback" => {
            require_builtin_arity(name, args, 2)?;
            if !matches!(args[0], Type::String | Type::Any)
                || !matches!(args[1], Type::Function(_, _) | Type::Any)
            {
                return Err("ffi_callback expects a signature string and function".into());
            }
            Ok(Type::Any)
        }
        "ffi_export_ptr" => {
            require_builtin_arity(name, args, 2)?;
            if !matches!(args[0], Type::String | Type::Any)
                || !matches!(args[1], Type::String | Type::Any)
            {
                return Err("ffi_export_ptr expects a function name and signature".into());
            }
            Ok(Type::Any)
        }
        "ffi_callback_error" => {
            require_builtin_arity(name, args, 1)?;
            Ok(Type::String)
        }
        "ffi_null" => {
            require_builtin_arity(name, args, 0)?;
            Ok(Type::Any)
        }
        "ffi_buffer" => {
            require_builtin_arity(name, args, 2)?;
            if (!args[0].is_integer() && args[0] != Type::Any)
                || (!args[1].is_integer() && args[1] != Type::Any)
            {
                return Err("ffi_buffer expects integer size and alignment".into());
            }
            Ok(Type::Any)
        }
        "ffi_buffer_read" => {
            require_builtin_arity(name, args, 3)?;
            Ok(Type::Any)
        }
        "ffi_buffer_write" => {
            require_builtin_arity(name, args, 4)?;
            Ok(Type::Unit)
        }
        "ffi_buffer_write_cstr" => {
            require_builtin_arity(name, args, 3)?;
            Ok(Type::Unit)
        }
        "ffi_buffer_read_cstr" => {
            require_builtin_arity(name, args, 3)?;
            Ok(Type::String)
        }
        "ffi_struct" => {
            require_builtin_arity(name, args, 2)?;
            if !matches!(args[0], Type::String | Type::Any)
                || !matches!(args[1], Type::Array | Type::Any)
            {
                return Err("ffi_struct expects a type string and field array".into());
            }
            Ok(Type::Any)
        }
        "ffi_struct_get" => {
            require_builtin_arity(name, args, 2)?;
            if !args[1].is_integer() && args[1] != Type::Any {
                return Err("ffi_struct_get expects an integer field index".into());
            }
            Ok(Type::Any)
        }
        "log_at" => {
            require_builtin_arity(name, args, 2)?;
            Ok(Type::Unit)
        }
        "trace" | "debug" | "info" | "warn" | "error" | "log_trace" | "log_debug" | "log_info"
        | "log_warn" | "log_error" => {
            require_builtin_arity(name, args, 1)?;
            Ok(Type::Unit)
        }
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "asinh"
        | "acosh" | "atanh" | "exp" | "exp2" | "expm1" | "ln" | "log" | "log2" | "log10"
        | "log1p" | "sqrt" | "cbrt" | "floor" | "ceil" | "round" | "trunc" | "abs"
        | "sleef_sin_u10" | "sleef_sin_u35" | "sleef_cos_u10" | "sleef_cos_u35"
        | "sleef_tan_u10" | "sleef_exp_u10" | "sleef_log_u10" | "sleef_sqrt_u05" | "llvm_sin"
        | "llvm_cos" | "llvm_exp" | "llvm_log" | "llvm_sqrt" => {
            require_builtin_arity(name, args, 1)?;
            Ok(if name == "abs" && args[0].is_integer() {
                args[0].clone()
            } else {
                Type::Float
            })
        }
        "pow" | "atan2" | "hypot" | "fmod" | "remainder" | "copysign" | "sleef_pow_u10"
        | "sleef_pow_u35" => {
            require_builtin_arity(name, args, 2)?;
            Ok(Type::Float)
        }
        _ => Err(format!("call to undefined function `{name}`")),
    }
}

fn is_numeric(ty: &Type) -> bool {
    ty.is_integer() || matches!(ty, Type::Float | Type::Any)
}

fn require_builtin_arity(name: &str, args: &[Type], expected: usize) -> Result<(), String> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "builtin `{name}` expects {expected} arguments, received {}",
            args.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_source;

    #[test]
    fn checks_time_builtins() {
        let funcs = parse_source(
            r#"
fn main() -> int
    wall:int = now_ms()
    monotonic:int = mono_ms()
ret wall + monotonic
"#,
        )
        .unwrap();
        assert!(check_program(&funcs).is_ok());

        let invalid = parse_source(
            r#"
fn main()
    mono_ms(1)
end
"#,
        )
        .unwrap();
        assert!(
            check_program(&invalid)
                .unwrap_err()
                .contains("expects 0 arguments")
        );
    }

    #[test]
    fn checks_fixed_width_and_buffer_builtins() {
        let funcs = parse_source(
            r#"
fn main() -> int
    data:any = buf_new(8)
    buf_set_u8(data, 0, u8(255))
    buf_set_u32(data, 4, u32(1024))
    copied:any = buf_new(8)
    memcpy(copied, 0, data, 0, 8)
ret buf_get_u32(copied, 4)
"#,
        )
        .unwrap();
        assert!(check_program(&funcs).is_ok());

        let invalid = parse_source(
            r#"
fn main()
    memcpy(buf_new(1))
end
"#,
        )
        .unwrap();
        assert!(
            check_program(&invalid)
                .unwrap_err()
                .contains("expects 3 or 5 arguments")
        );
    }

    #[test]
    fn distinguishes_fixed_width_integer_annotations() {
        let valid = parse_source(
            r#"
fn identity(value:u8) -> u8
ret value
fn main() -> int
    small:u8 = u8(255)
    widened:int = small
    copied:u8 = identity(small)
ret widened + copied
"#,
        )
        .unwrap();
        assert!(check_program(&valid).is_ok());

        let invalid = parse_source(
            r#"
fn main() -> int
    small:u8 = 300
ret small
"#,
        )
        .unwrap();
        assert!(
            check_program(&invalid)
                .unwrap_err()
                .contains("expects u8, got int")
        );

        let narrowed = parse_source(
            r#"
fn main() -> int
    small:u8 = u32(1)
ret small
"#,
        )
        .unwrap();
        assert!(
            check_program(&narrowed)
                .unwrap_err()
                .contains("expects u8, got u32")
        );
    }

    #[test]
    fn catches_initializer_and_call_type_errors() {
        let funcs = parse_source(
            r#"
fn takes_int(value:int) -> int
ret value
fn main() -> int
    result:int = takes_int("wrong")
ret result
"#,
        )
        .unwrap();
        let error = check_program(&funcs).unwrap_err();
        assert!(error.contains("argument to `takes_int` expects int, got string"));
    }

    #[test]
    fn checks_builtin_arities_and_allows_truthy_conditions() {
        let funcs = parse_source(
            r#"
fn main() -> int
    count:int = 1
    if count := count + 1
        log_info("incremented")
    end
ret count
"#,
        )
        .unwrap();
        assert!(check_program(&funcs).is_ok());

        let invalid = parse_source(
            r#"
fn main()
    print(1, 2)
end
"#,
        )
        .unwrap();
        assert!(
            check_program(&invalid)
                .unwrap_err()
                .contains("expects 1 arguments")
        );
    }
}
