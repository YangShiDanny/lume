//! Core 的 GC 门面：底层使用 `lumec_immix` 的 mark-region 收集器。
//!
//! 保留 `alloc` / `roots` / `collect` 等旧门面，同时对 root 注册和弹出显式报告
//! 错误，防止非堆指针或 root 栈下溢被静默忽略。
//!
//! 指针约定：本模块对外暴露与接收的都是**负载指针**（对象 header 之后的起始地址），
//! 而 Immix 内部使用对象起始地址（header）。转换由本模块完成。

use std::fmt;

use lumec_immix::{CollectStats, Header as ImmixHeader, ImmixHeap};

use crate::typeobj::TypeId;

/// 旧版 GC 头，保留以兼容已有 API；实际对象元数据由 Immix 的 `Header` 管理。
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

/// Immix 堆的 core 适配层。
pub struct GcHeap {
    heap: ImmixHeap,
}

pub struct GcPin<'heap> {
    heap: &'heap mut GcHeap,
    payload: *mut u8,
}

impl GcPin<'_> {
    pub fn as_ptr(&self) -> *mut u8 {
        self.payload
    }
}

impl Drop for GcPin<'_> {
    fn drop(&mut self) {
        if let Some(object) = self.heap.heap.object_from_payload(self.payload) {
            unsafe { ImmixHeap::unpin(object) };
        }
    }
}

impl Default for GcHeap {
    fn default() -> Self {
        Self {
            heap: ImmixHeap::new(),
        }
    }
}

impl fmt::Debug for GcHeap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcHeap").field("heap", &self.heap).finish()
    }
}

impl GcHeap {
    pub fn new() -> Self {
        Self::default()
    }

    /// 自定义 line/block 尺寸；参数会被 Immix 校验。
    pub fn with_options(line_size: usize, lines_per_block: usize) -> Self {
        Self {
            heap: ImmixHeap::with_options(line_size, lines_per_block),
        }
    }

    /// 分配 `size` 字节的负载，返回负载指针。
    pub fn alloc(&mut self, size: usize) -> *mut u8 {
        self.alloc_typed(size, TypeId(0))
    }

    /// 分配带类型标记的负载，返回负载指针。类型标记会写进 Immix header。
    pub fn alloc_typed(&mut self, size: usize, type_id: TypeId) -> *mut u8 {
        let object = self
            .heap
            .alloc(size, type_id.0)
            .expect("immix allocation failed");
        unsafe { ImmixHeap::payload::<u8>(object) }
    }

    /// 分配并写入一个 Rust 值，返回负载指针。
    pub fn alloc_value<T>(&mut self, value: T, type_id: TypeId) -> *mut u8 {
        let object = self
            .heap
            .alloc_value(value, type_id.0)
            .expect("immix allocation failed");
        unsafe { ImmixHeap::payload::<u8>(object) }
    }

    /// 登记负载指针为 GC 根。
    pub fn push_root(&mut self, ptr: *mut u8) -> Result<(), String> {
        if ptr.is_null() {
            return Err("null pointer cannot be used as a GC root".into());
        }
        let object = self
            .heap
            .object_from_payload(ptr)
            .ok_or_else(|| "pointer is not an object payload owned by this GC heap".to_string())?;
        self.heap
            .push_root(object)
            .map_err(|error| error.to_string())
    }

    /// 弹出最近登记的根。
    pub fn pop_root(&mut self) -> Result<(), String> {
        self.heap
            .pop_root()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// 弹出根并返回解析后的有效负载指针。
    pub fn try_pop_root(&mut self) -> Option<*mut u8> {
        self.heap
            .pop_root()
            .ok()
            .map(|object| unsafe { ImmixHeap::payload::<u8>(object) })
    }

    /// 内部使用的原始根（对象 header 地址）。判断根栈是否为空可直接用 `is_empty()`。
    pub fn roots(&self) -> &[*mut u8] {
        self.heap.roots()
    }

    /// 返回当前根对应的有效负载指针。
    pub fn root_payloads(&self) -> Vec<*mut u8> {
        self.heap
            .roots()
            .iter()
            .map(|&object| unsafe { ImmixHeap::payload::<u8>(object) })
            .collect()
    }

    /// 执行一次完整 Immix 收集；不追踪任何对象字段，只保留显式登记的根。
    pub fn collect(&mut self) -> CollectStats {
        self.collect_with(|_, _| {})
    }

    /// 执行收集，并用 `trace` 遍历对象字段中的子指针。
    ///
    /// `trace` 的第一个参数是对象的**负载指针**；第二个参数是访问器，必须传入
    /// 保存子对象负载指针的**真实字段槽位**，收集器会原地改写搬迁后的地址。
    pub fn collect_with<F>(&mut self, mut trace: F) -> CollectStats
    where
        F: FnMut(*mut u8, &mut dyn FnMut(&mut *mut u8)),
    {
        self.heap
            .collect_with_payloads(|payload, visit| trace(payload, visit))
    }

    pub fn should_collect(&self) -> bool {
        self.heap.should_collect()
    }

    pub fn set_collection_threshold(&mut self, bytes: usize) {
        self.heap.set_collection_threshold(bytes);
    }

    pub fn collection_threshold(&self) -> usize {
        self.heap.collection_threshold()
    }

    pub fn bytes_allocated(&self) -> usize {
        self.heap.bytes_allocated()
    }

    pub fn bytes_since_gc(&self) -> usize {
        self.heap.bytes_since_gc()
    }

    pub fn block_count(&self) -> usize {
        self.heap.block_count()
    }

    /// 解析负载指针：对象若已被搬迁，返回当前有效负载地址。
    pub fn resolve(&self, ptr: *mut u8) -> *mut u8 {
        self.heap.resolve_payload(ptr)
    }

    pub fn try_pin(&mut self, payload: *mut u8) -> Result<(), String> {
        let object = self
            .heap
            .object_from_payload(payload)
            .ok_or_else(|| "pointer is not an object payload owned by this GC heap".to_string())?;
        unsafe { ImmixHeap::pin(object) };
        Ok(())
    }

    /// Pins an object for a lexical scope and releases the pin when the guard drops.
    pub fn pin_guard(&mut self, payload: *mut u8) -> Result<GcPin<'_>, String> {
        self.try_pin(payload)?;
        Ok(GcPin {
            heap: self,
            payload,
        })
    }

    pub fn try_unpin(&mut self, payload: *mut u8) -> Result<(), String> {
        let object = self
            .heap
            .object_from_payload(payload)
            .ok_or_else(|| "pointer is not an object payload owned by this GC heap".to_string())?;
        if !unsafe { ImmixHeap::is_pinned(object) } {
            return Err("GC object is not pinned".into());
        }
        unsafe { ImmixHeap::unpin(object) };
        Ok(())
    }

    /// 读取负载对应对象的 Immix header。
    ///
    /// # Safety
    /// `payload` 必须是本堆分配且仍然存活的对象负载起始地址。
    pub unsafe fn header<'a>(&self, payload: *mut u8) -> &'a ImmixHeader {
        unsafe { ImmixHeap::header(ImmixHeap::header_from_payload(payload)) }
    }

    /// 固定对象，防止收集时搬迁。
    ///
    /// # Safety
    /// `payload` 必须是本堆分配且仍然存活的对象负载起始地址。
    pub unsafe fn pin(&self, payload: *mut u8) {
        if let Some(object) = self.heap.object_from_payload(payload) {
            unsafe { ImmixHeap::pin(object) };
        }
    }

    /// 取消固定。
    ///
    /// # Safety
    /// `payload` 必须是本堆分配且仍然存活的对象负载起始地址。
    pub unsafe fn unpin(&self, payload: *mut u8) {
        if let Some(object) = self.heap.object_from_payload(payload) {
            unsafe { ImmixHeap::unpin(object) };
        }
    }

    /// 对象是否已被固定。
    ///
    /// # Safety
    /// `payload` 必须是本堆分配且仍然存活的对象负载起始地址。
    pub unsafe fn is_pinned(&self, payload: *mut u8) -> bool {
        self.heap
            .object_from_payload(payload)
            .is_some_and(|object| unsafe { ImmixHeap::is_pinned(object) })
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

    #[test]
    fn roots_keep_objects_alive_across_collection() {
        let mut heap = GcHeap::new();
        let rooted = heap.alloc_value(11u64, TypeId(1));
        let unrooted = heap.alloc_value(22u64, TypeId(1));
        heap.push_root(rooted).unwrap();

        let stats = heap.collect();
        assert!(stats.objects_after >= 1);
        assert_eq!(heap.bytes_allocated(), stats.bytes_after);
        assert_eq!(unsafe { *heap.resolve(rooted).cast::<u64>() }, 11);
        assert_eq!(heap.roots().len(), 1);
        // 未登记为根的对象会被回收；不读取它的负载。
        assert!(!unrooted.is_null());
    }

    #[test]
    fn collect_with_rewrites_child_slots() {
        const PARENT: TypeId = TypeId(2);
        let mut heap = GcHeap::new();
        let child = heap.alloc_value(7u64, TypeId(1));
        let parent = heap.alloc_value(child, PARENT);
        heap.push_root(parent).unwrap();

        let stats = heap.collect_with(|payload, visit| unsafe {
            let object = ImmixHeap::header_from_payload(payload);
            if ImmixHeap::header(object).type_id() == PARENT.0 {
                visit(&mut *ImmixHeap::payload::<*mut u8>(object));
            }
        });

        assert!(stats.objects_after >= 2);
        let current_child = heap.resolve(child);
        assert_eq!(unsafe { *current_child.cast::<u64>() }, 7);
        let stored = unsafe { *heap.resolve(parent).cast::<*mut u8>() };
        assert_eq!(heap.resolve(stored), current_child);
    }

    #[test]
    fn threshold_can_be_configured() {
        let mut heap = GcHeap::new();
        heap.alloc(1);
        assert!(!heap.should_collect());
        heap.set_collection_threshold(1);
        assert!(heap.should_collect());
    }

    #[test]
    fn pinned_objects_remain_live_and_stationary_until_unpinned() {
        let mut heap = GcHeap::new();
        let pinned = heap.alloc_value(73u64, TypeId(1));
        let unrooted = heap.alloc_value(91u64, TypeId(1));
        unsafe { heap.pin(pinned) };

        let first = heap.collect();
        assert_eq!(heap.resolve(pinned), pinned);
        assert_eq!(unsafe { *pinned.cast::<u64>() }, 73);
        assert!(first.objects_after >= 1);

        unsafe {
            heap.pin(pinned);
            heap.unpin(pinned);
        }
        assert!(unsafe { heap.is_pinned(pinned) });
        unsafe { heap.unpin(pinned) };
        assert!(!unsafe { heap.is_pinned(pinned) });

        let second = heap.collect();
        assert_eq!(second.objects_after, 0);
        assert!(!unrooted.is_null());
    }

    #[test]
    fn rejects_non_heap_pointer_roots_without_dereferencing_them() {
        let mut heap = GcHeap::new();
        assert!(heap.push_root(std::ptr::dangling_mut::<u8>()).is_err());
    }

    #[test]
    fn preserves_non_gc_child_pointers_during_collection() {
        const PARENT: TypeId = TypeId(2);
        let mut heap = GcHeap::new();
        let parent = heap.alloc_value(std::ptr::dangling_mut::<u8>(), PARENT);
        heap.push_root(parent).unwrap();
        heap.collect_with(|payload, visit| unsafe {
            let object = ImmixHeap::header_from_payload(payload);
            if ImmixHeap::header(object).type_id() == PARENT.0 {
                visit(&mut *ImmixHeap::payload::<*mut u8>(object));
            }
        });
        assert_eq!(
            unsafe { *heap.resolve(parent).cast::<*mut u8>() },
            std::ptr::dangling_mut::<u8>()
        );
    }

    #[test]
    fn pin_guard_releases_its_pin_on_drop() {
        let mut heap = GcHeap::new();
        let payload = heap.alloc_value(5u64, TypeId(1));
        {
            let guard = heap.pin_guard(payload).unwrap();
            assert_eq!(guard.as_ptr(), payload);
        }
        assert!(!unsafe { heap.is_pinned(payload) });
    }
}
