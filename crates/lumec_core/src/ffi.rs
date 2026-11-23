//! Dynamic C FFI. Native calls are explicitly unsafe because symbol signatures
//! and pointer validity cannot be verified from a shared library.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ffi::{CString, c_void};
use std::fmt;
use std::path::Path;
use std::ptr::{self, NonNull};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use libffi::low;
use libffi::middle::{Arg, Cif, Closure, CodePtr, Ret, Type, arg};
use libloading::Library;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiError(pub String);

impl fmt::Display for FfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FfiError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiType {
    Void,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Pointer,
}

impl FfiType {
    pub fn parse(name: &str) -> Result<Self, FfiError> {
        match name.trim() {
            "void" => Ok(Self::Void),
            "i8" => Ok(Self::I8),
            "i16" => Ok(Self::I16),
            "i32" => Ok(Self::I32),
            "i64" => Ok(Self::I64),
            "isize" => Ok(if usize::BITS == 32 {
                Self::I32
            } else {
                Self::I64
            }),
            "u8" | "bool" => Ok(Self::U8),
            "u16" => Ok(Self::U16),
            "u32" => Ok(Self::U32),
            "u64" => Ok(Self::U64),
            "usize" => Ok(if usize::BITS == 32 {
                Self::U32
            } else {
                Self::U64
            }),
            "f32" => Ok(Self::F32),
            "f64" => Ok(Self::F64),
            "ptr" | "pointer" => Ok(Self::Pointer),
            other => Err(FfiError(format!("unsupported C FFI type `{other}`"))),
        }
    }

    fn libffi_type(self) -> Type {
        match self {
            Self::Void => Type::void(),
            Self::I8 => Type::i8(),
            Self::I16 => Type::i16(),
            Self::I32 => Type::i32(),
            Self::I64 => Type::i64(),
            Self::U8 => Type::u8(),
            Self::U16 => Type::u16(),
            Self::U32 => Type::u32(),
            Self::U64 => Type::u64(),
            Self::F32 => Type::f32(),
            Self::F64 => Type::f64(),
            Self::Pointer => Type::pointer(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiSignature {
    pub result: FfiType,
    pub args: Vec<FfiType>,
}

impl FfiSignature {
    pub fn new(result: FfiType, args: Vec<FfiType>) -> Result<Self, FfiError> {
        if args.contains(&FfiType::Void) {
            return Err(FfiError("void is only valid as a C function result".into()));
        }
        Ok(Self { result, args })
    }

    /// Parses signatures such as `i32(pointer, u64)` or `void()`.
    pub fn parse(spec: &str) -> Result<Self, FfiError> {
        let open = spec
            .find('(')
            .ok_or_else(|| FfiError("FFI signature is missing `(`".into()))?;
        if !spec.ends_with(')') {
            return Err(FfiError("FFI signature must end with `)`".into()));
        }
        let result = FfiType::parse(&spec[..open])?;
        let params = &spec[open + 1..spec.len() - 1];
        let args = if params.trim().is_empty() {
            Vec::new()
        } else {
            params
                .split(',')
                .map(FfiType::parse)
                .collect::<Result<Vec<_>, _>>()?
        };
        Self::new(result, args)
    }

    fn cif(&self) -> Result<Cif, FfiError> {
        Cif::try_new(
            self.args
                .iter()
                .copied()
                .map(FfiType::libffi_type)
                .collect::<Vec<_>>(),
            self.result.libffi_type(),
        )
        .map_err(|error| FfiError(format!("cannot prepare C ABI signature: {error:?}")))
    }
}

#[derive(Clone)]
pub enum FfiValue {
    Void,
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Pointer(*mut c_void),
    String(String),
    Callback(Rc<ForeignCallback>),
    Buffer(Rc<ForeignBuffer>),
}

impl fmt::Debug for FfiValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Void => f.write_str("Void"),
            Self::Signed(value) => f.debug_tuple("Signed").field(value).finish(),
            Self::Unsigned(value) => f.debug_tuple("Unsigned").field(value).finish(),
            Self::Float(value) => f.debug_tuple("Float").field(value).finish(),
            Self::Pointer(value) => f.debug_tuple("Pointer").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::Callback(_) => f.write_str("Callback(..)"),
            Self::Buffer(buffer) => f.debug_tuple("Buffer").field(&buffer.size()).finish(),
        }
    }
}

pub struct ForeignBuffer {
    pointer: NonNull<u8>,
    layout: Layout,
    size: usize,
}

impl fmt::Debug for ForeignBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForeignBuffer")
            .field("pointer", &self.pointer)
            .field("size", &self.size())
            .finish()
    }
}

impl ForeignBuffer {
    pub fn new(size: usize, alignment: usize) -> Result<Rc<Self>, FfiError> {
        let layout = Layout::from_size_align(size.max(1), alignment)
            .map_err(|error| FfiError(format!("invalid FFI buffer layout: {error}")))?;
        let pointer = NonNull::new(unsafe { alloc_zeroed(layout) })
            .ok_or_else(|| FfiError("cannot allocate FFI buffer".into()))?;
        Ok(Rc::new(Self {
            pointer,
            layout,
            size,
        }))
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.pointer.as_ptr().cast()
    }

    pub fn read(&self, offset: usize, ty: FfiType) -> Result<FfiValue, FfiError> {
        if ty == FfiType::Void {
            return Err(FfiError(
                "cannot read a void value from an FFI buffer".into(),
            ));
        }
        let pointer = self.field_ptr(offset, native_type_size(ty))?;
        unsafe { FfiValue::read_callback_arg(ty, pointer.cast()) }
    }

    pub fn write(&self, offset: usize, ty: FfiType, value: &FfiValue) -> Result<(), FfiError> {
        if ty == FfiType::Void {
            return Err(FfiError(
                "cannot write a void value to an FFI buffer".into(),
            ));
        }
        let destination = self.field_ptr(offset, native_type_size(ty))?.cast::<u8>();
        let native = NativeArg::new(ty, value)?;
        unsafe {
            match native {
                NativeArg::I8(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::I16(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::I32(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::I64(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::U8(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::U16(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::U32(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::U64(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::F32(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::F64(value) => ptr::write_unaligned(destination.cast(), value),
                NativeArg::Pointer {
                    _string: Some(_), ..
                } => {
                    return Err(FfiError(
                        "temporary C strings cannot be stored in an FFI buffer".into(),
                    ));
                }
                NativeArg::Pointer { address, .. } => {
                    ptr::write_unaligned(destination.cast(), address)
                }
            }
        }
        Ok(())
    }

    pub fn write_c_string(&self, offset: usize, value: &str) -> Result<(), FfiError> {
        let value = CString::new(value)
            .map_err(|_| FfiError("C string contains an interior NUL byte".into()))?;
        let bytes = value.as_bytes_with_nul();
        let destination = self.field_ptr(offset, bytes.len())?;
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast(), bytes.len()) };
        Ok(())
    }

    pub fn read_c_string(&self, offset: usize, max_len: usize) -> Result<String, FfiError> {
        let source = self.field_ptr(offset, max_len)?;
        let bytes =
            unsafe { std::slice::from_raw_parts(source.cast::<u8>().cast_const(), max_len) };
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| FfiError("FFI buffer string is not NUL terminated".into()))?;
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }

    fn field_ptr(&self, offset: usize, size: usize) -> Result<*mut c_void, FfiError> {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| FfiError("FFI buffer offset overflow".into()))?;
        if end > self.size() {
            return Err(FfiError(format!(
                "FFI buffer access {offset}..{end} exceeds buffer size {}",
                self.size()
            )));
        }
        Ok(unsafe { self.pointer.as_ptr().add(offset).cast() })
    }
}

impl Drop for ForeignBuffer {
    fn drop(&mut self) {
        unsafe { dealloc(self.pointer.as_ptr(), self.layout) };
    }
}

fn native_type_size(ty: FfiType) -> usize {
    match ty {
        FfiType::Void => 0,
        FfiType::I8 | FfiType::U8 => 1,
        FfiType::I16 | FfiType::U16 => 2,
        FfiType::I32 | FfiType::U32 | FfiType::F32 => 4,
        FfiType::I64 | FfiType::U64 | FfiType::F64 => 8,
        FfiType::Pointer => std::mem::size_of::<*mut c_void>(),
    }
}

pub struct ForeignLibrary {
    library: Library,
}

impl fmt::Debug for ForeignLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForeignLibrary").finish_non_exhaustive()
    }
}

impl ForeignLibrary {
    /// Loads a native library. Loading may execute arbitrary code in that library.
    ///
    /// # Safety
    /// The caller must trust the library and its initialization routines.
    pub unsafe fn open(path: impl AsRef<Path>) -> Result<Arc<Self>, FfiError> {
        let path = path.as_ref();
        let library = unsafe { Library::new(path) }
            .map_err(|error| FfiError(format!("cannot load {}: {error}", path.display())))?;
        Ok(Arc::new(Self { library }))
    }

    pub fn symbol(self: &Arc<Self>, name: &str) -> Result<ForeignSymbol, FfiError> {
        let name_c = CString::new(name)
            .map_err(|_| FfiError("symbol name contains an interior NUL byte".into()))?;
        let address = unsafe {
            *self
                .library
                .get::<*mut c_void>(name_c.as_bytes_with_nul())
                .map_err(|error| FfiError(format!("cannot resolve symbol `{name}`: {error}")))?
        };
        let address = NonNull::new(address)
            .ok_or_else(|| FfiError(format!("symbol `{name}` resolved to a null address")))?;
        Ok(ForeignSymbol {
            _library: Arc::clone(self),
            address,
        })
    }
}

pub struct ForeignSymbol {
    _library: Arc<ForeignLibrary>,
    address: NonNull<c_void>,
}

impl fmt::Debug for ForeignSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForeignSymbol")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl ForeignSymbol {
    /// Calls a native function using the supplied C ABI signature.
    ///
    /// # Safety
    /// `signature` must exactly match the function's calling convention and C types.
    /// Pointer arguments must remain valid for the duration required by the callee.
    pub unsafe fn call(
        &self,
        signature: &FfiSignature,
        args: &[FfiValue],
    ) -> Result<FfiValue, FfiError> {
        if args.len() != signature.args.len() {
            return Err(FfiError(format!(
                "C function expects {} arguments, received {}",
                signature.args.len(),
                args.len()
            )));
        }

        let native_args = signature
            .args
            .iter()
            .copied()
            .zip(args)
            .map(|(ty, value)| NativeArg::new(ty, value))
            .collect::<Result<Vec<_>, _>>()?;
        let ffi_args = native_args
            .iter()
            .map(NativeArg::as_arg)
            .collect::<Vec<_>>();
        let cif = signature.cif()?;
        let mut result = NativeValue::zero(signature.result);
        unsafe {
            result.call_into(&cif, CodePtr(self.address.as_ptr()), &ffi_args);
        }
        Ok(result.into_ffi_value())
    }
}

enum NativeArg {
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    Pointer {
        address: *mut c_void,
        _string: Option<CString>,
    },
}

impl NativeArg {
    fn new(ty: FfiType, value: &FfiValue) -> Result<Self, FfiError> {
        let mismatch = || FfiError(format!("value {value:?} does not match C type {ty:?}"));
        Ok(match (ty, value) {
            (FfiType::I8, FfiValue::Signed(value)) => Self::I8(
                i8::try_from(*value).map_err(|_| FfiError("i8 argument is out of range".into()))?,
            ),
            (FfiType::I16, FfiValue::Signed(value)) => Self::I16(
                i16::try_from(*value)
                    .map_err(|_| FfiError("i16 argument is out of range".into()))?,
            ),
            (FfiType::I32, FfiValue::Signed(value)) => Self::I32(
                i32::try_from(*value)
                    .map_err(|_| FfiError("i32 argument is out of range".into()))?,
            ),
            (FfiType::I64, FfiValue::Signed(value)) => Self::I64(*value),
            (FfiType::U8, FfiValue::Unsigned(value)) => Self::U8(
                u8::try_from(*value).map_err(|_| FfiError("u8 argument is out of range".into()))?,
            ),
            (FfiType::U16, FfiValue::Unsigned(value)) => Self::U16(
                u16::try_from(*value)
                    .map_err(|_| FfiError("u16 argument is out of range".into()))?,
            ),
            (FfiType::U32, FfiValue::Unsigned(value)) => Self::U32(
                u32::try_from(*value)
                    .map_err(|_| FfiError("u32 argument is out of range".into()))?,
            ),
            (FfiType::U64, FfiValue::Unsigned(value)) => Self::U64(*value),
            (FfiType::F32, FfiValue::Float(value)) => Self::F32(*value as f32),
            (FfiType::F64, FfiValue::Float(value)) => Self::F64(*value),
            (FfiType::Pointer, FfiValue::Pointer(address)) => Self::Pointer {
                address: *address,
                _string: None,
            },
            (FfiType::Pointer, FfiValue::String(value)) => {
                let string = CString::new(value.as_str()).map_err(|_| {
                    FfiError("C string argument contains an interior NUL byte".into())
                })?;
                let address = string.as_ptr() as *mut c_void;
                Self::Pointer {
                    address,
                    _string: Some(string),
                }
            }
            (FfiType::Pointer, FfiValue::Callback(callback)) => Self::Pointer {
                address: unsafe { callback.code_ptr() },
                _string: None,
            },
            (FfiType::Pointer, FfiValue::Buffer(buffer)) => Self::Pointer {
                address: buffer.as_ptr(),
                _string: None,
            },
            _ => return Err(mismatch()),
        })
    }

    fn as_arg(&self) -> Arg<'_> {
        match self {
            Self::I8(value) => arg(value),
            Self::I16(value) => arg(value),
            Self::I32(value) => arg(value),
            Self::I64(value) => arg(value),
            Self::U8(value) => arg(value),
            Self::U16(value) => arg(value),
            Self::U32(value) => arg(value),
            Self::U64(value) => arg(value),
            Self::F32(value) => arg(value),
            Self::F64(value) => arg(value),
            Self::Pointer { address, .. } => arg(address),
        }
    }
}

enum NativeValue {
    Void,
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    Pointer(*mut c_void),
}

impl NativeValue {
    fn zero(ty: FfiType) -> Self {
        match ty {
            FfiType::Void => Self::Void,
            FfiType::I8 => Self::I8(0),
            FfiType::I16 => Self::I16(0),
            FfiType::I32 => Self::I32(0),
            FfiType::I64 => Self::I64(0),
            FfiType::U8 => Self::U8(0),
            FfiType::U16 => Self::U16(0),
            FfiType::U32 => Self::U32(0),
            FfiType::U64 => Self::U64(0),
            FfiType::F32 => Self::F32(0.0),
            FfiType::F64 => Self::F64(0.0),
            FfiType::Pointer => Self::Pointer(ptr::null_mut()),
        }
    }

    unsafe fn call_into(&mut self, cif: &Cif, function: CodePtr, args: &[Arg<'_>]) {
        match self {
            Self::Void => unsafe { cif.call_return_into(function, args, Ret::void()) },
            Self::I8(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::I16(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::I32(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::I64(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::U8(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::U16(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::U32(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::U64(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::F32(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::F64(value) => unsafe { cif.call_return_into(function, args, Ret::new(value)) },
            Self::Pointer(value) => unsafe {
                cif.call_return_into(function, args, Ret::new(value))
            },
        }
    }

    fn into_ffi_value(self) -> FfiValue {
        match self {
            Self::Void => FfiValue::Void,
            Self::I8(value) => FfiValue::Signed(value.into()),
            Self::I16(value) => FfiValue::Signed(value.into()),
            Self::I32(value) => FfiValue::Signed(value.into()),
            Self::I64(value) => FfiValue::Signed(value),
            Self::U8(value) => FfiValue::Unsigned(value.into()),
            Self::U16(value) => FfiValue::Unsigned(value.into()),
            Self::U32(value) => FfiValue::Unsigned(value.into()),
            Self::U64(value) => FfiValue::Unsigned(value),
            Self::F32(value) => FfiValue::Float(value.into()),
            Self::F64(value) => FfiValue::Float(value),
            Self::Pointer(value) => FfiValue::Pointer(value),
        }
    }
}

type CallbackHandler = dyn Fn(&[FfiValue]) -> Result<FfiValue, String>;

struct CallbackData {
    signature: FfiSignature,
    handler: Box<CallbackHandler>,
    last_error: Mutex<Option<String>>,
}

#[repr(C, align(8))]
#[derive(Clone, Copy, Default)]
struct CallbackResult([u64; 2]);

unsafe extern "C" fn callback_dispatch(
    _cif: &low::ffi_cif,
    result: &mut CallbackResult,
    arguments: *const *const c_void,
    data: &CallbackData,
) {
    result.0 = [0; 2];
    let pointers = if data.signature.args.is_empty() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(arguments, data.signature.args.len()) }
    };
    let values = data
        .signature
        .args
        .iter()
        .copied()
        .zip(pointers.iter().copied())
        .map(|(ty, ptr)| unsafe { FfiValue::read_callback_arg(ty, ptr) })
        .collect::<Result<Vec<_>, _>>();
    let response = match values {
        Err(error) => Err(error.to_string()),
        Ok(values) => {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (data.handler)(&values)))
                .unwrap_or_else(|panic| {
                    let message = panic
                        .downcast_ref::<&str>()
                        .map(|message| (*message).to_owned())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "callback panicked".into());
                    Err(message)
                })
        }
    };
    match response {
        Ok(value) => {
            if let Err(error) =
                unsafe { write_callback_result(data.signature.result, value, result) }
            {
                *data
                    .last_error
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = Some(error.0);
            }
        }
        Err(error) => {
            *data
                .last_error
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(error);
        }
    }
}

impl FfiValue {
    unsafe fn read_callback_arg(ty: FfiType, pointer: *const c_void) -> Result<Self, FfiError> {
        if pointer.is_null() {
            return Err(FfiError("C callback provided a null argument slot".into()));
        }
        let value = match ty {
            FfiType::Void => return Err(FfiError("void callback argument is invalid".into())),
            FfiType::I8 => {
                Self::Signed(unsafe { ptr::read_unaligned(pointer.cast::<i8>()) }.into())
            }
            FfiType::I16 => {
                Self::Signed(unsafe { ptr::read_unaligned(pointer.cast::<i16>()) }.into())
            }
            FfiType::I32 => {
                Self::Signed(unsafe { ptr::read_unaligned(pointer.cast::<i32>()) }.into())
            }
            FfiType::I64 => Self::Signed(unsafe { ptr::read_unaligned(pointer.cast::<i64>()) }),
            FfiType::U8 => {
                Self::Unsigned(unsafe { ptr::read_unaligned(pointer.cast::<u8>()) }.into())
            }
            FfiType::U16 => {
                Self::Unsigned(unsafe { ptr::read_unaligned(pointer.cast::<u16>()) }.into())
            }
            FfiType::U32 => {
                Self::Unsigned(unsafe { ptr::read_unaligned(pointer.cast::<u32>()) }.into())
            }
            FfiType::U64 => Self::Unsigned(unsafe { ptr::read_unaligned(pointer.cast::<u64>()) }),
            FfiType::F32 => {
                Self::Float(unsafe { ptr::read_unaligned(pointer.cast::<f32>()) }.into())
            }
            FfiType::F64 => Self::Float(unsafe { ptr::read_unaligned(pointer.cast::<f64>()) }),
            FfiType::Pointer => {
                Self::Pointer(unsafe { ptr::read_unaligned(pointer.cast::<*mut c_void>()) })
            }
        };
        Ok(value)
    }
}

unsafe fn write_callback_result(
    ty: FfiType,
    value: FfiValue,
    result: &mut CallbackResult,
) -> Result<(), FfiError> {
    if ty == FfiType::Void {
        return if matches!(value, FfiValue::Void) {
            Ok(())
        } else {
            Err(FfiError("void callback must return no value".into()))
        };
    }
    let destination = result.0.as_mut_ptr().cast::<u8>();
    let native = NativeArg::new(ty, &value)?;
    unsafe {
        match native {
            NativeArg::I8(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::I16(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::I32(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::I64(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::U8(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::U16(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::U32(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::U64(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::F32(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::F64(value) => ptr::write_unaligned(destination.cast(), value),
            NativeArg::Pointer { address, .. } => ptr::write_unaligned(destination.cast(), address),
        }
    }
    Ok(())
}

pub struct ForeignCallback {
    closure: Option<Closure<'static>>,
    data: Option<Box<CallbackData>>,
}

impl fmt::Debug for ForeignCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForeignCallback").finish_non_exhaustive()
    }
}

impl ForeignCallback {
    pub fn new(
        signature: FfiSignature,
        handler: impl Fn(&[FfiValue]) -> Result<FfiValue, String> + 'static,
    ) -> Result<Rc<Self>, FfiError> {
        let cif = signature.cif()?;
        let data = Box::new(CallbackData {
            signature,
            handler: Box::new(handler),
            last_error: Mutex::new(None),
        });
        let data_ref = unsafe { &*(&*data as *const CallbackData) };
        let closure = Closure::try_new(cif, callback_dispatch, data_ref)
            .map_err(|error| FfiError(format!("cannot create C callback: {error:?}")))?;
        Ok(Rc::new(Self {
            closure: Some(closure),
            data: Some(data),
        }))
    }

    /// Gets the generated C function pointer.
    ///
    /// # Safety
    /// The pointer must not be called or retained after this callback is dropped.
    pub unsafe fn code_ptr(&self) -> *mut c_void {
        let function = self
            .closure
            .as_ref()
            .expect("callback closure remains alive")
            .code_ptr();
        CodePtr::from_fun(*function).0
    }

    pub fn take_error(&self) -> Result<Option<String>, FfiError> {
        self.data
            .as_ref()
            .ok_or_else(|| FfiError("callback data is unavailable".into()))?
            .last_error
            .lock()
            .map(|mut error| error.take())
            .map_err(|_| FfiError("callback error state is poisoned".into()))
    }
}

impl Drop for ForeignCallback {
    fn drop(&mut self) {
        drop(self.closure.take());
        drop(self.data.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_signatures_and_rejects_void_arguments() {
        assert_eq!(
            FfiSignature::parse("i32(pointer, u64)").unwrap(),
            FfiSignature::new(FfiType::I32, vec![FfiType::Pointer, FfiType::U64]).unwrap()
        );
        assert!(FfiSignature::parse("void(void)").is_err());
        assert!(FfiSignature::parse("i32(i128)").is_err());
    }

    #[test]
    fn ffi_buffers_are_aligned_bounded_and_support_c_strings() {
        let buffer = ForeignBuffer::new(16, 8).unwrap();
        buffer
            .write(1, FfiType::U32, &FfiValue::Unsigned(0x1234_5678))
            .unwrap();
        assert!(matches!(
            buffer.read(1, FfiType::U32).unwrap(),
            FfiValue::Unsigned(0x1234_5678)
        ));
        buffer.write_c_string(8, "Lume").unwrap();
        assert_eq!(buffer.read_c_string(8, 8).unwrap(), "Lume");
        assert!(buffer.read(15, FfiType::U32).is_err());
        assert!(buffer.read_c_string(8, 3).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn opens_shared_library_and_calls_typed_c_symbol() {
        let library = unsafe { ForeignLibrary::open("libc.so.6") }.unwrap();
        let abs = library.symbol("abs").unwrap();
        let signature = FfiSignature::parse("i32(i32)").unwrap();
        let result = unsafe { abs.call(&signature, &[FfiValue::Signed(-42)]) }.unwrap();
        assert!(matches!(result, FfiValue::Signed(42)));

        let strlen = library.symbol("strlen").unwrap();
        let signature = FfiSignature::parse("usize(pointer)").unwrap();
        let result =
            unsafe { strlen.call(&signature, &[FfiValue::String("Lume".into())]) }.unwrap();
        assert!(matches!(result, FfiValue::Unsigned(4)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn passes_ffi_callback_to_a_c_sort_routine() {
        let library = unsafe { ForeignLibrary::open("libc.so.6") }.unwrap();
        let qsort = library.symbol("qsort").unwrap();
        let signature = FfiSignature::parse("i32(pointer, pointer)").unwrap();
        let callback = ForeignCallback::new(signature, |args| {
            let [FfiValue::Pointer(lhs), FfiValue::Pointer(rhs)] = args else {
                return Err("qsort passed invalid comparator arguments".into());
            };
            let lhs = unsafe { *(*lhs).cast::<i32>() };
            let rhs = unsafe { *(*rhs).cast::<i32>() };
            Ok(FfiValue::Signed((lhs - rhs).into()))
        })
        .unwrap();
        let mut values = [5i32, -2, 9, 0];
        let signature = FfiSignature::parse("void(pointer, usize, usize, pointer)").unwrap();
        let args = [
            FfiValue::Pointer(values.as_mut_ptr().cast()),
            FfiValue::Unsigned(values.len() as u64),
            FfiValue::Unsigned(std::mem::size_of::<i32>() as u64),
            FfiValue::Callback(Rc::clone(&callback)),
        ];
        unsafe { qsort.call(&signature, &args) }.unwrap();
        assert_eq!(values, [-2, 0, 5, 9]);
        assert_eq!(callback.take_error().unwrap(), None);
    }

    #[test]
    fn c_callback_invokes_handler_and_reports_errors_without_unwinding() {
        let signature = FfiSignature::parse("i32(i32)").unwrap();
        let callback = ForeignCallback::new(signature.clone(), |args| match args {
            [FfiValue::Signed(value)] => Ok(FfiValue::Signed(value + 1)),
            _ => Err("unexpected callback arguments".into()),
        })
        .unwrap();
        let cif = signature.cif().unwrap();
        let input = 41i32;
        let callback_ptr = unsafe { callback.code_ptr() };
        let result = unsafe { cif.call::<i32>(CodePtr(callback_ptr), &[arg(&input)]) };
        assert_eq!(result, 42);
        assert_eq!(callback.take_error().unwrap(), None);

        let failing =
            ForeignCallback::new(signature.clone(), |_| Err("callback failure".into())).unwrap();
        let callback_ptr = unsafe { failing.code_ptr() };
        let result = unsafe { cif.call::<i32>(CodePtr(callback_ptr), &[arg(&input)]) };
        assert_eq!(result, 0);
        assert_eq!(
            failing.take_error().unwrap().as_deref(),
            Some("callback failure")
        );
    }
}
