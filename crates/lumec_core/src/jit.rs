use crate::buffer::{GcBuffer, GcBufferHeap};
use crate::clock::{mono_ms_builtin, now_ms_builtin};
use crate::ffi::{FfiSignature, FfiType, FfiValue, ForeignCallback, ForeignLibrary};
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
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

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
    externs: HashMap<String, FfiSignature>,
    dynamic_externs: HashMap<String, Rc<ForeignCallback>>,
    export_callbacks: Vec<Rc<ForeignCallback>>,
    gc: GcHeap,
    buffers: GcBufferHeap,
}

thread_local! {
    static EXPORT_CALLBACKS: RefCell<HashMap<i64, Rc<ForeignCallback>>> =
        RefCell::new(HashMap::new());
    static JIT_LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn export_slot(name: &str, signature: &str) -> i64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    signature.hash(&mut hasher);
    let hash = hasher.finish();
    (hash & 0x7fff_ffff_ffff_ffff) as i64
}

fn set_jit_error(message: impl Into<String>) {
    JIT_LAST_ERROR.with(|error| {
        *error.borrow_mut() = Some(message.into());
    });
}

fn take_jit_error() -> Option<String> {
    JIT_LAST_ERROR.with(|error| error.borrow_mut().take())
}

pub fn clear_jit_export_callbacks() {
    EXPORT_CALLBACKS.with(|callbacks| callbacks.borrow_mut().clear());
}

fn register_jit_export_callback(slot: i64, callback: Rc<ForeignCallback>) {
    EXPORT_CALLBACKS.with(|callbacks| {
        callbacks.borrow_mut().insert(slot, callback);
    });
}

fn lookup_jit_export_callback(slot: i64) -> Option<Rc<ForeignCallback>> {
    EXPORT_CALLBACKS.with(|callbacks| callbacks.borrow().get(&slot).cloned())
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
            externs: HashMap::new(),
            dynamic_externs: HashMap::new(),
            export_callbacks: Vec::new(),
            gc: GcHeap::new(),
            buffers: GcBufferHeap::default(),
        };
        core.register_builtin("now_ms", now_ms_builtin)
            .expect("default time builtins must register successfully");
        core.register_builtin("mono_ms", mono_ms_builtin)
            .expect("default time builtins must register successfully");
        core.register_builtin("ffi_export_ptr", ffi_export_ptr_builtin)
            .expect("default FFI shims must register successfully");
        for (name, function) in [
            ("buf_new", buf_new_builtin as BuiltinFn),
            ("buf_len", buf_len_builtin as BuiltinFn),
            ("buf_get_u8", buf_get_u8_builtin as BuiltinFn),
            ("buf_set_u8", buf_set_u8_builtin as BuiltinFn),
            ("buf_get_u32", buf_get_u32_builtin as BuiltinFn),
            ("buf_set_u32", buf_set_u32_builtin as BuiltinFn),
            ("memcpy", memcpy_builtin as BuiltinFn),
            ("memmove", memmove_builtin as BuiltinFn),
        ] {
            core.register_builtin(name, function)
                .expect("default buffer builtins must register successfully");
        }
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

    /// Declares a C ABI signature for an external symbol.
    ///
    /// Bitcode-linked symbols use this for arity validation and clearer errors;
    /// dynamic externs also register a runtime trampoline with
    /// [`LumeCore::register_dynamic_extern`].
    pub fn register_extern_signature(
        &mut self,
        name: impl Into<String>,
        signature: FfiSignature,
    ) -> Result<(), CoreError> {
        self.ensure_not_compiled("register extern signatures")?;
        let name = name.into();
        if name.is_empty() {
            return Err(CoreError::new("extern name must not be empty"));
        }
        if self.externs.contains_key(&name) {
            return Err(CoreError::new(format!(
                "extern `{name}` is already declared"
            )));
        }
        self.externs.insert(name, signature);
        Ok(())
    }

    /// Registers a shared-library symbol as a JIT-visible extern.
    ///
    /// The symbol is dispatched through `ffi.rs` using libffi, so the signature
    /// must match the native function exactly. The generated trampoline uses
    /// the core's `(args, count) -> i64` ABI and is kept alive for the lifetime
    /// of this runtime.
    pub fn register_dynamic_extern(
        &mut self,
        name: impl Into<String>,
        library_path: impl AsRef<std::path::Path>,
        signature: FfiSignature,
    ) -> Result<(), CoreError> {
        self.ensure_not_compiled("register dynamic externs")?;
        let name = name.into();
        if name.is_empty() {
            return Err(CoreError::new("extern name must not be empty"));
        }
        self.register_extern_signature(name.clone(), signature.clone())?;
        let library = unsafe { ForeignLibrary::open(library_path) }
            .map_err(|error| CoreError::new(error.to_string()))?;
        let symbol = Rc::new(
            library
                .symbol(&name)
                .map_err(|error| CoreError::new(error.to_string()))?,
        );
        let call_signature = signature.clone();
        let call_symbol = Rc::clone(&symbol);
        let error_name = name.clone();
        let callback = ForeignCallback::new(
            FfiSignature::parse("i64(pointer, u64)")
                .map_err(|error| CoreError::new(error.to_string()))?,
            move |args| {
                let (FfiValue::Pointer(raw_args), FfiValue::Unsigned(count)) = (&args[0], &args[1])
                else {
                    return Err("invalid dynamic extern trampoline arguments".into());
                };
                let count_usize = usize::try_from(*count)
                    .map_err(|_| "dynamic extern argument count is too large")?;
                let jit_args = if count_usize == 0 {
                    &[][..]
                } else {
                    unsafe { std::slice::from_raw_parts(*raw_args as *const i64, count_usize) }
                };
                if jit_args.len() != call_signature.args.len() {
                    return Err(format!(
                        "extern `{error_name}` expects {} arguments, received {}",
                        call_signature.args.len(),
                        jit_args.len()
                    ));
                }
                let ffi_args = call_signature
                    .args
                    .iter()
                    .cloned()
                    .zip(jit_args.iter().copied())
                    .map(|(ty, value)| jit_i64_to_ffi(ty, value))
                    .collect::<Result<Vec<_>, String>>()?;
                let result = unsafe { call_symbol.call(&call_signature, &ffi_args) }
                    .map_err(|error| error.to_string())?;
                ffi_to_jit_i64(result).map(FfiValue::Signed)
            },
        )
        .map_err(|error| CoreError::new(error.to_string()))?;
        let code = unsafe { callback.code_ptr() };
        let trampoline: BuiltinFn = unsafe { std::mem::transmute::<*mut c_void, BuiltinFn>(code) };
        self.register_builtin(name.clone(), trampoline)?;
        self.dynamic_externs.insert(name, callback);
        Ok(())
    }

    /// Returns and clears the last error from a dynamic extern trampoline.
    pub fn take_dynamic_extern_error(&self, name: &str) -> Result<Option<String>, CoreError> {
        let callback = self
            .dynamic_externs
            .get(name)
            .ok_or_else(|| CoreError::new(format!("dynamic extern `{name}` is not registered")))?;
        callback
            .take_error()
            .map_err(|error| CoreError::new(error.to_string()))
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
        let _heap_guard = crate::buffer::set_current_heap(self.buffers.clone());
        let compiled = self
            .compiled
            .as_ref()
            .expect("compiled module is initialized after successful compilation");
        let args_ptr = if args.is_empty() {
            std::ptr::null()
        } else {
            args.as_ptr()
        };
        let result = compiled.with_dependent(|_, compiled| {
            let function: JitFunction<'_, NativeFunction> = unsafe {
                compiled
                    .engine
                    .get_function(name)
                    .map_err(|error| CoreError::new(format!("cannot resolve `{name}`: {error}")))?
            };
            Ok(unsafe { function.call(args_ptr, args.len() as u64) })
        })?;
        if let Some(error) = take_jit_error() {
            return Err(CoreError::new(error));
        }
        for callback in &self.export_callbacks {
            if let Some(error) = callback
                .take_error()
                .map_err(|error| CoreError::new(error.to_string()))?
            {
                return Err(CoreError::new(error));
            }
        }
        Ok(result)
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
        let externs = &self.externs;
        let compiled = OwnedJit::try_new(Context::create(), |context| {
            compile_module(context, functions, builtins, bitcodes, externs)
        })
        .map_err(|error| CoreError::new(error.to_string()))?;
        self.compiled = Some(compiled);

        clear_jit_export_callbacks();
        let mut registered = Vec::new();
        for (slot, name, signature_text) in collect_export_callbacks(&self.functions)? {
            let signature = FfiSignature::parse(&signature_text)
                .map_err(|error| CoreError::new(error.to_string()))?;
            let result_type = signature.result.clone();
            let target_name = name.clone();
            let target =
                {
                    let compiled = self
                        .compiled
                        .as_ref()
                        .expect("compiled module exists after compilation");
                    compiled.with_dependent(|_, compiled| {
                        let function: JitFunction<'_, NativeFunction> =
                            unsafe {
                                compiled.engine.get_function(&target_name).map_err(|error| {
                            CoreError::new(format!(
                                "cannot resolve exported function `{target_name}`: {error}"
                            ))
                        })?
                            };
                        Ok(unsafe { function.as_raw() })
                    })?
                };
            let callback = ForeignCallback::new(signature, move |values| {
                let mut jit_args = Vec::with_capacity(values.len());
                for value in values {
                    jit_args.push(ffi_to_jit_i64(value.clone())?);
                }
                let args_ptr = if jit_args.is_empty() {
                    std::ptr::null()
                } else {
                    jit_args.as_ptr()
                };
                let result = unsafe { target(args_ptr, jit_args.len() as u64) };
                if result_type == FfiType::Void {
                    Ok(FfiValue::Void)
                } else {
                    jit_i64_to_ffi(result_type.clone(), result).map_err(|error| error.to_string())
                }
            })
            .map_err(|error| CoreError::new(error.to_string()))?;
            registered.push(Rc::clone(&callback));
            register_jit_export_callback(slot, callback);
        }
        self.export_callbacks = registered;
        Ok(())
    }
}

fn collect_export_callbacks(
    functions: &HashMap<String, IrFunc>,
) -> Result<Vec<(i64, String, String)>, CoreError> {
    let mut callbacks = HashMap::new();
    for ir in functions.values() {
        for op in ir.blocks.iter().flat_map(|block| &block.ops) {
            if let IrOp::FfiExportPtr { name, signature } = op {
                let signature_type = FfiSignature::parse(signature)
                    .map_err(|error| CoreError::new(error.to_string()))?;
                let target = functions.get(name).ok_or_else(|| {
                    CoreError::new(format!(
                        "ffi_export_ptr target `{name}` is not a JIT function"
                    ))
                })?;
                if target.n_params != signature_type.args.len() {
                    return Err(CoreError::new(format!(
                        "ffi_export_ptr signature for `{name}` has {} arguments, function has {}",
                        signature_type.args.len(),
                        target.n_params
                    )));
                }
                let slot = export_slot(name, signature);
                callbacks.insert(slot, (slot, name.clone(), signature.clone()));
            }
        }
    }
    let mut callbacks: Vec<_> = callbacks.into_values().collect();
    callbacks.sort_by_key(|(slot, _, _)| *slot);
    Ok(callbacks)
}

impl Default for LumeCore {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LumeCore {
    fn drop(&mut self) {
        clear_jit_export_callbacks();
        crate::buffer::clear_jit_buffers();
    }
}

unsafe fn builtin_slice<'a>(args: *const i64, count: u64) -> Result<&'a [i64], String> {
    if count == 0 {
        return Ok(&[]);
    }
    if args.is_null() {
        return Err("JIT builtin received a null argument array".into());
    }
    Ok(unsafe { std::slice::from_raw_parts(args, count as usize) })
}

fn builtin_usize(value: i64, what: &str) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| format!("{what} must be non-negative"))
}

fn builtin_result(result: Result<i64, String>) -> i64 {
    match result {
        Ok(value) => value,
        Err(error) => {
            set_jit_error(error);
            -1
        }
    }
}

unsafe extern "C" fn ffi_export_ptr_builtin(args: *const i64, count: u64) -> i64 {
    builtin_result((|| {
        let args = unsafe { builtin_slice(args, count)? };
        if args.len() != 1 {
            return Err("ffi_export_ptr expects one callback slot".into());
        }
        let callback = lookup_jit_export_callback(args[0])
            .ok_or_else(|| format!("JIT callback slot {} is not registered", args[0]))?;
        Ok(unsafe { callback.code_ptr() } as i64)
    })())
}

unsafe extern "C" fn buf_new_builtin(args: *const i64, count: u64) -> i64 {
    builtin_result((|| {
        let args = unsafe { builtin_slice(args, count)? };
        if args.len() != 1 {
            return Err("buf_new expects one argument".into());
        }
        let length = builtin_usize(args[0], "buf_new length")?;
        let buffer = crate::buffer::with_current_heap(|heap| GcBuffer::new(heap, length))
            .map_err(|error| error.to_string())?;
        Ok(crate::buffer::jit_buffer_insert(buffer))
    })())
}

unsafe extern "C" fn buf_len_builtin(args: *const i64, count: u64) -> i64 {
    builtin_result((|| {
        let args = unsafe { builtin_slice(args, count)? };
        if args.len() != 1 {
            return Err("buf_len expects one argument".into());
        }
        let buffer = crate::buffer::jit_buffer_lookup(args[0])?;
        Ok(buffer.len() as i64)
    })())
}

unsafe extern "C" fn buf_get_u8_builtin(args: *const i64, count: u64) -> i64 {
    builtin_result((|| {
        let args = unsafe { builtin_slice(args, count)? };
        if args.len() != 2 {
            return Err("buf_get_u8 expects two arguments".into());
        }
        let buffer = crate::buffer::jit_buffer_lookup(args[0])?;
        let offset = builtin_usize(args[1], "buf_get_u8 offset")?;
        Ok(buffer.read_u8(offset).map_err(|error| error.to_string())? as i64)
    })())
}

unsafe extern "C" fn buf_set_u8_builtin(args: *const i64, count: u64) -> i64 {
    builtin_result((|| {
        let args = unsafe { builtin_slice(args, count)? };
        if args.len() != 3 {
            return Err("buf_set_u8 expects three arguments".into());
        }
        let buffer = crate::buffer::jit_buffer_lookup(args[0])?;
        let offset = builtin_usize(args[1], "buf_set_u8 offset")?;
        let value = u8::try_from(args[2]).map_err(|_| "buf_set_u8 value is out of range")?;
        buffer
            .write_u8(offset, value)
            .map_err(|error| error.to_string())?;
        Ok(0)
    })())
}

unsafe extern "C" fn buf_get_u32_builtin(args: *const i64, count: u64) -> i64 {
    builtin_result((|| {
        let args = unsafe { builtin_slice(args, count)? };
        if args.len() != 2 {
            return Err("buf_get_u32 expects two arguments".into());
        }
        let buffer = crate::buffer::jit_buffer_lookup(args[0])?;
        let offset = builtin_usize(args[1], "buf_get_u32 offset")?;
        Ok(buffer.read_u32(offset).map_err(|error| error.to_string())? as i64)
    })())
}

unsafe extern "C" fn buf_set_u32_builtin(args: *const i64, count: u64) -> i64 {
    builtin_result((|| {
        let args = unsafe { builtin_slice(args, count)? };
        if args.len() != 3 {
            return Err("buf_set_u32 expects three arguments".into());
        }
        let buffer = crate::buffer::jit_buffer_lookup(args[0])?;
        let offset = builtin_usize(args[1], "buf_set_u32 offset")?;
        let value = u32::try_from(args[2]).map_err(|_| "buf_set_u32 value is out of range")?;
        buffer
            .write_u32(offset, value)
            .map_err(|error| error.to_string())?;
        Ok(0)
    })())
}

unsafe fn copy_builtin(args: *const i64, count: u64, name: &str, overlapping: bool) -> i64 {
    builtin_result((|| {
        let args = unsafe { builtin_slice(args, count)? };
        let (dst_offset, src_offset, byte_count) = match args.len() {
            3 => (0usize, 0usize, builtin_usize(args[2], name)?),
            5 => (
                builtin_usize(args[1], name)?,
                builtin_usize(args[3], name)?,
                builtin_usize(args[4], name)?,
            ),
            _ => return Err(format!("{name} expects 3 or 5 arguments")),
        };
        let destination = crate::buffer::jit_buffer_lookup(args[0])?;
        let source = crate::buffer::jit_buffer_lookup(args[if args.len() == 3 { 1 } else { 2 }])?;
        let result = if overlapping {
            destination.memmove(dst_offset, &source, src_offset, byte_count)
        } else {
            destination.memcpy(dst_offset, &source, src_offset, byte_count)
        };
        result.map_err(|error| error.to_string())?;
        Ok(0)
    })())
}

unsafe extern "C" fn memcpy_builtin(args: *const i64, count: u64) -> i64 {
    unsafe { copy_builtin(args, count, "memcpy", false) }
}

unsafe extern "C" fn memmove_builtin(args: *const i64, count: u64) -> i64 {
    unsafe { copy_builtin(args, count, "memmove", true) }
}

fn jit_i64_to_ffi(ty: FfiType, value: i64) -> Result<FfiValue, String> {
    Ok(match ty {
        FfiType::Void => return Err("void is not a valid extern argument".into()),
        FfiType::I8 => FfiValue::Signed(value as i8 as i64),
        FfiType::I16 => FfiValue::Signed(value as i16 as i64),
        FfiType::I32 => FfiValue::Signed(value as i32 as i64),
        FfiType::I64 => FfiValue::Signed(value),
        FfiType::U8 => FfiValue::Unsigned(value as u8 as u64),
        FfiType::U16 => FfiValue::Unsigned(value as u16 as u64),
        FfiType::U32 => FfiValue::Unsigned(value as u32 as u64),
        FfiType::U64 => FfiValue::Unsigned(value as u64),
        FfiType::F32 => FfiValue::Float(f32::from_bits(value as u32) as f64),
        FfiType::F64 => FfiValue::Float(f64::from_bits(value as u64)),
        FfiType::Pointer => FfiValue::Pointer(value as *mut c_void),
        FfiType::Struct(_) | FfiType::Union(_) => {
            return Err("by-value C aggregates are not available in the JIT i64 ABI".into());
        }
    })
}

fn lower_external_call<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    callee: FunctionValue<'ctx>,
    arg_ids: &[ValueId],
    values: &[Option<BasicValueEnum<'ctx>>],
) -> Result<BasicValueEnum<'ctx>, CoreError> {
    let param_types = callee.get_type().get_param_types();
    if arg_ids.len() != param_types.len() {
        return Err(CoreError::new(format!(
            "external function `{}` expects {} arguments, received {}",
            callee.get_name().to_str().unwrap_or("<extern>"),
            param_types.len(),
            arg_ids.len()
        )));
    }
    let mut args = Vec::with_capacity(arg_ids.len());
    for (id, target) in arg_ids.iter().copied().zip(param_types) {
        let source = value(values, id)?;
        let coerced = if target.is_pointer_type() {
            builder
                .build_int_to_ptr(source, target.into_pointer_type(), "extern_arg_ptr")
                .map_err(build_error)?
                .into()
        } else if target.is_int_type() {
            let int_ty = target.into_int_type();
            let width = int_ty.get_bit_width();
            let source_width = source.get_type().get_bit_width();
            let adjusted = if source_width < width {
                builder
                    .build_int_s_extend(source, int_ty, "extern_arg_sext")
                    .map_err(build_error)?
            } else if source_width > width {
                builder
                    .build_int_truncate(source, int_ty, "extern_arg_trunc")
                    .map_err(build_error)?
            } else {
                source
            };
            adjusted.into()
        } else if target.is_float_type() {
            builder
                .build_signed_int_to_float(source, target.into_float_type(), "extern_arg_float")
                .map_err(build_error)?
                .into()
        } else {
            return Err(CoreError::new("unsupported extern parameter type"));
        };
        args.push(coerced);
    }

    let call = builder
        .build_call(callee, &args, "external_call")
        .map_err(build_error)?;
    let Some(result) = call.try_as_basic_value().basic() else {
        return Ok(context.i64_type().const_zero().into());
    };
    let i64_ty = context.i64_type();
    let converted = match result {
        BasicValueEnum::IntValue(int) => {
            let width = int.get_type().get_bit_width();
            if width < 64 {
                builder
                    .build_int_s_extend(int, i64_ty, "extern_result_sext")
                    .map_err(build_error)?
                    .into()
            } else if width > 64 {
                builder
                    .build_int_truncate(int, i64_ty, "extern_result_trunc")
                    .map_err(build_error)?
                    .into()
            } else {
                int.into()
            }
        }
        BasicValueEnum::PointerValue(pointer) => builder
            .build_ptr_to_int(pointer, i64_ty, "extern_result_ptr")
            .map_err(build_error)?
            .into(),
        BasicValueEnum::FloatValue(float) => builder
            .build_float_to_signed_int(float, i64_ty, "extern_result_float")
            .map_err(build_error)?
            .into(),
        _ => return Err(CoreError::new("unsupported extern return type")),
    };
    Ok(converted)
}

fn ffi_to_jit_i64(value: FfiValue) -> Result<i64, String> {
    Ok(match value {
        FfiValue::Void => 0,
        FfiValue::Signed(value) => value,
        FfiValue::Unsigned(value) => value as i64,
        FfiValue::Float(value) => value.to_bits() as i64,
        FfiValue::Pointer(pointer) => pointer as i64,
        FfiValue::String(_)
        | FfiValue::Callback(_)
        | FfiValue::Buffer(_)
        | FfiValue::Aggregate(_) => {
            return Err("extern return type cannot be represented in JIT i64 ABI".into());
        }
    })
}

fn compile_module<'ctx>(
    context: &'ctx Context,
    functions: &HashMap<String, IrFunc>,
    builtins: &HashMap<String, BuiltinFn>,
    bitcodes: &[Vec<u8>],
    externs: &HashMap<String, FfiSignature>,
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
    validate_calls(functions, builtins, &external_names, externs)?;
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
    externs: &HashMap<String, FfiSignature>,
) -> Result<(), CoreError> {
    for function in functions.values() {
        for op in function.blocks.iter().flat_map(|block| &block.ops) {
            if let IrOp::FfiExportPtr { .. } = op {
                if !builtins.contains_key("ffi_export_ptr") {
                    return Err(CoreError::new(
                        "ffi_export_ptr requires the default runtime builtins",
                    ));
                }
                continue;
            }
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
                    continue;
                }
                if let Some(signature) = externs.get(name) {
                    if args.len() != signature.args.len() {
                        return Err(CoreError::new(format!(
                            "call to extern `{name}` in `{}` passes {} arguments, expected {}",
                            function.name,
                            args.len(),
                            signature.args.len()
                        )));
                    }
                    if !builtins.contains_key(name) && !external_names.contains(name) {
                        return Err(CoreError::new(format!(
                            "extern `{name}` has no bitcode definition or dynamic library"
                        )));
                    }
                    continue;
                }
                if !builtins.contains_key(name) && !external_names.contains(name) {
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
            IrOp::FfiExportPtr { .. } => Some(1),
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
                IrOp::CastUInt(source, bits) => {
                    let source = value(&values, *source)?;
                    let mask = match bits {
                        8 => u64::from(u8::MAX),
                        16 => u64::from(u16::MAX),
                        32 => u64::from(u32::MAX),
                        64 => u64::MAX,
                        other => {
                            return Err(CoreError::new(format!(
                                "unsupported unsigned integer width {other}"
                            )));
                        }
                    };
                    let narrowed = builder
                        .build_and(source, i64_ty.const_int(mask, false), "cast_uint")
                        .map_err(build_error)?;
                    values[value_id] = Some(narrowed.into());
                }
                IrOp::FfiExportPtr { name, signature } => {
                    let slot = i64_ty.const_int(export_slot(name, signature) as u64, false);
                    let array =
                        call_args.ok_or_else(|| CoreError::new("missing call argument storage"))?;
                    let element = unsafe {
                        builder
                            .build_gep(i64_ty, array, &[i64_ty.const_zero()], "callback_slot")
                            .map_err(build_error)?
                    };
                    builder.build_store(element, slot).map_err(build_error)?;
                    let callee = functions.get("ffi_export_ptr").ok_or_else(|| {
                        CoreError::new("missing ffi_export_ptr builtin declaration")
                    })?;
                    let args: [BasicMetadataValueEnum; 2] =
                        [array.into(), i64_ty.const_int(1, false).into()];
                    let call = builder
                        .build_call(*callee, &args, "ffi_export_ptr")
                        .map_err(build_error)?;
                    values[value_id] =
                        Some(call.try_as_basic_value().basic().ok_or_else(|| {
                            CoreError::new("ffi_export_ptr did not produce a value")
                        })?);
                }
                IrOp::Call(name, arg_ids) => {
                    let callee = functions.get(name).ok_or_else(|| {
                        CoreError::new(format!("missing compiled function declaration `{name}`"))
                    })?;
                    if external_names.contains(name) {
                        values[value_id] = Some(lower_external_call(
                            context, &builder, *callee, arg_ids, &values,
                        )?);
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
