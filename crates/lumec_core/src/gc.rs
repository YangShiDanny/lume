use crate::typeobj::TypeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct GcHeader {
    pub marked: bool,
    pub size: usize,
    pub type_id: TypeId,
}

impl GcHeader {
    pub const fn new(size: usize, type_id: TypeId) -> Self {
        Self {
            marked: false,
            size,
            type_id,
        }
    }
}

#[derive(Debug, Default)]
pub struct GcMarker;

impl GcMarker {
    pub fn mark_header(&mut self, header: &mut GcHeader) {
        header.marked = true;
    }
}

pub trait GcTrace {
    fn trace(&mut self, marker: &mut GcMarker);
}

/// Minimal bump allocator reserved for GC-managed core objects.
///
/// This allocator deliberately has no collection semantics yet. It provides
/// stable object storage for the GC skeleton and builtin type allocation; a
/// later mark-and-sweep implementation can replace allocation and collection
/// without changing the public GC header or trace interfaces.
#[derive(Debug, Default)]
pub struct GcHeap {
    memory: Vec<Box<[u8]>>,
    roots: Vec<*mut u8>,
}

impl GcHeap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self, size: usize) -> *mut u8 {
        let align = std::mem::align_of::<usize>();
        let padded_size = (size + align - 1) & !(align - 1);
        let mut chunk = vec![0u8; padded_size].into_boxed_slice();
        let ptr = chunk.as_mut_ptr();
        self.memory.push(chunk);
        ptr
    }

    pub fn push_root(&mut self, ptr: *mut u8) {
        self.roots.push(ptr);
    }

    pub fn pop_root(&mut self) {
        self.roots.pop();
    }

    pub fn roots(&self) -> &[*mut u8] {
        &self.roots
    }

    pub fn collect(&mut self) {
        // Intentionally a no-op until mark/sweep object tracing is implemented.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_can_mark_an_object_header() {
        let mut header = GcHeader::new(16, TypeId(1));
        GcMarker.mark_header(&mut header);
        assert!(header.marked);
    }

    #[test]
    fn bump_heap_allocates_distinct_aligned_objects() {
        let mut heap = GcHeap::new();
        let first = heap.alloc(16) as usize;
        let second = heap.alloc(16) as usize;
        assert!(first.is_multiple_of(std::mem::align_of::<usize>()));
        assert_ne!(second, first);
    }
}
