use crate::clock::{mono_ms_builtin, now_ms_builtin};
use crate::gc::GcHeap;
use crate::ir::{IrFunc, IrOp, ValueId};
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::OptimizationLevel;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::execution_engine::{ExecutionEngine, JitFunction};
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, IntValue, PhiValue,
};
use self_cell::self_cell;
use std::collections::{HashMap, HashSet};
use std::fmt;

pub type BuiltinFn = unsafe extern "C" fn(*const i64, u64) -> i64;
type NativeFunction = unsafe extern "C" fn(*const i64, u64) -> i64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError(pub String);

impl CoreError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CoreError {}

struct CompiledModule<'ctx> {
    module: Module<'ctx>,
    engine: ExecutionEngine<'ctx>,
}

self_cell!(
    struct OwnedJit {
        owner: Context,
        #[not_covariant]
        dependent: CompiledModule,
    }
);

pub struct LumeCore {
    functions: HashMap<String, IrFunc>,
    builtins: HashMap<String, BuiltinFn>,
    compiled: Option<OwnedJit>,
    bitcodes: Vec<Vec<u8>>,
    gc: GcHeap,
}

impl LumeCore {
    /// Creates an empty core.
    ///
    /// This is a batch/one-shot JIT runtime: define all IR functions and builtins,
    /// then make the first call. After compilation, the module is frozen and
    /// additional definitions are rejected. Hot reload can be implemented by
    /// replacing this runtime with a newly compiled one.
    pub fn new() -> Self {
        let mut core = Self {
            functions: HashMap::new(),
            builtins: HashMap::new(),
            compiled: None,
            bitcodes: Vec::new(),
            gc: GcHeap::new(),
        };
        core.register_builtin("now_ms", now_ms_builtin)
            .expect("default time builtins must register successfully");
        core.register_builtin("mono_ms", mono_ms_builtin)
            .expect("default time builtins must register successfully");
        core
    }

    /// Adds an IR function. All definitions must be registered before the first call.
    pub fn define_function(&mut self, function: IrFunc) -> Result<(), CoreError> {
        self.ensure_not_compiled("define functions")?;
        function
            .validate()
            .map_err(|error| CoreError::new(error.to_string()))?;
        if self.functions.contains_key(&function.name) {
            return Err(CoreError::new(format!(
                "function `{}` is already defined",
                function.name
            )));
        }
        if self.builtins.contains_key(&function.name) {
            return Err(CoreError::new(format!(
                "function `{}` conflicts with a registered builtin",
                function.name
            )));
        }
        self.functions.insert(function.name.clone(), function);
        Ok(())
    }

    /// Registers an external function using the core's uniform `(args, count) -> i64` ABI.
    ///
    /// The JIT integer ABI is intentionally separate from tagged GC values:
    /// arithmetic and control flow use `i64`, while object/pointer operations
    /// belong in external builtin dispatch and can bridge to `Value` separately.
    pub fn register_builtin(
        &mut self,
        name: impl Into<String>,
        function: BuiltinFn,
    ) -> Result<(), CoreError> {
        self.ensure_not_compiled("register builtins")?;
        let name = name.into();
        if name.is_empty() {
            return Err(CoreError::new("builtin name must not be empty"));
        }
        if self.functions.contains_key(&name) || self.builtins.contains_key(&name) {
            return Err(CoreError::new(format!(
                "function or builtin `{name}` is already defined"
            )));
        }
        self.builtins.insert(name, function);
        Ok(())
    }

    /// Loads LLVM bitcode produced by another language front-end.
    ///
    /// Bitcode must be loaded before the JIT module is compiled. It is linked
    /// into the generated module when the first IR function is invoked.
    pub fn load_bitcode(&mut self, bytes: &[u8]) -> Result<(), CoreError> {
        self.ensure_not_compiled("load bitcode")?;
        if bytes.is_empty() {
            return Err(CoreError::new("cannot load empty LLVM bitcode"));
        }
        self.bitcodes.push(bytes.to_vec());
        Ok(())
    }

    /// Reads a file as bytes for the Lume bootstrap/self-hosting layer.
    pub fn read_file(&self, path: impl AsRef<std::path::Path>) -> Result<Vec<u8>, CoreError> {
        std::fs::read(path).map_err(|error| CoreError::new(error.to_string()))
    }

    /// Writes bytes to a file for build and bootstrap tools.
    pub fn write_file(
        &self,
        path: impl AsRef<std::path::Path>,
        bytes: &[u8],
    ) -> Result<(), CoreError> {
        std::fs::write(path, bytes).map_err(|error| CoreError::new(error.to_string()))
    }

    /// Runs an external build tool and requires a successful exit status.
    pub fn run_command(
        &self,
        program: impl AsRef<std::ffi::OsStr>,
        args: &[&std::ffi::OsStr],
    ) -> Result<std::process::Output, CoreError> {
        std::process::Command::new(program)
            .args(args)
            .output()
            .map_err(|error| CoreError::new(error.to_string()))
            .and_then(|output| {
                if output.status.success() {
                    Ok(output)
                } else {
                    Err(CoreError::new(format!(
                        "external command failed with {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                    )))
                }
            })
    }

    /// Registers a GC-managed pointer as an explicit root.
    pub fn gc_push_root(&mut self, ptr: i64) -> Result<(), CoreError> {
        if ptr == 0 {
            return Err(CoreError::new("cannot use a null pointer as a GC root"));
        }
        self.gc.push_root(ptr as *mut u8).map_err(CoreError::new)
    }

    pub fn gc_pop_root(&mut self) -> Result<(), CoreError> {
        self.gc.pop_root().map_err(CoreError::new)
    }

    /// Pins a GC object so it stays live and cannot move while native code holds it.
    pub fn gc_pin(&mut self, ptr: i64) -> Result<(), CoreError> {
        if ptr == 0 {
            return Err(CoreError::new("cannot pin a null pointer"));
        }
        self.gc.try_pin(ptr as *mut u8).map_err(CoreError::new)
    }

    /// Releases one pin previously acquired with [`LumeCore::gc_pin`].
    pub fn gc_unpin(&mut self, ptr: i64) -> Result<(), CoreError> {
        if ptr == 0 {
            return Err(CoreError::new("cannot unpin a null pointer"));
        }
        self.gc.try_unpin(ptr as *mut u8).map_err(CoreError::new)
    }

    /// Resolves a GC pointer after collection may have moved its object.
    pub fn gc_resolve(&self, ptr: i64) -> i64 {
        self.gc.resolve(ptr as *mut u8) as i64
    }

    /// Compiles all registered IR functions on the first invocation and reuses the JIT code.
    pub fn call(&mut self, name: &str, args: &[i64]) -> Result<i64, CoreError> {
        let function = self
            .functions
            .get(name)
            .ok_or_else(|| CoreError::new(format!("function `{name}` is not defined")))?;
        if args.len() != function.n_params {
            return Err(CoreError::new(format!(
                "function `{name}` expects {} arguments, received {}",
                function.n_params,
                args.len()
            )));
        }

        self.ensure_compiled()?;
        let compiled = self
            .compiled
            .as_ref()
            .expect("compiled module is initialized after successful compilation");
        let args_ptr = if args.is_empty() {
            std::ptr::null()
        } else {
            args.as_ptr()
        };
        compiled.with_dependent(|_, compiled| {
            let function: JitFunction<'_, NativeFunction> = unsafe {
                compiled
                    .engine
                    .get_function(name)
                    .map_err(|error| CoreError::new(format!("cannot resolve `{name}`: {error}")))?
            };
            Ok(unsafe { function.call(args_ptr, args.len() as u64) })
        })
    }

    /// Returns the generated LLVM IR, compiling the module if necessary.
    pub fn llvm_ir(&mut self) -> Result<String, CoreError> {
        self.ensure_compiled()?;
        let compiled = self
            .compiled
            .as_ref()
            .expect("compiled module is initialized after successful compilation");
        Ok(compiled.with_dependent(|_, compiled| compiled.module.print_to_string().to_string()))
    }

    pub fn is_compiled(&self) -> bool {
        self.compiled.is_some()
    }

    fn ensure_not_compiled(&self, action: &str) -> Result<(), CoreError> {
        if self.is_compiled() {
            return Err(CoreError::new(format!(
                "cannot {action} after the JIT module has been compiled"
            )));
        }
        Ok(())
    }

    fn ensure_compiled(&mut self) -> Result<(), CoreError> {
        if self.is_compiled() {
            return Ok(());
        }
        if self.functions.is_empty() {
            return Err(CoreError::new("cannot compile an empty module"));
        }

        let functions = &self.functions;
        let builtins = &self.builtins;
        let bitcodes = &self.bitcodes;
        let compiled = OwnedJit::try_new(Context::create(), |context| {
            compile_module(context, functions, builtins, bitcodes)
        })
        .map_err(|error| CoreError::new(error.to_string()))?;
        self.compiled = Some(compiled);
        Ok(())
    }
}

impl Default for LumeCore {
    fn default() -> Self {
        Self::new()
    }
}

fn compile_module<'ctx>(
    context: &'ctx Context,
    functions: &HashMap<String, IrFunc>,
    builtins: &HashMap<String, BuiltinFn>,
    bitcodes: &[Vec<u8>],
) -> Result<CompiledModule<'ctx>, CoreError> {
    let module = context.create_module("lumec_core");
    let mut external_names = HashSet::new();
    for (index, bytes) in bitcodes.iter().enumerate() {
        let buffer =
            MemoryBuffer::create_from_memory_range_copy(bytes, &format!("lume_bitcode_{index}"));
        let external = Module::parse_bitcode_from_buffer(&buffer, context)
            .map_err(|error| CoreError::new(format!("cannot parse LLVM bitcode: {error}")))?;
        for function in external.get_functions() {
            if let Ok(name) = function.get_name().to_str() {
                external_names.insert(name.to_string());
            }
        }
        module
            .link_in_module(external)
            .map_err(|error| CoreError::new(format!("cannot link LLVM bitcode: {error}")))?;
    }
    validate_calls(functions, builtins, &external_names)?;
    let i64_ty = context.i64_type();
    let args_ptr_ty = context.ptr_type(AddressSpace::default());
    let native_fn_ty = i64_ty.fn_type(&[args_ptr_ty.into(), i64_ty.into()], false);

    let mut function_values = HashMap::new();
    for ir in functions.values() {
        function_values.insert(
            ir.name.clone(),
            module.add_function(&ir.name, native_fn_ty, None),
        );
    }
    for name in builtins.keys() {
        function_values.insert(name.clone(), module.add_function(name, native_fn_ty, None));
    }
    for external in module.get_functions() {
        if let Ok(name) = external.get_name().to_str() {
            function_values.entry(name.to_string()).or_insert(external);
        }
    }

    for ir in functions.values() {
        let function = function_values[&ir.name];
        lower_function(context, ir, function, &function_values, &external_names)?;
    }

    module
        .verify()
        .map_err(|error| CoreError::new(format!("generated invalid LLVM IR:\n{error}")))?;

    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .map_err(|error| CoreError::new(format!("cannot create JIT engine: {error}")))?;
    for (name, builtin) in builtins {
        let declaration = function_values[name];
        engine.add_global_mapping(&declaration, *builtin as usize);
    }

    Ok(CompiledModule { module, engine })
}

fn validate_calls(
    functions: &HashMap<String, IrFunc>,
    builtins: &HashMap<String, BuiltinFn>,
    external_names: &HashSet<String>,
) -> Result<(), CoreError> {
    for function in functions.values() {
        for op in function.blocks.iter().flat_map(|block| &block.ops) {
            if let IrOp::Call(name, args) = op {
                if let Some(callee) = functions.get(name) {
                    if args.len() != callee.n_params {
                        return Err(CoreError::new(format!(
                            "call to `{name}` in `{}` passes {} arguments, expected {}",
                            function.name,
                            args.len(),
                            callee.n_params
                        )));
                    }
                } else if !builtins.contains_key(name) && !external_names.contains(name) {
                    return Err(CoreError::new(format!(
                        "call to undefined function or builtin `{name}` in `{}`",
                        function.name
                    )));
                }
            }
        }
    }
    Ok(())
}

fn lower_function<'ctx>(
    context: &'ctx Context,
    ir: &IrFunc,
    function: FunctionValue<'ctx>,
    functions: &HashMap<String, FunctionValue<'ctx>>,
    external_names: &HashSet<String>,
) -> Result<(), CoreError> {
    let builder = context.create_builder();
    let i64_ty = context.i64_type();
    let args_ptr = function
        .get_nth_param(0)
        .ok_or_else(|| CoreError::new("missing argument array parameter"))?
        .into_pointer_value();
    let blocks: Vec<_> = ir
        .blocks
        .iter()
        .enumerate()
        .map(|(index, _)| context.append_basic_block(function, &format!("bb{index}")))
        .collect();
    let max_call_args = ir
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .filter_map(|op| match op {
            IrOp::Call(_, args) => Some(args.len()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let call_args = if max_call_args == 0 {
        None
    } else {
        builder.position_at_end(blocks[0]);
        Some(
            builder
                .build_array_alloca(
                    i64_ty,
                    i64_ty.const_int(max_call_args as u64, false),
                    "call_args",
                )
                .map_err(build_error)?,
        )
    };
    let mut values: Vec<Option<BasicValueEnum>> = vec![None; ir.value_count()];
    let mut value_id = 0;
    let mut pending_phis: Vec<(PhiValue, Vec<(usize, ValueId)>)> = Vec::new();

    for (block_id, block) in ir.blocks.iter().enumerate() {
        builder.position_at_end(blocks[block_id]);
        for op in &block.ops {
            match op {
                IrOp::Const(value) => {
                    values[value_id] = Some(i64_ty.const_int(*value as u64, false).into());
                }
                IrOp::Param(index) => {
                    let index = i64_ty.const_int(*index as u64, false);
                    let arg_ptr = unsafe {
                        builder
                            .build_gep(i64_ty, args_ptr, &[index], "arg_ptr")
                            .map_err(build_error)?
                    };
                    values[value_id] = Some(
                        builder
                            .build_load(i64_ty, arg_ptr, "arg")
                            .map_err(build_error)?,
                    );
                }
                IrOp::Add(a, b) => {
                    values[value_id] = Some(
                        builder
                            .build_int_add(value(&values, *a)?, value(&values, *b)?, "add")
                            .map_err(build_error)?
                            .into(),
                    );
                }
                IrOp::Sub(a, b) => {
                    values[value_id] = Some(
                        builder
                            .build_int_sub(value(&values, *a)?, value(&values, *b)?, "sub")
                            .map_err(build_error)?
                            .into(),
                    );
                }
                IrOp::Mul(a, b) => {
                    values[value_id] = Some(
                        builder
                            .build_int_mul(value(&values, *a)?, value(&values, *b)?, "mul")
                            .map_err(build_error)?
                            .into(),
                    );
                }
                IrOp::Div(a, b) => {
                    values[value_id] = Some(
                        builder
                            .build_int_signed_div(value(&values, *a)?, value(&values, *b)?, "div")
                            .map_err(build_error)?
                            .into(),
                    );
                }
                IrOp::Eq(a, b)
                | IrOp::Neq(a, b)
                | IrOp::Lt(a, b)
                | IrOp::Gt(a, b)
                | IrOp::Le(a, b)
                | IrOp::Ge(a, b) => {
                    let predicate = match op {
                        IrOp::Eq(..) => IntPredicate::EQ,
                        IrOp::Neq(..) => IntPredicate::NE,
                        IrOp::Lt(..) => IntPredicate::SLT,
                        IrOp::Gt(..) => IntPredicate::SGT,
                        IrOp::Le(..) => IntPredicate::SLE,
                        IrOp::Ge(..) => IntPredicate::SGE,
                        _ => unreachable!(),
                    };
                    let compare = builder
                        .build_int_compare(
                            predicate,
                            value(&values, *a)?,
                            value(&values, *b)?,
                            "compare",
                        )
                        .map_err(build_error)?;
                    values[value_id] = Some(
                        builder
                            .build_int_z_extend(compare, i64_ty, "compare_i64")
                            .map_err(build_error)?
                            .into(),
                    );
                }
                IrOp::Phi(incoming) => {
                    let phi = builder.build_phi(i64_ty, "phi").map_err(build_error)?;
                    values[value_id] = Some(phi.as_basic_value());
                    pending_phis.push((phi, incoming.clone()));
                }
                IrOp::If(cond, then_block, else_block) => {
                    let is_true = builder
                        .build_int_compare(
                            IntPredicate::NE,
                            value(&values, *cond)?,
                            i64_ty.const_zero(),
                            "if_cond",
                        )
                        .map_err(build_error)?;
                    builder
                        .build_conditional_branch(is_true, blocks[*then_block], blocks[*else_block])
                        .map_err(build_error)?;
                }
                IrOp::Jump(target) => {
                    builder
                        .build_unconditional_branch(blocks[*target])
                        .map_err(build_error)?;
                }
                IrOp::Return(result) => {
                    builder
                        .build_return(Some(&value(&values, *result)?))
                        .map_err(build_error)?;
                }
                IrOp::Call(name, arg_ids) => {
                    let callee = functions.get(name).ok_or_else(|| {
                        CoreError::new(format!("missing compiled function declaration `{name}`"))
                    })?;
                    if external_names.contains(name) {
                        let args: Vec<BasicMetadataValueEnum> = arg_ids
                            .iter()
                            .map(|id| value(&values, *id).map(Into::into))
                            .collect::<Result<Vec<_>, _>>()?;
                        let call = builder
                            .build_call(*callee, &args, "external_call")
                            .map_err(build_error)?;
                        let result = call.try_as_basic_value().basic().ok_or_else(|| {
                            CoreError::new("external call did not produce a value")
                        })?;
                        values[value_id] = Some(result);
                    } else {
                        let arg_ptr = build_arg_array(
                            &builder,
                            i64_ty,
                            context.ptr_type(AddressSpace::default()),
                            call_args,
                            &values,
                            arg_ids,
                        )?;
                        let args: [BasicMetadataValueEnum; 2] = [
                            arg_ptr.into(),
                            i64_ty.const_int(arg_ids.len() as u64, false).into(),
                        ];
                        let call = builder
                            .build_call(*callee, &args, "call")
                            .map_err(build_error)?;
                        let result = call
                            .try_as_basic_value()
                            .basic()
                            .ok_or_else(|| CoreError::new("call did not produce a value"))?;
                        values[value_id] = Some(result);
                    }
                }
            }
            if op.defines_value() {
                value_id += 1;
            }
        }
    }

    for (phi, incoming) in pending_phis {
        let incoming_values: Result<Vec<_>, CoreError> = incoming
            .iter()
            .map(|(predecessor, source)| {
                let incoming_value =
                    values
                        .get(*source)
                        .and_then(|value| *value)
                        .ok_or_else(|| {
                            CoreError::new(format!("phi source value {source} is unavailable"))
                        })?;
                Ok((incoming_value, blocks[*predecessor]))
            })
            .collect();
        let incoming_values = incoming_values?;
        let incoming_refs: Vec<(&dyn BasicValue, _)> = incoming_values
            .iter()
            .map(|(value, block)| (value as &dyn BasicValue, *block))
            .collect();
        phi.add_incoming(&incoming_refs);
    }

    Ok(())
}

fn build_arg_array<'ctx>(
    builder: &Builder<'ctx>,
    i64_ty: inkwell::types::IntType<'ctx>,
    args_ptr_ty: inkwell::types::PointerType<'ctx>,
    call_args: Option<inkwell::values::PointerValue<'ctx>>,
    values: &[Option<BasicValueEnum<'ctx>>],
    arg_ids: &[ValueId],
) -> Result<inkwell::values::PointerValue<'ctx>, CoreError> {
    if arg_ids.is_empty() {
        return Ok(args_ptr_ty.const_null());
    }
    let array = call_args.ok_or_else(|| CoreError::new("missing call argument storage"))?;
    for (index, arg_id) in arg_ids.iter().enumerate() {
        let element_index = i64_ty.const_int(index as u64, false);
        let element = unsafe {
            builder
                .build_gep(i64_ty, array, &[element_index], "call_arg")
                .map_err(build_error)?
        };
        builder
            .build_store(element, value(values, *arg_id)?)
            .map_err(build_error)?;
    }
    Ok(array)
}

fn value<'ctx>(
    values: &[Option<BasicValueEnum<'ctx>>],
    id: ValueId,
) -> Result<IntValue<'ctx>, CoreError> {
    values
        .get(id)
        .and_then(|value| *value)
        .map(BasicValueEnum::into_int_value)
        .ok_or_else(|| CoreError::new(format!("value {id} is not available at this point")))
}

fn build_error(error: impl fmt::Debug) -> CoreError {
    CoreError::new(format!("LLVM IR construction failed: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IrBlock, IrOp};

    fn block(ops: Vec<IrOp>) -> IrBlock {
        IrBlock { ops }
    }

    fn if_merge_function() -> IrFunc {
        IrFunc {
            name: "if_merge".into(),
            n_params: 0,
            blocks: vec![
                block(vec![
                    IrOp::Const(10),
                    IrOp::Const(20),
                    IrOp::Lt(0, 1),
                    IrOp::If(2, 1, 2),
                ]),
                block(vec![IrOp::Const(99), IrOp::Jump(3)]),
                block(vec![IrOp::Const(1), IrOp::Jump(3)]),
                block(vec![IrOp::Phi(vec![(1, 3), (2, 4)]), IrOp::Return(5)]),
            ],
        }
    }

    #[test]
    fn if_merge_runs_and_compiled_function_is_reused() {
        let mut core = LumeCore::new();
        core.define_function(if_merge_function()).unwrap();
        core.define_function(IrFunc {
            name: "if_returns".into(),
            n_params: 0,
            blocks: vec![
                block(vec![IrOp::Const(0), IrOp::If(0, 1, 2)]),
                block(vec![IrOp::Const(11), IrOp::Return(1)]),
                block(vec![IrOp::Const(22), IrOp::Return(2)]),
            ],
        })
        .unwrap();

        assert_eq!(core.call("if_merge", &[]).unwrap(), 99);
        assert_eq!(core.call("if_returns", &[]).unwrap(), 22);
        assert!(core.is_compiled());
        assert_eq!(core.call("if_merge", &[]).unwrap(), 99);
        let llvm = core.llvm_ir().unwrap();
        assert!(llvm.contains("phi i64"));
    }

    #[test]
    fn while_loop_and_multiple_function_calls_run() {
        let mut core = LumeCore::new();
        core.define_function(IrFunc {
            name: "sum_to".into(),
            n_params: 1,
            blocks: vec![
                block(vec![IrOp::Const(0), IrOp::Const(1), IrOp::Jump(1)]),
                block(vec![
                    IrOp::Phi(vec![(0, 0), (2, 6)]),
                    IrOp::Phi(vec![(0, 1), (2, 8)]),
                    IrOp::Param(0),
                    IrOp::Le(3, 4),
                    IrOp::If(5, 2, 3),
                ]),
                block(vec![
                    IrOp::Add(2, 3),
                    IrOp::Const(1),
                    IrOp::Add(3, 7),
                    IrOp::Jump(1),
                ]),
                block(vec![IrOp::Return(2)]),
            ],
        })
        .unwrap();
        core.define_function(IrFunc {
            name: "twice_sum".into(),
            n_params: 1,
            blocks: vec![block(vec![
                IrOp::Param(0),
                IrOp::Call("sum_to".into(), vec![0]),
                IrOp::Call("sum_to".into(), vec![0]),
                IrOp::Add(1, 2),
                IrOp::Return(3),
            ])],
        })
        .unwrap();

        assert_eq!(core.call("sum_to", &[10]).unwrap(), 55);
        assert_eq!(core.call("twice_sum", &[4]).unwrap(), 20);
        assert_eq!(core.call("sum_to", &[10]).unwrap(), 55);
    }

    #[test]
    fn mutually_recursive_functions_run() {
        let mut core = LumeCore::new();
        core.define_function(IrFunc {
            name: "is_even".into(),
            n_params: 1,
            blocks: vec![
                block(vec![
                    IrOp::Param(0),
                    IrOp::Const(0),
                    IrOp::Eq(0, 1),
                    IrOp::If(2, 1, 2),
                ]),
                block(vec![IrOp::Const(1), IrOp::Return(3)]),
                block(vec![
                    IrOp::Const(1),
                    IrOp::Sub(0, 4),
                    IrOp::Call("is_odd".into(), vec![5]),
                    IrOp::Return(6),
                ]),
            ],
        })
        .unwrap();
        core.define_function(IrFunc {
            name: "is_odd".into(),
            n_params: 1,
            blocks: vec![
                block(vec![
                    IrOp::Param(0),
                    IrOp::Const(0),
                    IrOp::Eq(0, 1),
                    IrOp::If(2, 1, 2),
                ]),
                block(vec![IrOp::Const(0), IrOp::Return(3)]),
                block(vec![
                    IrOp::Const(1),
                    IrOp::Sub(0, 4),
                    IrOp::Call("is_even".into(), vec![5]),
                    IrOp::Return(6),
                ]),
            ],
        })
        .unwrap();

        assert_eq!(core.call("is_even", &[10]).unwrap(), 1);
        assert_eq!(core.call("is_even", &[9]).unwrap(), 0);
    }

    #[test]
    fn integer_arithmetic_and_all_signed_comparisons_work() {
        let mut core = LumeCore::new();
        core.define_function(IrFunc {
            name: "arithmetic".into(),
            n_params: 0,
            blocks: vec![block(vec![
                IrOp::Const(7),
                IrOp::Const(3),
                IrOp::Sub(0, 1),
                IrOp::Mul(2, 1),
                IrOp::Div(3, 1),
                IrOp::Return(4),
            ])],
        })
        .unwrap();
        core.define_function(IrFunc {
            name: "comparisons".into(),
            n_params: 0,
            blocks: vec![block(vec![
                IrOp::Const(-2),
                IrOp::Const(3),
                IrOp::Eq(0, 1),
                IrOp::Neq(0, 1),
                IrOp::Lt(0, 1),
                IrOp::Gt(0, 1),
                IrOp::Le(0, 1),
                IrOp::Ge(0, 1),
                IrOp::Add(2, 3),
                IrOp::Add(8, 4),
                IrOp::Add(9, 5),
                IrOp::Add(10, 6),
                IrOp::Add(11, 7),
                IrOp::Return(12),
            ])],
        })
        .unwrap();

        assert_eq!(core.call("arithmetic", &[]).unwrap(), 4);
        assert_eq!(core.call("comparisons", &[]).unwrap(), 3);
    }

    struct FakeClock;

    impl crate::clock::Clock for FakeClock {
        fn wall_ms(&self) -> i64 {
            9001
        }

        fn monotonic_ms(&self) -> i64 {
            17
        }
    }

    #[test]
    fn default_time_builtins_can_be_called_from_ir() {
        let mut core = LumeCore::new();
        core.define_function(IrFunc {
            name: "read_monotonic".into(),
            n_params: 0,
            blocks: vec![block(vec![
                IrOp::Call("mono_ms".into(), vec![]),
                IrOp::Return(0),
            ])],
        })
        .unwrap();

        crate::clock::with_clock(std::sync::Arc::new(FakeClock), || {
            assert_eq!(core.call("read_monotonic", &[]).unwrap(), 17);
        });
    }

    unsafe extern "C" fn add_builtin(args: *const i64, count: u64) -> i64 {
        if count != 2 {
            return -1;
        }
        let values = unsafe { std::slice::from_raw_parts(args, count as usize) };
        values[0] + values[1]
    }

    #[test]
    fn registered_builtin_can_be_called_from_ir() {
        let mut core = LumeCore::new();
        core.register_builtin("add_builtin", add_builtin).unwrap();
        core.define_function(IrFunc {
            name: "use_builtin".into(),
            n_params: 0,
            blocks: vec![block(vec![
                IrOp::Const(8),
                IrOp::Const(9),
                IrOp::Call("add_builtin".into(), vec![0, 1]),
                IrOp::Return(2),
            ])],
        })
        .unwrap();

        assert_eq!(core.call("use_builtin", &[]).unwrap(), 17);
    }

    #[test]
    fn loaded_bitcode_external_function_can_be_called() {
        let context = Context::create();
        let module = context.create_module("external");
        let i64_ty = context.i64_type();
        let fn_ty = i64_ty.fn_type(&[i64_ty.into(), i64_ty.into()], false);
        let function = module.add_function("extern_add", fn_ty, None);
        let entry = context.append_basic_block(function, "entry");
        let builder = context.create_builder();
        builder.position_at_end(entry);
        let lhs = function.get_nth_param(0).unwrap().into_int_value();
        let rhs = function.get_nth_param(1).unwrap().into_int_value();
        let sum = builder.build_int_add(lhs, rhs, "sum").unwrap();
        builder.build_return(Some(&sum)).unwrap();
        let bitcode = module.write_bitcode_to_memory();
        let bytes = bitcode.as_slice();

        let mut core = LumeCore::new();
        core.load_bitcode(bytes).unwrap();
        core.define_function(IrFunc {
            name: "use_external".into(),
            n_params: 0,
            blocks: vec![block(vec![
                IrOp::Const(20),
                IrOp::Const(22),
                IrOp::Call("extern_add".into(), vec![0, 1]),
                IrOp::Return(2),
            ])],
        })
        .unwrap();

        assert_eq!(core.call("use_external", &[]).unwrap(), 42);
    }

    #[test]
    fn rejects_late_definitions_and_wrong_call_arity() {
        let mut core = LumeCore::new();
        core.define_function(IrFunc {
            name: "one".into(),
            n_params: 1,
            blocks: vec![block(vec![IrOp::Param(0), IrOp::Return(0)])],
        })
        .unwrap();
        assert!(core.call("one", &[]).is_err());
        core.call("one", &[1]).unwrap();
        assert!(
            core.define_function(IrFunc {
                name: "two".into(),
                n_params: 0,
                blocks: vec![block(vec![IrOp::Const(2), IrOp::Return(0)])],
            })
            .is_err()
        );
    }
}
