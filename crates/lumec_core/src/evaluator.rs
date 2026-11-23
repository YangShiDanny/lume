use crate::parser::{Expr, FuncDef, Param, Pattern, Stmt};
#[cfg(feature = "system-sleef")]
use crate::sleef;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Struct {
        name: String,
        fields: HashMap<String, Value>,
    },
    Closure {
        params: Vec<Param>,
        body: Expr,
        captured: HashMap<String, Value>,
    },
    Unit,
}

fn assign_field(
    env: &mut HashMap<String, Value>,
    name: &str,
    field: &str,
    value: Value,
) -> Result<(), String> {
    match env.get_mut(name) {
        Some(Value::Struct { fields, .. }) => {
            let slot = fields
                .get_mut(field)
                .ok_or_else(|| format!("struct has no field `{field}`"))?;
            *slot = value;
            Ok(())
        }
        Some(_) => Err(format!("`{name}` is not a struct")),
        None => Err(format!("undefined variable `{name}`")),
    }
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Value::Int(v) => *v != 0,
            Value::Float(v) => *v != 0.0 && !v.is_nan(),
            Value::Bool(v) => *v,
            Value::String(v) => !v.is_empty(),
            Value::Array(v) => !v.is_empty(),
            Value::Struct { .. } => true,
            Value::Closure { .. } => true,
            Value::Unit => false,
        }
    }
}

#[derive(Debug)]
pub enum EvalResult {
    Normal,
    Return(Value),
}

pub struct Evaluator {
    funcs: HashMap<String, FuncDef>,
}

impl Evaluator {
    pub fn new(funcs: Vec<FuncDef>) -> Self {
        let funcs = funcs.into_iter().map(|f| (f.name.clone(), f)).collect();
        Self { funcs }
    }

    pub fn call(&self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        let func = self
            .funcs
            .get(name)
            .ok_or_else(|| format!("undefined function `{name}`"))?;
        if args.len() != func.params.len() {
            return Err(format!(
                "function `{name}` expects {} arguments, received {}",
                func.params.len(),
                args.len()
            ));
        }
        let mut env = HashMap::new();
        for (param, value) in func.params.iter().zip(args) {
            env.insert(param.name.clone(), value);
        }
        match self.exec_block(&func.body, &mut env)? {
            EvalResult::Normal => Ok(Value::Int(0)),
            EvalResult::Return(value) => Ok(value),
        }
    }

    pub fn has_function(&self, name: &str) -> bool {
        self.funcs.contains_key(name)
    }

    fn invoke_closure(
        &self,
        params: &[Param],
        body: &Expr,
        captured: &HashMap<String, Value>,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        if args.len() != params.len() {
            return Err(format!(
                "closure expects {} arguments, received {}",
                params.len(),
                args.len()
            ));
        }
        let mut env = captured.clone();
        for (param, value) in params.iter().zip(args) {
            env.insert(param.name.clone(), value);
        }
        self.eval(body, &mut env)
    }

    fn exec_block(
        &self,
        block: &[Stmt],
        env: &mut HashMap<String, Value>,
    ) -> Result<EvalResult, String> {
        for stmt in block {
            match stmt {
                Stmt::VarDecl { name, init, .. } => {
                    let value = self.eval(init, env)?;
                    env.insert(name.clone(), value);
                }
                Stmt::Assign { name, expr } => {
                    if !env.contains_key(name) {
                        return Err(format!("cannot assign undefined variable `{name}`"));
                    }
                    let value = self.eval(expr, env)?;
                    env.insert(name.clone(), value);
                }
                Stmt::IndexAssign { name, index, expr } => {
                    let index = self.eval(index, env)?;
                    let value = self.eval(expr, env)?;
                    assign_index(env, name, index, value)?;
                }
                Stmt::FieldAssign { name, field, expr } => {
                    let value = self.eval(expr, env)?;
                    assign_field(env, name, field, value)?;
                }
                Stmt::Walrus { name, expr } => {
                    if !env.contains_key(name) {
                        return Err(format!(
                            "walrus assignment requires existing variable `{name}`"
                        ));
                    }
                    let value = self.eval(expr, env)?;
                    env.insert(name.clone(), value);
                }
                Stmt::Ret(expr) => {
                    let value = match expr {
                        Some(expr) => self.eval(expr, env)?,
                        None => Value::Int(0),
                    };
                    return Ok(EvalResult::Return(value));
                }
                Stmt::Expr(expr) => {
                    self.eval(expr, env)?;
                }
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    let cond_value = self.eval(cond, env)?.truthy();
                    let result = if cond_value {
                        self.exec_block(then_body, env)?
                    } else {
                        self.exec_block(else_body, env)?
                    };
                    if matches!(result, EvalResult::Return(_)) {
                        return Ok(result);
                    }
                }
                Stmt::While { cond, body } => {
                    let mut guard = 0usize;
                    while self.eval(cond, env)?.truthy() {
                        guard += 1;
                        if guard > 100_000 {
                            return Err("while loop exceeded 100000 iterations".into());
                        }
                        let result = self.exec_block(body, env)?;
                        if matches!(result, EvalResult::Return(_)) {
                            return Ok(result);
                        }
                    }
                }
            }
        }
        Ok(EvalResult::Normal)
    }

    fn eval_block_value(
        &self,
        block: &[Stmt],
        env: &mut HashMap<String, Value>,
    ) -> Result<Value, String> {
        let mut last = Value::Int(0);
        for stmt in block {
            match stmt {
                Stmt::VarDecl { name, init, .. } => {
                    let value = self.eval(init, env)?;
                    env.insert(name.clone(), value);
                }
                Stmt::Assign { name, expr } => {
                    if !env.contains_key(name) {
                        return Err(format!("cannot assign undefined variable `{name}`"));
                    }
                    let value = self.eval(expr, env)?;
                    env.insert(name.clone(), value);
                }
                Stmt::IndexAssign { name, index, expr } => {
                    let index = self.eval(index, env)?;
                    let value = self.eval(expr, env)?;
                    assign_index(env, name, index, value)?;
                }
                Stmt::FieldAssign { name, field, expr } => {
                    let value = self.eval(expr, env)?;
                    assign_field(env, name, field, value)?;
                }
                Stmt::Walrus { name, expr } => {
                    let value = self.eval(expr, env)?;
                    env.insert(name.clone(), value.clone());
                    last = value;
                }
                Stmt::Ret(expr) => {
                    return match expr {
                        Some(expr) => self.eval(expr, env),
                        None => Ok(Value::Int(0)),
                    };
                }
                Stmt::Expr(expr) => last = self.eval(expr, env)?,
                Stmt::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    let cond = self.eval(cond, env)?.truthy();
                    let chosen = if cond { then_body } else { else_body };
                    last = self.eval_block_value(chosen, env)?;
                }
                Stmt::While { cond, body } => {
                    let mut guard = 0usize;
                    while self.eval(cond, env)?.truthy() {
                        guard += 1;
                        if guard > 100_000 {
                            return Err("while block expression exceeded 100000 iterations".into());
                        }
                        last = self.eval_block_value(body, env)?;
                    }
                }
            }
        }
        Ok(last)
    }

    fn eval(&self, expr: &Expr, env: &mut HashMap<String, Value>) -> Result<Value, String> {
        match expr {
            Expr::Int(v) => Ok(Value::Int(*v)),
            Expr::Float(v) => Ok(Value::Float(*v)),
            Expr::String(v) => Ok(Value::String(v.clone())),
            Expr::Bool(v) => Ok(Value::Bool(*v)),
            Expr::Ident(name) => env
                .get(name)
                .cloned()
                .ok_or_else(|| format!("undefined variable `{name}`")),
            Expr::Unary(op, inner) => {
                let value = self.eval(inner, env)?;
                match (op.as_str(), value) {
                    ("-", Value::Int(v)) => Ok(Value::Int(-v)),
                    ("-", Value::Float(v)) => Ok(Value::Float(-v)),
                    ("not", value) => Ok(Value::Bool(!value.truthy())),
                    _ => Err("invalid unary operation".into()),
                }
            }
            Expr::Binary(op, lhs, rhs) => {
                let lhs = self.eval(lhs, env)?;
                if op == "and" && !lhs.truthy() {
                    return Ok(Value::Bool(false));
                }
                if op == "or" && lhs.truthy() {
                    return Ok(Value::Bool(true));
                }
                let rhs = self.eval(rhs, env)?;
                binary(op, lhs, rhs)
            }
            Expr::Call(name, args) => {
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.eval(arg, env)?);
                }
                if let Some(Value::Closure {
                    params,
                    body,
                    captured,
                }) = env.get(name).cloned()
                {
                    return self.invoke_closure(&params, &body, &captured, values);
                }
                if let Some(value) = builtin(name, &values)? {
                    return Ok(value);
                }
                self.call(name, values)
            }
            Expr::Apply(callee, args) => {
                let callee = self.eval(callee, env)?;
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.eval(arg, env)?);
                }
                match callee {
                    Value::Closure {
                        params,
                        body,
                        captured,
                    } => self.invoke_closure(&params, &body, &captured, values),
                    _ => Err("value is not callable".into()),
                }
            }
            Expr::Lambda(params, _, body) => Ok(Value::Closure {
                params: params.clone(),
                body: (**body).clone(),
                captured: env.clone(),
            }),
            Expr::MethodCall(receiver, method, args) => {
                let receiver = self.eval(receiver, env)?;
                let type_name = match &receiver {
                    Value::Struct { name, .. } => name,
                    _ => return Err("method receiver must be a struct value".into()),
                };
                let function = format!("{type_name}.{method}");
                let mut values = vec![receiver];
                for arg in args {
                    values.push(self.eval(arg, env)?);
                }
                self.call(&function, values)
            }
            Expr::Field(receiver, field) => match self.eval(receiver, env)? {
                Value::Struct { fields, .. } => fields
                    .get(field)
                    .cloned()
                    .ok_or_else(|| format!("struct has no field `{field}`")),
                _ => Err("field access requires a struct value".into()),
            },
            Expr::StructLiteral(name, fields) => {
                let mut values = HashMap::new();
                for (field, expr) in fields {
                    values.insert(field.clone(), self.eval(expr, env)?);
                }
                Ok(Value::Struct {
                    name: name.clone(),
                    fields: values,
                })
            }
            Expr::Index(base, index) => {
                let base = self.eval(base, env)?;
                let index = self.eval(index, env)?;
                match (base, index) {
                    (Value::Array(items), Value::Int(index)) => {
                        let index = usize::try_from(index).map_err(|_| "negative array index")?;
                        items
                            .get(index)
                            .cloned()
                            .ok_or_else(|| "array index out of bounds".into())
                    }
                    _ => Err("index is only supported for integer arrays".into()),
                }
            }
            Expr::Array(items) => {
                let mut values = Vec::new();
                for item in items {
                    values.push(self.eval(item, env)?);
                }
                Ok(Value::Array(values))
            }
            Expr::Walrus(name, expr) => {
                if !env.contains_key(name) {
                    return Err(format!(
                        "walrus assignment requires existing variable `{name}`"
                    ));
                }
                let value = self.eval(expr, env)?;
                env.insert(name.clone(), value.clone());
                Ok(value)
            }
            Expr::If(cond, then_body, else_body) => {
                let cond = self.eval(cond, env)?.truthy();
                let block = if cond { then_body } else { else_body };
                self.eval_block_value(block, env)
            }
            Expr::While(cond, body) => {
                let mut last = Value::Int(0);
                let mut guard = 0usize;
                while self.eval(cond, env)?.truthy() {
                    guard += 1;
                    if guard > 100_000 {
                        return Err("while expression exceeded 100000 iterations".into());
                    }
                    last = self.eval_block_value(body, env)?;
                }
                Ok(last)
            }
            Expr::Match(value, arms) => {
                let value = self.eval(value, env)?;
                for (pattern, body) in arms {
                    let binding = match pattern {
                        Pattern::Wildcard => None,
                        Pattern::Bind(name) => Some((name.as_str(), value.clone())),
                        Pattern::Literal(literal) => {
                            let literal = self.eval(literal, env)?;
                            if !values_equal(&value, &literal) {
                                continue;
                            }
                            None
                        }
                    };
                    let previous = binding
                        .as_ref()
                        .map(|(name, _)| ((*name).to_string(), env.get(*name).cloned()));
                    if let Some((name, bound_value)) = binding {
                        env.insert(name.to_string(), bound_value);
                    }
                    let result = self.eval_block_value(body, env);
                    if let Some((name, previous_value)) = previous {
                        if let Some(previous_value) = previous_value {
                            env.insert(name, previous_value);
                        } else {
                            env.remove(&name);
                        }
                    }
                    return result;
                }
                Err("non-exhaustive match: no pattern matched the value".into())
            }
        }
    }
}

fn binary(op: &str, lhs: Value, rhs: Value) -> Result<Value, String> {
    match op {
        "==" => Ok(Value::Bool(values_equal(&lhs, &rhs))),
        "!=" => Ok(Value::Bool(!values_equal(&lhs, &rhs))),
        "is" => Ok(Value::Bool(values_equal(&lhs, &rhs))),
        "+" | "-" | "*" | "/" | "%" | "<" | "<=" | ">" | ">=" => {
            if matches!(&lhs, Value::Float(_)) || matches!(&rhs, Value::Float(_)) {
                let lhs = value_float(&lhs, op)?;
                let rhs = value_float(&rhs, op)?;
                return Ok(match op {
                    "+" => Value::Float(lhs + rhs),
                    "-" => Value::Float(lhs - rhs),
                    "*" => Value::Float(lhs * rhs),
                    "/" => Value::Float(lhs / rhs),
                    "%" => Value::Float(lhs % rhs),
                    "<" => Value::Bool(lhs < rhs),
                    "<=" => Value::Bool(lhs <= rhs),
                    ">" => Value::Bool(lhs > rhs),
                    ">=" => Value::Bool(lhs >= rhs),
                    _ => unreachable!(),
                });
            }
            let lhs = value_int(&lhs, op)?;
            let rhs = value_int(&rhs, op)?;
            Ok(match op {
                "+" => Value::Int(lhs.saturating_add(rhs)),
                "-" => Value::Int(lhs.saturating_sub(rhs)),
                "*" => Value::Int(lhs.saturating_mul(rhs)),
                "/" => {
                    if rhs == 0 {
                        return Err("division by zero".into());
                    }
                    Value::Int(lhs / rhs)
                }
                "%" => {
                    if rhs == 0 {
                        return Err("division by zero".into());
                    }
                    Value::Int(lhs % rhs)
                }
                "<" => Value::Bool(lhs < rhs),
                "<=" => Value::Bool(lhs <= rhs),
                ">" => Value::Bool(lhs > rhs),
                ">=" => Value::Bool(lhs >= rhs),
                _ => unreachable!(),
            })
        }
        _ => Err(format!("unsupported operator `{op}`")),
    }
}

fn assign_index(
    env: &mut HashMap<String, Value>,
    name: &str,
    index: Value,
    value: Value,
) -> Result<(), String> {
    let index = match index {
        Value::Int(index) => usize::try_from(index).map_err(|_| "negative array index")?,
        _ => return Err("array index must be an integer".into()),
    };
    match env.get_mut(name) {
        Some(Value::Array(items)) => {
            let len = items.len();
            let slot = items
                .get_mut(index)
                .ok_or_else(|| format!("array index {index} out of bounds for length {len}"))?;
            *slot = value;
            Ok(())
        }
        Some(_) => Err(format!("`{name}` is not an array")),
        None => Err(format!("undefined variable `{name}`")),
    }
}

fn value_int(value: &Value, op: &str) -> Result<i64, String> {
    match value {
        Value::Int(v) => Ok(*v),
        _ => Err(format!("operator `{op}` requires integers")),
    }
}

fn value_float(value: &Value, op: &str) -> Result<f64, String> {
    match value {
        Value::Int(v) => Ok(*v as f64),
        Value::Float(v) => Ok(*v),
        _ => Err(format!("operator `{op}` requires numbers")),
    }
}

fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Unit, Value::Unit) => true,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| values_equal(a, b))
        }
        (
            Value::Struct {
                name: name_a,
                fields: fields_a,
            },
            Value::Struct {
                name: name_b,
                fields: fields_b,
            },
        ) => {
            name_a == name_b
                && fields_a.len() == fields_b.len()
                && fields_a.iter().all(|(field, value)| {
                    fields_b
                        .get(field)
                        .is_some_and(|other| values_equal(value, other))
                })
        }
        _ => false,
    }
}

fn builtin(name: &str, args: &[Value]) -> Result<Option<Value>, String> {
    match name {
        "print" => {
            if args.len() != 1 {
                return Err("print expects 1 argument".into());
            }
            println!("{}", value_to_string(&args[0]));
            Ok(Some(Value::Unit))
        }
        "len" => {
            if args.len() != 1 {
                return Err("len expects 1 argument".into());
            }
            match &args[0] {
                Value::Array(items) => Ok(Some(Value::Int(items.len() as i64))),
                Value::String(text) => Ok(Some(Value::Int(text.len() as i64))),
                _ => Err("len expects string or array".into()),
            }
        }
        "c_matmul" => {
            if args.is_empty() {
                return Err("c_matmul expects at least 1 argument".into());
            }
            let n = value_int(&args[0], "c_matmul")?;
            Ok(Some(Value::Int(n.saturating_mul(2))))
        }
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "sinh" | "cosh" | "tanh" | "asinh"
        | "acosh" | "atanh" | "exp" | "exp2" | "expm1" | "ln" | "log" | "log2" | "log10"
        | "log1p" | "sqrt" | "cbrt" | "floor" | "ceil" | "round" | "trunc" | "abs"
        | "sleef_sin_u10" | "sleef_sin_u35" | "sleef_cos_u10" | "sleef_cos_u35"
        | "sleef_tan_u10" | "sleef_exp_u10" | "sleef_log_u10" | "sleef_sqrt_u05" | "llvm_sin"
        | "llvm_cos" | "llvm_exp" | "llvm_log" | "llvm_sqrt" => {
            require_arity(name, args, 1)?;
            let input = value_float(&args[0], name)?;
            if name == "abs"
                && let Value::Int(value) = &args[0]
            {
                return value
                    .checked_abs()
                    .map(|value| Some(Value::Int(value)))
                    .ok_or_else(|| "integer absolute value overflow".into());
            }
            let output = unary_math(name, input)?;
            Ok(Some(Value::Float(output)))
        }
        "pow" | "atan2" | "hypot" | "fmod" | "remainder" | "copysign" | "sleef_pow_u10"
        | "sleef_pow_u35" => {
            require_arity(name, args, 2)?;
            let lhs = value_float(&args[0], name)?;
            let rhs = value_float(&args[1], name)?;
            Ok(Some(Value::Float(binary_math(name, lhs, rhs))))
        }
        "trace" | "debug" | "info" | "warn" | "error" | "log_trace" | "log_debug" | "log_info"
        | "log_warn" | "log_error" => {
            require_arity(name, args, 1)?;
            let level = name.strip_prefix("log_").unwrap_or(name);
            eprintln!("[{level}] {}", value_to_string(&args[0]));
            Ok(Some(Value::Unit))
        }
        "log_at" => {
            require_arity(name, args, 2)?;
            let level = match &args[0] {
                Value::String(level) => level.as_str(),
                _ => return Err("log_at expects a string level as its first argument".into()),
            };
            if !matches!(level, "trace" | "debug" | "info" | "warn" | "error") {
                return Err(format!("unknown log level `{level}`"));
            }
            eprintln!("[{level}] {}", value_to_string(&args[1]));
            Ok(Some(Value::Unit))
        }
        _ => Ok(None),
    }
}

fn require_arity(name: &str, args: &[Value], expected: usize) -> Result<(), String> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "`{name}` expects {expected} arguments, received {}",
            args.len()
        ))
    }
}

fn unary_math(name: &str, x: f64) -> Result<f64, String> {
    #[cfg(feature = "system-sleef")]
    {
        let value = unsafe {
            match name {
                "sleef_sin_u10" => Some(sleef::sin_u10(x)),
                "sleef_sin_u35" => Some(sleef::sin_u35(x)),
                "sleef_cos_u10" => Some(sleef::cos_u10(x)),
                "sleef_cos_u35" => Some(sleef::cos_u35(x)),
                "sleef_tan_u10" => Some(sleef::tan_u10(x)),
                "sleef_exp_u10" => Some(sleef::exp_u10(x)),
                "sleef_log_u10" => Some(sleef::log_u10(x)),
                "sleef_sqrt_u05" => Some(sleef::sqrt_u05(x)),
                _ => None,
            }
        };
        if let Some(value) = value {
            return Ok(value);
        }
    }
    Ok(match name {
        "sin" | "sleef_sin_u10" | "sleef_sin_u35" | "llvm_sin" => x.sin(),
        "cos" | "sleef_cos_u10" | "sleef_cos_u35" | "llvm_cos" => x.cos(),
        "tan" | "sleef_tan_u10" => x.tan(),
        "asin" => x.asin(),
        "acos" => x.acos(),
        "atan" => x.atan(),
        "sinh" => x.sinh(),
        "cosh" => x.cosh(),
        "tanh" => x.tanh(),
        "asinh" => x.asinh(),
        "acosh" => x.acosh(),
        "atanh" => x.atanh(),
        "exp" | "sleef_exp_u10" | "llvm_exp" => x.exp(),
        "exp2" => x.exp2(),
        "expm1" => x.exp_m1(),
        "ln" | "log" | "sleef_log_u10" | "llvm_log" => x.ln(),
        "log2" => x.log2(),
        "log10" => x.log10(),
        "log1p" => x.ln_1p(),
        "sqrt" | "sleef_sqrt_u05" | "llvm_sqrt" => x.sqrt(),
        "cbrt" => x.cbrt(),
        "floor" => x.floor(),
        "ceil" => x.ceil(),
        "round" => x.round(),
        "trunc" => x.trunc(),
        "abs" => x.abs(),
        _ => return Err(format!("unknown math function `{name}`")),
    })
}

fn binary_math(name: &str, x: f64, y: f64) -> f64 {
    #[cfg(feature = "system-sleef")]
    if matches!(name, "sleef_pow_u10" | "sleef_pow_u35") {
        return unsafe { sleef::pow_u10(x, y) };
    }
    match name {
        "pow" | "sleef_pow_u10" | "sleef_pow_u35" => x.powf(y),
        "atan2" => x.atan2(y),
        "hypot" => x.hypot(y),
        "fmod" => x % y,
        "remainder" => x - (x / y).round_ties_even() * y,
        "copysign" => x.copysign(y),
        _ => f64::NAN,
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Int(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(value_to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Struct { name, fields } => {
            let fields = fields
                .iter()
                .map(|(field, value)| format!("{field}: {}", value_to_string(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{name}{{{fields}}}")
        }
        Value::Closure { .. } => "<closure>".into(),
        Value::Unit => "()".into(),
    }
}
