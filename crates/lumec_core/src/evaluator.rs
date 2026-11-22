use crate::parser::{Expr, FuncDef, Stmt};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Bool(bool),
    String(String),
    Array(Vec<Value>),
    Unit,
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Value::Int(v) => *v != 0,
            Value::Bool(v) => *v,
            Value::String(v) => !v.is_empty(),
            Value::Array(v) => !v.is_empty(),
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
                    ("not", value) => Ok(Value::Bool(!value.truthy())),
                    _ => Err("invalid unary operation".into()),
                }
            }
            Expr::Binary(op, lhs, rhs) => {
                let lhs = self.eval(lhs, env)?;
                let rhs = self.eval(rhs, env)?;
                binary(op, lhs, rhs)
            }
            Expr::Call(name, args) => {
                let mut values = Vec::new();
                for arg in args {
                    values.push(self.eval(arg, env)?);
                }
                if let Some(value) = builtin(name, &values)? {
                    return Ok(value);
                }
                self.call(name, values)
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
                    match self.exec_block(body, env)? {
                        EvalResult::Return(value) => return Ok(value),
                        EvalResult::Normal => {}
                    }
                    last = Value::Int(guard as i64);
                }
                Ok(last)
            }
        }
    }
}

fn binary(op: &str, lhs: Value, rhs: Value) -> Result<Value, String> {
    match op {
        "and" => Ok(Value::Bool(lhs.truthy() && rhs.truthy())),
        "or" => Ok(Value::Bool(lhs.truthy() || rhs.truthy())),
        "==" => Ok(Value::Bool(values_equal(&lhs, &rhs))),
        "!=" => Ok(Value::Bool(!values_equal(&lhs, &rhs))),
        "is" => Ok(Value::Bool(values_equal(&lhs, &rhs))),
        "+" | "-" | "*" | "/" | "%" | "<" | "<=" | ">" | ">=" => {
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

fn value_int(value: &Value, op: &str) -> Result<i64, String> {
    match value {
        Value::Int(v) => Ok(*v),
        _ => Err(format!("operator `{op}` requires integers")),
    }
}

fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Unit, Value::Unit) => true,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| values_equal(a, b))
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
        _ => Ok(None),
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Int(v) => v.to_string(),
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
        Value::Unit => "()".into(),
    }
}
