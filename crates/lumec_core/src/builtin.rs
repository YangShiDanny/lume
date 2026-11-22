use crate::gc::GcHeap;
use crate::typeobj::{TypeId, TypeKind, TypeObj};

#[derive(Debug)]
pub struct BuiltinTypes {
    pub any: *mut TypeObj,
    pub int64: *mut TypeObj,
    pub float64: *mut TypeObj,
    pub bool_ty: *mut TypeObj,
}

impl BuiltinTypes {
    pub fn init(heap: &mut GcHeap) -> Self {
        let any = heap.alloc(std::mem::size_of::<TypeObj>()) as *mut TypeObj;
        unsafe {
            *any = TypeObj {
                id: TypeId(0),
                name: "Any".into(),
                kind: TypeKind::Any,
            };
        }
        let int64 = heap.alloc(std::mem::size_of::<TypeObj>()) as *mut TypeObj;
        unsafe {
            *int64 = TypeObj {
                id: TypeId(1),
                name: "Int64".into(),
                kind: TypeKind::Int64,
            };
        }
        let float64 = heap.alloc(std::mem::size_of::<TypeObj>()) as *mut TypeObj;
        unsafe {
            *float64 = TypeObj {
                id: TypeId(2),
                name: "Float64".into(),
                kind: TypeKind::Float64,
            };
        }
        let bool_ty = heap.alloc(std::mem::size_of::<TypeObj>()) as *mut TypeObj;
        unsafe {
            *bool_ty = TypeObj {
                id: TypeId(3),
                name: "Bool".into(),
                kind: TypeKind::Bool,
            };
        }
        Self {
            any,
            int64,
            float64,
            bool_ty,
        }
    }
}
