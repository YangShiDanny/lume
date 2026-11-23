//! GC-managed byte buffers for Lume.
//!
//! The first eight payload bytes store the buffer length in little-endian
//! order. Buffers are pinned for their whole lifetime so Lume values can keep
//! a stable payload pointer across collections.

use crate::gc::GcHeap;
use crate::typeobj::TypeId;
use std::cell::RefCell;
use std::fmt;
use std::ptr;
use std::rc::Rc;

const BUFFER_TYPE_ID: TypeId = TypeId(0x6275_6631); // "buf1"
const LEN_PREFIX: usize = std::mem::size_of::<u64>();

#[derive(Clone, Default)]
pub struct GcBufferHeap(Rc<RefCell<GcHeap>>);

#[derive(Clone)]
pub struct GcBuffer {
    heap: GcBufferHeap,
    payload: *mut u8,
}

impl fmt::Debug for GcBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcBuffer")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl GcBuffer {
    pub fn new(heap: &GcBufferHeap, len: usize) -> Result<Self, String> {
        let total = LEN_PREFIX
            .checked_add(len)
            .ok_or_else(|| format!("buffer length {len} overflows allocation size"))?;
        let mut gc = heap.0.borrow_mut();
        let payload = gc.alloc_typed(total, BUFFER_TYPE_ID);
        unsafe {
            ptr::write_unaligned(payload as *mut u64, len as u64);
            ptr::write_bytes(payload.add(LEN_PREFIX), 0, len);
        }
        gc.try_pin(payload)?;
        Ok(Self {
            heap: heap.clone(),
            payload,
        })
    }

    pub fn len(&self) -> usize {
        unsafe { ptr::read_unaligned(self.payload as *const u64) as usize }
    }

    fn data_ptr(&self) -> *mut u8 {
        unsafe { self.payload.add(LEN_PREFIX) }
    }

    fn check_range(&self, op: &str, offset: usize, size: usize) -> Result<(), String> {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| format!("{op}: offset {offset} overflows buffer range"))?;
        if end > self.len() {
            return Err(format!(
                "{op}: range {offset}..{end} is outside buffer of length {}",
                self.len()
            ));
        }
        Ok(())
    }

    pub fn read_u8(&self, offset: usize) -> Result<u8, String> {
        self.check_range("buf_get_u8", offset, 1)?;
        Ok(unsafe { *self.data_ptr().add(offset) })
    }

    pub fn write_u8(&self, offset: usize, value: u8) -> Result<(), String> {
        self.check_range("buf_set_u8", offset, 1)?;
        unsafe { *self.data_ptr().add(offset) = value };
        Ok(())
    }

    pub fn read_u32(&self, offset: usize) -> Result<u32, String> {
        self.check_range("buf_get_u32", offset, 4)?;
        let mut bytes = [0u8; 4];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = unsafe { *self.data_ptr().add(offset + index) };
        }
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn write_u32(&self, offset: usize, value: u32) -> Result<(), String> {
        self.check_range("buf_set_u32", offset, 4)?;
        let bytes = value.to_le_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            unsafe { *self.data_ptr().add(offset + index) = *byte };
        }
        Ok(())
    }

    pub fn write_bytes(&self, offset: usize, bytes: &[u8]) -> Result<(), String> {
        self.check_range("buf_write_bytes", offset, bytes.len())?;
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), self.data_ptr().add(offset), bytes.len()) };
        Ok(())
    }

    pub fn read_bytes(&self) -> Vec<u8> {
        unsafe { std::slice::from_raw_parts(self.data_ptr(), self.len()) }.to_vec()
    }

    pub fn memcpy(&self, dst_offset: usize, src: &GcBuffer, src_offset: usize, count: usize) -> Result<(), String> {
        self.check_range("memcpy destination", dst_offset, count)?;
        src.check_range("memcpy source", src_offset, count)?;
        if ptr::eq(self.payload, src.payload) && ranges_overlap(dst_offset, src_offset, count) {
            return Err("memcpy: source and destination ranges overlap; use a separate buffer".into());
        }
        unsafe {
            ptr::copy_nonoverlapping(
                src.data_ptr().add(src_offset),
                self.data_ptr().add(dst_offset),
                count,
            );
        }
        Ok(())
    }

    /// Stable pointer to the first byte of user data.
    pub fn as_ptr(&self) -> *mut u8 {
        self.data_ptr()
    }

    pub fn resolve(&self) -> *mut u8 {
        self.heap.0.borrow().resolve(self.payload)
    }
}

impl Drop for GcBuffer {
    fn drop(&mut self) {
        let resolved = self.resolve();
        let mut gc = self.heap.0.borrow_mut();
        let _ = gc.try_unpin(resolved);
    }
}

fn ranges_overlap(a: usize, b: usize, len: usize) -> bool {
    len > 0 && a < b.saturating_add(len) && b < a.saturating_add(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_values_round_trip_little_endian_and_check_bounds() {
        let heap = GcBufferHeap::default();
        let buffer = GcBuffer::new(&heap, 8).unwrap();
        assert_eq!(buffer.len(), 8);

        buffer.write_u8(0, 0xab).unwrap();
        buffer.write_u32(4, 0x0102_0304).unwrap();
        assert_eq!(buffer.read_u8(0).unwrap(), 0xab);
        assert_eq!(buffer.read_u32(4).unwrap(), 0x0102_0304);
        assert_eq!(buffer.read_bytes()[4..8], [4, 3, 2, 1]);

        assert!(buffer.read_u8(8).is_err());
        assert!(buffer.write_u32(5, 1).is_err());
    }

    #[test]
    fn memcpy_rejects_overlap_and_copies_between_buffers() {
        let heap = GcBufferHeap::default();
        let source = GcBuffer::new(&heap, 4).unwrap();
        let destination = GcBuffer::new(&heap, 4).unwrap();
        source.write_u32(0, 0xdead_beef).unwrap();
        destination.memcpy(0, &source, 0, 4).unwrap();
        assert_eq!(destination.read_u32(0).unwrap(), 0xdead_beef);
        assert!(destination.memcpy(0, &destination, 1, 3).is_err());
    }

    #[test]
    fn dropping_a_buffer_releases_its_pin() {
        let heap = GcBufferHeap::default();
        let buffer = GcBuffer::new(&heap, 4).unwrap();
        let payload = buffer.resolve();
        assert!(unsafe { heap.0.borrow().is_pinned(payload) });
        drop(buffer);
        assert!(!unsafe { heap.0.borrow().is_pinned(payload) });
    }
}
