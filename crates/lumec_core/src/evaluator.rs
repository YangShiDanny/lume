use crate::buffer::{GcBuffer, GcBufferHeap};
use crate::clock;
use crate::ffi::{
    FfiAbiName, FfiSignature, FfiType, FfiValue, ForeignBuffer, ForeignCallback, ForeignLibrary,
    ForeignSymbol, aggregate_field, make_aggregate,
};
use crate::parser::{Expr, FuncDef, Param, Pattern, Stmt};
#[cfg(feature = "system-sleef")]
use crate::sleef;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    UInt(u64),
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
    FfiLibrary(Arc<ForeignLibrary>),
    FfiSymbol(Rc<ForeignSymbol>),
    FfiCallback(Rc<ForeignCallback>),
    FfiBuffer(Rc<ForeignBuffer>),
    FfiAggregate(crate::ffi::AggregateValue),
    FfiPointer(*mut c_void),
    GcBuffer(Rc<GcBuffer>),
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
            Value::UInt(v) => *v != 0,
            Value::Float(v) => *v != 0.0 && !v.is_nan(),
            Value::Bool(v) => *v,
            Value::String(v) => !v.is_empty(),
            Value::Array(v) => !v.is_empty(),
            Value::Struct { .. } => true,
            Value::Closure { .. } => true,
            Value::FfiLibrary(_)
            | Value::FfiSymbol(_)
            | Value::FfiCallback(_)
            | Value::FfiBuffer(_)
            | Value::FfiAggregate(_)
            | Value::FfiPointer(_) => true,
            Value::GcBuffer(buffer) => !buffer.is_empty(),
            Value::Unit => false,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        values_equal(self, other)
    }
}

#[derive(Debug)]
pub enum EvalResult {
    Normal,
    Return(Value),
}

#[derive(Clone)]
pub struct ExternBinding {
    pub signature: FfiSignature,
    pub symbol: Rc<ForeignSymbol>,
}

#[derive(Clone)]
pub struct Evaluator {
    funcs: HashMap<String, FuncDef>,
    heap: GcBufferHeap,
    externs: HashMap<String, ExternBinding>,
}

impl Evaluator {
    pub fn new(funcs: Vec<FuncDef>) -> Self {
        Self::with_externs(funcs, HashMap::new())
    }

    pub fn with_externs(funcs: Vec<FuncDef>, externs: HashMap<String, ExternBinding>) -> Self {
        let funcs = funcs.into_iter().map(|f| (f.name.clone(), f)).collect();
        Self {
            funcs,
            heap: GcBufferHeap::default(),
            externs,
        }
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

    fn call_extern(&self, name: &str, args: &[Value]) -> Result<Option<Value>, String> {
        let Some(binding) = self.externs.get(name) else {
            return Ok(None);
        };
        if args.len() != binding.signature.args.len() {
            return Err(format!(
                "extern `{name}` expects {} arguments, received {}",
                binding.signature.args.len(),
                args.len()
            ));
        }
        let ffi_args = binding
            .signature
            .args
            .iter()
            .cloned()
            .zip(args)
            .map(|(ty, value)| ffi_value_from_lume(ty, value))
            .collect::<Result<Vec<_>, _>>()?;
        let result = unsafe { binding.symbol.call(&binding.signature, &ffi_args) }
            .map_err(|error| error.to_string())?;
        Ok(Some(lume_value_from_ffi(result)))
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

    fn ffi_builtin(&self, name: &str, args: &[Value]) -> Result<Option<Value>, String> {
        match name {
            "ffi_unsafe_load" => {
                require_arity(name, args, 1)?;
                let Value::String(path) = &args[0] else {
                    return Err("ffi_unsafe_load expects a library path string".into());
                };
                let library =
                    unsafe { ForeignLibrary::open(path) }.map_err(|error| error.to_string())?;
                Ok(Some(Value::FfiLibrary(library)))
            }
            "ffi_symbol" => {
                require_arity(name, args, 2)?;
                let (Value::FfiLibrary(library), Value::String(symbol)) = (&args[0], &args[1])
                else {
                    return Err("ffi_symbol expects a library and symbol name".into());
                };
                Ok(Some(Value::FfiSymbol(Rc::new(
                    library.symbol(symbol).map_err(|error| error.to_string())?,
                ))))
            }
            "ffi_unsafe_call" => {
                require_arity(name, args, 3)?;
                let (Value::FfiSymbol(symbol), Value::String(spec), Value::Array(values)) =
                    (&args[0], &args[1], &args[2])
                else {
                    return Err(
                        "ffi_unsafe_call expects a symbol, signature string, and argument array"
                            .into(),
                    );
                };
                let signature = FfiSignature::parse(spec).map_err(|error| error.to_string())?;
                if values.len() != signature.args.len() {
                    return Err(format!(
                        "C function expects {} arguments, received {}",
                        signature.args.len(),
                        values.len()
                    ));
                }
                let ffi_args = signature
                    .args
                    .iter()
                    .cloned()
                    .zip(values)
                    .map(|(ty, value)| ffi_value_from_lume(ty, value))
                    .collect::<Result<Vec<_>, _>>()?;
                let result = unsafe { symbol.call(&signature, &ffi_args) }
                    .map_err(|error| error.to_string())?;
                Ok(Some(lume_value_from_ffi(result)))
            }
            "ffi_callback" => {
                require_arity(name, args, 2)?;
                let (
                    Value::String(spec),
                    Value::Closure {
                        params,
                        body,
                        captured,
                    },
                ) = (&args[0], &args[1])
                else {
                    return Err("ffi_callback expects a signature string and a Lume closure".into());
                };
                let signature = FfiSignature::parse(spec).map_err(|error| error.to_string())?;
                let callback_signature = signature.clone();
                let evaluator = self.clone();
                let params = params.clone();
                let body = body.clone();
                let captured = captured.clone();
                let callback = ForeignCallback::new(signature, move |values| {
                    if values.len() != params.len() {
                        return Err(format!(
                            "Lume callback expects {} arguments, received {}",
                            params.len(),
                            values.len()
                        ));
                    }
                    let lume_args = values.iter().cloned().map(lume_value_from_ffi).collect();
                    let result = evaluator.invoke_closure(&params, &body, &captured, lume_args)?;
                    ffi_value_from_lume(callback_signature.result.clone(), &result)
                })
                .map_err(|error| error.to_string())?;
                Ok(Some(Value::FfiCallback(callback)))
            }
            "ffi_callback_error" => {
                require_arity(name, args, 1)?;
                let Value::FfiCallback(callback) = &args[0] else {
                    return Err("ffi_callback_error expects an FFI callback".into());
                };
                Ok(Some(Value::String(
                    callback
                        .take_error()
                        .map_err(|error| error.to_string())?
                        .unwrap_or_default(),
                )))
            }
            "ffi_null" => {
                require_arity(name, args, 0)?;
                Ok(Some(Value::FfiPointer(std::ptr::null_mut())))
            }
            "ffi_export_ptr" => {
                require_arity(name, args, 2)?;
                let (Value::String(function), Value::String(spec)) = (&args[0], &args[1]) else {
                    return Err("ffi_export_ptr expects a function name and signature".into());
                };
                if !self.has_function(function) {
                    return Err(format!("cannot export undefined function `{function}`"));
                }
                let signature = FfiSignature::parse(spec).map_err(|error| error.to_string())?;
                let result_type = signature.result.clone();
                let evaluator = self.clone();
                let function = function.clone();
                let callback = ForeignCallback::new(signature, move |values| {
                    let lume_args = values.iter().cloned().map(lume_value_from_ffi).collect();
                    let result = evaluator.call(&function, lume_args)?;
                    ffi_value_from_lume(result_type.clone(), &result)
                })
                .map_err(|error| error.to_string())?;
                Ok(Some(Value::FfiCallback(callback)))
            }
            "ffi_buffer" => {
                require_arity(name, args, 2)?;
                let size = usize::try_from(value_int(&args[0], name)?)
                    .map_err(|_| "FFI buffer size must be non-negative")?;
                let alignment = usize::try_from(value_int(&args[1], name)?)
                    .map_err(|_| "FFI buffer alignment must be positive")?;
                let buffer =
                    ForeignBuffer::new(size, alignment).map_err(|error| error.to_string())?;
                Ok(Some(Value::FfiBuffer(buffer)))
            }
            "ffi_buffer_read" => {
                require_arity(name, args, 3)?;
                let (Value::FfiBuffer(buffer), Value::String(type_name)) = (&args[0], &args[2])
                else {
                    return Err("ffi_buffer_read expects a buffer, offset, and type name".into());
                };
                let offset = usize::try_from(value_int(&args[1], name)?)
                    .map_err(|_| "FFI buffer offset must be non-negative")?;
                let ty = FfiType::parse(type_name).map_err(|error| error.to_string())?;
                let value = buffer.read(offset, ty).map_err(|error| error.to_string())?;
                Ok(Some(lume_value_from_ffi(value)))
            }
            "ffi_buffer_write" => {
                require_arity(name, args, 4)?;
                let (Value::FfiBuffer(buffer), Value::String(type_name)) = (&args[0], &args[2])
                else {
                    return Err(
                        "ffi_buffer_write expects a buffer, offset, type name, and value".into(),
                    );
                };
                let offset = usize::try_from(value_int(&args[1], name)?)
                    .map_err(|_| "FFI buffer offset must be non-negative")?;
                let ty = FfiType::parse(type_name).map_err(|error| error.to_string())?;
                let value = ffi_value_from_lume(ty.clone(), &args[3])?;
                buffer
                    .write(offset, ty, &value)
                    .map_err(|error| error.to_string())?;
                Ok(Some(Value::Unit))
            }
            "ffi_buffer_write_cstr" => {
                require_arity(name, args, 3)?;
                let (Value::FfiBuffer(buffer), Value::String(value)) = (&args[0], &args[2]) else {
                    return Err("ffi_buffer_write_cstr expects a buffer, offset, and string".into());
                };
                let offset = usize::try_from(value_int(&args[1], name)?)
                    .map_err(|_| "FFI buffer offset must be non-negative")?;
                buffer
                    .write_c_string(offset, value)
                    .map_err(|error| error.to_string())?;
                Ok(Some(Value::Unit))
            }
            "ffi_buffer_read_cstr" => {
                require_arity(name, args, 3)?;
                let (Value::FfiBuffer(buffer), Value::Int(max_len)) = (&args[0], &args[2]) else {
                    return Err(
                        "ffi_buffer_read_cstr expects a buffer, offset, and max length".into(),
                    );
                };
                let offset = usize::try_from(value_int(&args[1], name)?)
                    .map_err(|_| "FFI buffer offset must be non-negative")?;
                let max_len = usize::try_from(*max_len)
                    .map_err(|_| "FFI string max length must be non-negative")?;
                let value = buffer
                    .read_c_string(offset, max_len)
                    .map_err(|error| error.to_string())?;
                Ok(Some(Value::String(value)))
            }
            "ffi_struct" => {
                require_arity(name, args, 2)?;
                let (Value::String(spec), Value::Array(fields)) = (&args[0], &args[1]) else {
                    return Err("ffi_struct expects an aggregate type and field values".into());
                };
                let ty = FfiType::parse(spec).map_err(|error| error.to_string())?;
                let values = match &ty {
                    FfiType::Struct(field_types) => field_types
                        .iter()
                        .zip(fields)
                        .map(|(field, value)| ffi_value_from_lume(field.clone(), value))
                        .collect::<Result<Vec<_>, _>>()?,
                    FfiType::Union(_) if fields.len() == 1 => {
                        vec![ffi_value_from_lume(
                            infer_ffi_type(&fields[0])?,
                            &fields[0],
                        )?]
                    }
                    FfiType::Union(_) => {
                        return Err("a C union value needs exactly one member value".into());
                    }
                    _ => return Err("ffi_struct expects a struct or union type".into()),
                };
                let aggregate = make_aggregate(ty, &values, FfiAbiName::Default)
                    .map_err(|error| error.to_string())?;
                Ok(Some(Value::FfiAggregate(aggregate)))
            }
            "ffi_struct_get" => {
                require_arity(name, args, 2)?;
                let (Value::FfiAggregate(aggregate), Value::Int(index)) = (&args[0], &args[1])
                else {
                    return Err("ffi_struct_get expects an aggregate and field index".into());
                };
                let index = usize::try_from(*index)
                    .map_err(|_| "ffi_struct_get field index must be non-negative")?;
                let field = aggregate_field(aggregate, index, FfiAbiName::Default)
                    .map_err(|error| error.to_string())?;
                Ok(Some(lume_value_from_ffi(field)))
            }
            _ => Ok(None),
        }
    }

    fn gc_buffer_builtin(&self, name: &str, args: &[Value]) -> Result<Option<Value>, String> {
        match name {
            "buf_new" => {
                require_arity(name, args, 1)?;
                let len = usize_arg(&args[0], name)?;
                let buffer = GcBuffer::new(&self.heap, len).map_err(|error| error.to_string())?;
                Ok(Some(Value::GcBuffer(Rc::new(buffer))))
            }
            "buf_len" => {
                require_arity(name, args, 1)?;
                let buffer = gc_buffer_arg(&args[0], name)?;
                Ok(Some(Value::Int(buffer.len() as i64)))
            }
            "buf_get_u8" => {
                require_arity(name, args, 2)?;
                let buffer = gc_buffer_arg(&args[0], name)?;
                let offset = usize_arg(&args[1], name)?;
                let value = buffer.read_u8(offset).map_err(|error| error.to_string())?;
                Ok(Some(Value::UInt(value as u64)))
            }
            "buf_set_u8" => {
                require_arity(name, args, 3)?;
                let buffer = gc_buffer_arg(&args[0], name)?;
                let offset = usize_arg(&args[1], name)?;
                let value = unsigned_arg(&args[2], name, u8::MAX as u64)? as u8;
                buffer
                    .write_u8(offset, value)
                    .map_err(|error| error.to_string())?;
                Ok(Some(Value::Unit))
            }
            "buf_get_u32" => {
                require_arity(name, args, 2)?;
                let buffer = gc_buffer_arg(&args[0], name)?;
                let offset = usize_arg(&args[1], name)?;
                let value = buffer.read_u32(offset).map_err(|error| error.to_string())?;
                Ok(Some(Value::UInt(value as u64)))
            }
            "buf_set_u32" => {
                require_arity(name, args, 3)?;
                let buffer = gc_buffer_arg(&args[0], name)?;
                let offset = usize_arg(&args[1], name)?;
                let value = unsigned_arg(&args[2], name, u32::MAX as u64)? as u32;
                buffer
                    .write_u32(offset, value)
                    .map_err(|error| error.to_string())?;
                Ok(Some(Value::Unit))
            }
            "memcpy" | "memmove" => {
                let (dst, dst_offset, src, src_offset, count) = match args.len() {
                    3 => (
                        &args[0],
                        0usize,
                        &args[1],
                        0usize,
                        usize_arg(&args[2], name)?,
                    ),
                    5 => (
                        &args[0],
                        usize_arg(&args[1], name)?,
                        &args[2],
                        usize_arg(&args[3], name)?,
                        usize_arg(&args[4], name)?,
                    ),
                    _ => {
                        return Err(format!(
                            "`{name}` expects 3 or 5 arguments, received {}",
                            args.len()
                        ));
                    }
                };
                let destination = gc_buffer_arg(dst, name)?;
                let source = gc_buffer_arg(src, name)?;
                let result = if name == "memmove" {
                    destination.memmove(dst_offset, source.as_ref(), src_offset, count)
                } else {
                    destination.memcpy(dst_offset, source.as_ref(), src_offset, count)
                };
                result.map_err(|error| error.to_string())?;
                Ok(Some(Value::Unit))
            }
            "str_to_buf" => {
                require_arity(name, args, 1)?;
                let Value::String(text) = &args[0] else {
                    return Err("str_to_buf expects a string".into());
                };
                let buffer =
                    GcBuffer::new(&self.heap, text.len()).map_err(|error| error.to_string())?;
                buffer
                    .write_bytes(0, text.as_bytes())
                    .map_err(|error| error.to_string())?;
                Ok(Some(Value::GcBuffer(Rc::new(buffer))))
            }
            "buf_to_str" => {
                require_arity(name, args, 1)?;
                let buffer = gc_buffer_arg(&args[0], name)?;
                let text = String::from_utf8(buffer.read_bytes())
                    .map_err(|error| format!("buf_to_str: invalid UTF-8: {error}"))?;
                Ok(Some(Value::String(text)))
            }
            _ => Ok(None),
        }
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
                if op == "and" || op == "or" {
                    return Ok(Value::Bool(rhs.truthy()));
                }
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
                if let Some(value) = self.gc_buffer_builtin(name, &values)? {
                    return Ok(value);
                }
                if let Some(value) = self.ffi_builtin(name, &values)? {
                    return Ok(value);
                }
                if let Some(value) = self.call_extern(name, &values)? {
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
                        items.get(index).cloned().ok_or_else(|| {
                            format!("array index {index} out of bounds (length {})", items.len())
                        })
                    }
                    (Value::String(text), Value::Int(index)) => {
                        let index = usize::try_from(index).map_err(|_| "negative string index")?;
                        text.chars()
                            .nth(index)
                            .map(|character| Value::String(character.to_string()))
                            .ok_or_else(|| "string index out of bounds".into())
                    }
                    _ => Err("index is only supported for arrays and strings".into()),
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
    match (op, lhs, rhs) {
        ("+", Value::String(lhs), Value::String(rhs)) => Ok(Value::String(lhs + &rhs)),
        ("+", Value::Array(mut lhs), Value::Array(rhs)) => {
            lhs.extend(rhs);
            Ok(Value::Array(lhs))
        }
        (op, lhs, rhs) => binary_numeric(op, lhs, rhs),
    }
}

fn binary_numeric(op: &str, lhs: Value, rhs: Value) -> Result<Value, String> {
    match op {
        "==" => Ok(Value::Bool(values_equal(&lhs, &rhs))),
        "!=" => Ok(Value::Bool(!values_equal(&lhs, &rhs))),
        "is" => Ok(Value::Bool(values_equal(&lhs, &rhs))),
        "+" | "-" | "*" | "/" | "%" | "<" | "<=" | ">" | ">=" => {
            if matches!(lhs, Value::Int(_) | Value::UInt(_))
                && matches!(rhs, Value::Int(_) | Value::UInt(_))
            {
                if matches!(op, "<" | "<=" | ">" | ">=") {
                    let ordering = compare_integer_values(&lhs, &rhs)?;
                    return Ok(Value::Bool(match op {
                        "<" => ordering == Ordering::Less,
                        "<=" => ordering != Ordering::Greater,
                        ">" => ordering == Ordering::Greater,
                        ">=" => ordering != Ordering::Less,
                        _ => unreachable!(),
                    }));
                }
                if matches!(lhs, Value::UInt(_)) || matches!(rhs, Value::UInt(_)) {
                    let lhs = nonnegative_integer(&lhs, op)?;
                    let rhs = nonnegative_integer(&rhs, op)?;
                    return Ok(match op {
                        "+" => Value::UInt(lhs.saturating_add(rhs)),
                        "-" => Value::UInt(lhs.saturating_sub(rhs)),
                        "*" => Value::UInt(lhs.saturating_mul(rhs)),
                        "/" if rhs != 0 => Value::UInt(lhs / rhs),
                        "%" if rhs != 0 => Value::UInt(lhs % rhs),
                        "/" | "%" => return Err("division by zero".into()),
                        _ => unreachable!(),
                    });
                }
            }
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

fn compare_integer_values(lhs: &Value, rhs: &Value) -> Result<Ordering, String> {
    match (lhs, rhs) {
        (Value::Int(lhs), Value::Int(rhs)) => Ok(lhs.cmp(rhs)),
        (Value::UInt(lhs), Value::UInt(rhs)) => Ok(lhs.cmp(rhs)),
        (Value::Int(lhs), Value::UInt(rhs)) => Ok(if *lhs < 0 {
            Ordering::Less
        } else {
            (*lhs as u64).cmp(rhs)
        }),
        (Value::UInt(lhs), Value::Int(rhs)) => Ok(if *rhs < 0 {
            Ordering::Greater
        } else {
            lhs.cmp(&(*rhs as u64))
        }),
        _ => Err("integer comparison requires integer operands".into()),
    }
}

fn nonnegative_integer(value: &Value, op: &str) -> Result<u64, String> {
    match value {
        Value::Int(value) => u64::try_from(*value)
            .map_err(|_| format!("operator `{op}` requires non-negative integers")),
        Value::UInt(value) => Ok(*value),
        _ => Err(format!("operator `{op}` requires integers")),
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
        Value::UInt(index) => usize::try_from(index).map_err(|_| "array index is out of range")?,
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
        Value::UInt(v) => {
            i64::try_from(*v).map_err(|_| format!("operator `{op}` integer is out of range"))
        }
        _ => Err(format!("operator `{op}` requires integers")),
    }
}

fn value_float(value: &Value, op: &str) -> Result<f64, String> {
    match value {
        Value::Int(v) => Ok(*v as f64),
        Value::UInt(v) => Ok(*v as f64),
        Value::Float(v) => Ok(*v),
        _ => Err(format!("operator `{op}` requires numbers")),
    }
}

fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::UInt(a), Value::UInt(b)) => a == b,
        (Value::Int(a), Value::UInt(b)) | (Value::UInt(b), Value::Int(a)) => {
            *a >= 0 && *a as u64 == *b
        }
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::FfiPointer(a), Value::FfiPointer(b)) => a == b,
        (Value::GcBuffer(a), Value::GcBuffer(b)) => Rc::ptr_eq(a, b),
        (Value::FfiAggregate(a), Value::FfiAggregate(b)) => {
            a.ty == b.ty && a.bytes.as_ref() == b.bytes.as_ref()
        }
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

fn infer_ffi_type(value: &Value) -> Result<FfiType, String> {
    Ok(match value {
        Value::Int(_) => FfiType::I64,
        Value::UInt(_) => FfiType::U64,
        Value::Float(_) => FfiType::F64,
        Value::Bool(_) => FfiType::U8,
        Value::String(_) | Value::FfiPointer(_) | Value::FfiBuffer(_) => FfiType::Pointer,
        Value::FfiAggregate(aggregate) => aggregate.ty.clone(),
        other => {
            return Err(format!(
                "cannot infer a C type for {}",
                value_to_string(other)
            ));
        }
    })
}

fn ffi_value_from_lume(ty: FfiType, value: &Value) -> Result<FfiValue, String> {
    let mismatch_type = ty.clone();
    let mismatch = move || {
        format!(
            "value {} does not match C type {mismatch_type:?}",
            value_to_string(value)
        )
    };
    Ok(match (ty.clone(), value) {
        (FfiType::Void, Value::Unit) => FfiValue::Void,
        (FfiType::I8 | FfiType::I16 | FfiType::I32 | FfiType::I64, Value::Int(value)) => {
            FfiValue::Signed(*value)
        }
        (FfiType::I8 | FfiType::I16 | FfiType::I32 | FfiType::I64, Value::UInt(value)) => {
            FfiValue::Signed(
                i64::try_from(*value).map_err(|_| "signed C argument is out of range")?,
            )
        }
        (FfiType::U8 | FfiType::U16 | FfiType::U32 | FfiType::U64, Value::UInt(value)) => {
            FfiValue::Unsigned(*value)
        }
        (FfiType::U8 | FfiType::U16 | FfiType::U32 | FfiType::U64, Value::Int(value)) => {
            FfiValue::Unsigned(
                u64::try_from(*value).map_err(|_| "unsigned C argument must be non-negative")?,
            )
        }
        (FfiType::U8, Value::Bool(value)) => FfiValue::Unsigned(u64::from(*value)),
        (FfiType::F32 | FfiType::F64, Value::Float(value)) => FfiValue::Float(*value),
        (FfiType::F32 | FfiType::F64, Value::Int(value)) => FfiValue::Float(*value as f64),
        (FfiType::F32 | FfiType::F64, Value::UInt(value)) => FfiValue::Float(*value as f64),
        (FfiType::Pointer, Value::FfiPointer(pointer)) => FfiValue::Pointer(*pointer),
        (FfiType::Pointer, Value::String(value)) => FfiValue::String(value.clone()),
        (FfiType::Pointer, Value::FfiCallback(callback)) => FfiValue::Callback(Rc::clone(callback)),
        (FfiType::Pointer, Value::FfiBuffer(buffer)) => FfiValue::Buffer(Rc::clone(buffer)),
        (FfiType::Pointer, Value::GcBuffer(buffer)) => {
            FfiValue::Pointer(buffer.as_ptr() as *mut c_void)
        }
        (_, Value::FfiAggregate(value)) if ty == value.ty => FfiValue::Aggregate(value.clone()),
        _ => return Err(mismatch()),
    })
}

fn lume_value_from_ffi(value: FfiValue) -> Value {
    match value {
        FfiValue::Void => Value::Unit,
        FfiValue::Signed(value) => Value::Int(value),
        FfiValue::Unsigned(value) => Value::UInt(value),
        FfiValue::Float(value) => Value::Float(value),
        FfiValue::Pointer(value) => Value::FfiPointer(value),
        FfiValue::String(value) => Value::String(value),
        FfiValue::Callback(callback) => Value::FfiCallback(callback),
        FfiValue::Buffer(buffer) => Value::FfiBuffer(buffer),
        FfiValue::Aggregate(value) => Value::FfiAggregate(value),
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
                Value::String(text) => Ok(Some(Value::Int(text.chars().count() as i64))),
                _ => Err("len expects string or array".into()),
            }
        }
        "to_string" => {
            require_arity(name, args, 1)?;
            Ok(Some(Value::String(value_to_string(&args[0]))))
        }
        "now_ms" => {
            require_arity(name, args, 0)?;
            Ok(Some(Value::Int(clock::wall_ms())))
        }
        "mono_ms" => {
            require_arity(name, args, 0)?;
            Ok(Some(Value::Int(clock::monotonic_ms())))
        }
        "u8" => {
            require_arity(name, args, 1)?;
            let value = unsigned_arg(&args[0], name, u8::MAX as u64)? as u8;
            Ok(Some(Value::UInt(value as u64)))
        }
        "u32" => {
            require_arity(name, args, 1)?;
            let value = unsigned_arg(&args[0], name, u32::MAX as u64)? as u32;
            Ok(Some(Value::UInt(value as u64)))
        }
        "is_alpha"
        | "is_digit"
        | "is_alphanumeric"
        | "is_whitespace"
        | "is_newline"
        | "is_identifier_start"
        | "is_identifier_continue" => {
            require_arity(name, args, 1)?;
            let character = match &args[0] {
                Value::String(value) => {
                    let mut chars = value.chars();
                    let Some(character) = chars.next() else {
                        return Ok(Some(Value::Bool(false)));
                    };
                    if chars.next().is_some() {
                        return Err(format!("{name} expects a single-character string"));
                    }
                    character
                }
                _ => return Err(format!("{name} expects a string")),
            };
            let valid = match name {
                "is_alpha" => character.is_alphabetic(),
                "is_digit" => character.is_ascii_digit(),
                "is_alphanumeric" => character.is_alphanumeric(),
                "is_whitespace" => character.is_whitespace(),
                "is_newline" => character == '\n',
                "is_identifier_start" => character == '_' || character.is_alphabetic(),
                "is_identifier_continue" => character == '_' || character.is_alphanumeric(),
                _ => unreachable!(),
            };
            Ok(Some(Value::Bool(valid)))
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

fn gc_buffer_arg<'a>(value: &'a Value, op: &str) -> Result<&'a Rc<GcBuffer>, String> {
    match value {
        Value::GcBuffer(buffer) => Ok(buffer),
        _ => Err(format!("`{op}` expects a GC byte buffer")),
    }
}

fn usize_arg(value: &Value, op: &str) -> Result<usize, String> {
    let number = value_int(value, op)?;
    usize::try_from(number).map_err(|_| format!("`{op}` expects a non-negative size"))
}

fn unsigned_arg(value: &Value, op: &str, max: u64) -> Result<u64, String> {
    let number = value_int(value, op)?;
    if number < 0 {
        return Err(format!("`{op}` expects a non-negative integer"));
    }
    let number = number as u64;
    if number > max {
        return Err(format!("`{op}` value {number} is out of range 0..={max}"));
    }
    Ok(number)
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
        Value::UInt(v) => v.to_string(),
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
        Value::FfiLibrary(_) => "<ffi library>".into(),
        Value::FfiSymbol(_) => "<ffi symbol>".into(),
        Value::FfiCallback(_) => "<ffi callback>".into(),
        Value::FfiBuffer(buffer) => format!("<ffi buffer {} bytes>", buffer.size()),
        Value::FfiAggregate(value) => {
            format!("<ffi aggregate {:?} {} bytes>", value.ty, value.bytes.len())
        }
        Value::FfiPointer(pointer) => format!("ptr({pointer:p})"),
        Value::GcBuffer(buffer) => format!("<gc buffer {} bytes>", buffer.len()),
        Value::Unit => "()".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::Clock;

    struct FakeClock;

    impl Clock for FakeClock {
        fn wall_ms(&self) -> i64 {
            1_700_000_000_999
        }

        fn monotonic_ms(&self) -> i64 {
            1234
        }
    }

    #[test]
    fn fixed_width_constructors_check_ranges() {
        assert_eq!(
            builtin("u8", &[Value::Int(255)]).unwrap(),
            Some(Value::UInt(255))
        );
        assert!(builtin("u8", &[Value::Int(256)]).is_err());
        assert_eq!(
            builtin("u32", &[Value::Int(4_294_967_295)]).unwrap(),
            Some(Value::UInt(4_294_967_295))
        );
        assert!(builtin("u32", &[Value::Int(-1)]).is_err());
    }

    #[test]
    fn gc_buffer_builtins_round_trip() {
        let evaluator = Evaluator::new(vec![]);
        let buffer = evaluator
            .gc_buffer_builtin("buf_new", &[Value::Int(8)])
            .unwrap()
            .unwrap();

        evaluator
            .gc_buffer_builtin(
                "buf_set_u8",
                &[buffer.clone(), Value::Int(0), Value::Int(255)],
            )
            .unwrap();
        evaluator
            .gc_buffer_builtin(
                "buf_set_u32",
                &[buffer.clone(), Value::Int(4), Value::Int(0x0102_0304)],
            )
            .unwrap();

        assert_eq!(
            evaluator
                .gc_buffer_builtin("buf_get_u8", &[buffer.clone(), Value::Int(0)])
                .unwrap(),
            Some(Value::UInt(255))
        );
        assert_eq!(
            evaluator
                .gc_buffer_builtin("buf_get_u32", &[buffer.clone(), Value::Int(4)])
                .unwrap(),
            Some(Value::UInt(0x0102_0304))
        );
        assert!(
            evaluator
                .gc_buffer_builtin("buf_get_u32", &[buffer.clone(), Value::Int(5)])
                .is_err()
        );

        let text = evaluator
            .gc_buffer_builtin("str_to_buf", &[Value::String("lume".into())])
            .unwrap()
            .unwrap();
        assert_eq!(
            evaluator.gc_buffer_builtin("buf_to_str", &[text]).unwrap(),
            Some(Value::String("lume".into()))
        );
    }

    #[test]
    fn memcpy_supports_offsets() {
        let evaluator = Evaluator::new(vec![]);
        let source = evaluator
            .gc_buffer_builtin("buf_new", &[Value::Int(4)])
            .unwrap()
            .unwrap();
        let destination = evaluator
            .gc_buffer_builtin("buf_new", &[Value::Int(8)])
            .unwrap()
            .unwrap();
        evaluator
            .gc_buffer_builtin(
                "buf_set_u32",
                &[source.clone(), Value::Int(0), Value::Int(0xdead_beef)],
            )
            .unwrap();
        evaluator
            .gc_buffer_builtin(
                "memcpy",
                &[
                    destination.clone(),
                    Value::Int(4),
                    source,
                    Value::Int(0),
                    Value::Int(4),
                ],
            )
            .unwrap();
        assert_eq!(
            evaluator
                .gc_buffer_builtin("buf_get_u32", &[destination, Value::Int(4)])
                .unwrap(),
            Some(Value::UInt(0xdead_beef))
        );
    }

    #[test]
    fn memmove_allows_overlapping_ranges() {
        let evaluator = Evaluator::new(vec![]);
        let buffer = evaluator
            .gc_buffer_builtin("buf_new", &[Value::Int(8)])
            .unwrap()
            .unwrap();
        for index in 0..8 {
            evaluator
                .gc_buffer_builtin(
                    "buf_set_u8",
                    &[buffer.clone(), Value::Int(index), Value::Int(index + 1)],
                )
                .unwrap();
        }
        evaluator
            .gc_buffer_builtin(
                "memmove",
                &[
                    buffer.clone(),
                    Value::Int(1),
                    buffer.clone(),
                    Value::Int(0),
                    Value::Int(4),
                ],
            )
            .unwrap();
        let bytes = (0..8)
            .map(|index| {
                let Value::UInt(byte) = evaluator
                    .gc_buffer_builtin("buf_get_u8", &[buffer.clone(), Value::Int(index)])
                    .unwrap()
                    .unwrap()
                else {
                    panic!("buf_get_u8 must return UInt");
                };
                byte
            })
            .collect::<Vec<_>>();
        assert_eq!(bytes, [1, 1, 2, 3, 4, 6, 7, 8]);
    }

    #[test]
    fn time_builtins_use_the_injected_clock() {
        clock::with_clock(Arc::new(FakeClock), || {
            assert_eq!(
                builtin("now_ms", &[]).unwrap(),
                Some(Value::Int(1_700_000_000_999))
            );
            assert_eq!(builtin("mono_ms", &[]).unwrap(), Some(Value::Int(1234)));
            assert!(builtin("now_ms", &[Value::Int(1)]).is_err());
        });
    }
}
