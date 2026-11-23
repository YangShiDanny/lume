//! Immix 风格的标记-区域（mark-region）收集器。
//!
//! 这是 `lumec_core` 之外的新 crate，不会自动替换现有 `GcHeap`。
//! 它提供完整的 Immix 基础能力：
//! - line/block 分块与 bump 分配；
//! - 精确的标记阶段，支持用户自定义对象图遍历；
//! - line 级空闲回收；
//! - 碎片 line 上存活对象的搬迁式整理（evacuation）+ 转发指针；
//! - 根集合与对象内指针的自动改写；
//! - 固定（pin）对象，避免被 FFI 持有的对象移动；
//! - 超大对象专用 block；
//! - 收集统计与分配阈值触发。
//!
//! 不做的事：并发/增量标记、写屏障、跨代引用、保守栈扫描。这些需要 core
//! 侧配合，留到接入 `lumec_core` 时再做。

use std::error::Error;
use std::fmt;
use std::mem::size_of;
use std::ptr;

/// 默认 line 大小：256 字节。
pub const DEFAULT_LINE_SIZE: usize = 256;
/// 默认每个 block 的 line 数：256，即 64 KiB block。
pub const DEFAULT_LINES_PER_BLOCK: usize = 256;
/// 默认触发阈值：1 MiB 分配量。
pub const DEFAULT_COLLECTION_THRESHOLD: usize = 1024 * 1024;

const ALIGN: usize = 8;
const MAGIC: u64 = 0x494D_4D49_5800_0000; // "IMMIX"
const FORWARD_MAGIC: u64 = 0x494D_4D49_4600_0000; // "IMMIF"，此时 total_size 存新地址
const MAX_FORWARD_HOPS: usize = 16;

/// 对象起始处的元数据；用户负载紧跟在 header 后面。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    magic: u64,
    total_size: u64,
    type_id: u64,
    start_line: u64,
    line_count: u64,
    marked: u64,
    pinned: u64,
}

impl Header {
    /// 分配时传入的总字节数（含 header，已对齐）。
    pub fn total_size(&self) -> usize {
        self.total_size as usize
    }

    /// 用户负载大小。
    pub fn payload_size(&self) -> usize {
        self.total_size() - size_of::<Header>()
    }

    /// 建堆时传入的类型标签，供遍历回调区分对象布局。
    pub fn type_id(&self) -> u64 {
        self.type_id
    }

    /// 对象覆盖的 line 数。
    pub fn line_count(&self) -> usize {
        self.line_count as usize
    }

    /// 是否被本轮标记存活。
    pub fn is_marked(&self) -> bool {
        self.marked != 0
    }

    /// 是否被固定，不会被搬迁整理移动。
    pub fn is_pinned(&self) -> bool {
        self.pinned != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmixError {
    ZeroBlock,
    InvalidLineSize,
    ObjectTooLarge { size: usize, max: usize },
    NullRoot,
    RootStackUnderflow,
    InvalidObject,
}

impl fmt::Display for ImmixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroBlock => write!(f, "immix heap needs at least one line per block"),
            Self::InvalidLineSize => {
                write!(f, "immix line size must be a positive multiple of {ALIGN}")
            }
            Self::ObjectTooLarge { size, max } => {
                write!(
                    f,
                    "object of {size} bytes does not fit in a {max} byte block"
                )
            }
            Self::NullRoot => write!(f, "null pointer cannot be used as a GC root"),
            Self::RootStackUnderflow => write!(f, "no GC root to pop"),
            Self::InvalidObject => write!(f, "pointer is not a live immix object"),
        }
    }
}

impl Error for ImmixError {}

/// 一次收集的统计信息。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollectStats {
    pub objects_before: usize,
    pub objects_after: usize,
    pub bytes_before: usize,
    pub bytes_after: usize,
    pub reclaimed_lines: usize,
    pub evacuated_objects: usize,
}

#[derive(Clone, Copy)]
struct Line {
    free: bool,
    marked: bool,
    live_objects: usize,
    live_bytes: usize,
}

impl Line {
    fn empty() -> Self {
        Self {
            free: true,
            marked: false,
            live_objects: 0,
            live_bytes: 0,
        }
    }
}

struct Block {
    /// 以 u64 存储，保证至少 8 字节对齐。
    storage: Box<[u64]>,
    lines: Box<[Line]>,
    objects: Vec<*mut u8>,
    cursor: usize,
    run_end: usize,
    /// 超大对象专用 block：整个 block 只有一个对象，死亡后整块回收。
    dedicated: bool,
}

impl Block {
    fn new(line_size: usize, lines_per_block: usize, dedicated: bool) -> Self {
        let bytes = line_size * lines_per_block;
        let words = bytes.div_ceil(ALIGN);
        Self {
            storage: vec![0u64; words].into_boxed_slice(),
            lines: vec![Line::empty(); lines_per_block].into_boxed_slice(),
            objects: Vec::new(),
            cursor: 0,
            run_end: 0,
            dedicated,
        }
    }

    fn bytes(&self) -> usize {
        self.storage.len() * ALIGN
    }

    fn find_free_run(&self, needed: usize, from_line: usize) -> Option<(usize, usize)> {
        if needed == 0 || needed > self.lines.len() {
            return None;
        }
        let max_start = self.lines.len() - needed;
        let begin = from_line.min(self.lines.len());
        for start in begin..=max_start {
            if (start..start + needed).all(|i| self.lines[i].free) {
                return Some((start, start + needed));
            }
        }
        None
    }
}

/// Immix 堆：分配对象、维护根集合、执行标记/搬迁/回收。
pub struct ImmixHeap {
    line_size: usize,
    lines_per_block: usize,
    blocks: Vec<Block>,
    roots: Vec<*mut u8>,
    bytes_allocated: usize,
    bytes_since_gc: usize,
    collection_threshold: usize,
}

impl Default for ImmixHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl ImmixHeap {
    pub fn new() -> Self {
        Self::with_options(DEFAULT_LINE_SIZE, DEFAULT_LINES_PER_BLOCK)
    }

    pub fn with_options(line_size: usize, lines_per_block: usize) -> Self {
        assert!(
            line_size >= ALIGN && line_size.is_multiple_of(ALIGN),
            "bad line size"
        );
        assert!(lines_per_block > 0, "bad lines per block");
        Self {
            line_size,
            lines_per_block,
            blocks: Vec::new(),
            roots: Vec::new(),
            bytes_allocated: 0,
            bytes_since_gc: 0,
            collection_threshold: DEFAULT_COLLECTION_THRESHOLD,
        }
    }

    pub fn line_size(&self) -> usize {
        self.line_size
    }

    pub fn lines_per_block(&self) -> usize {
        self.lines_per_block
    }

    pub fn block_size(&self) -> usize {
        self.line_size * self.lines_per_block
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn bytes_allocated(&self) -> usize {
        self.bytes_allocated
    }

    pub fn bytes_since_gc(&self) -> usize {
        self.bytes_since_gc
    }

    pub fn collection_threshold(&self) -> usize {
        self.collection_threshold
    }

    pub fn set_collection_threshold(&mut self, bytes: usize) {
        self.collection_threshold = bytes;
    }

    pub fn should_collect(&self) -> bool {
        self.bytes_since_gc >= self.collection_threshold && self.collection_threshold > 0
    }

    /// 分配 `payload_size` 字节并写入 header，返回对象起始指针（即 header 指针）。
    pub fn alloc(&mut self, payload_size: usize, type_id: u64) -> Result<*mut u8, ImmixError> {
        let payload_size = payload_size.max(1);
        let total = align_up(size_of::<Header>() + payload_size, ALIGN);
        let lines_needed = total.div_ceil(self.line_size);

        if lines_needed > self.lines_per_block {
            // 超大对象：专用 block，死亡时整块回收。
            let mut block = Block::new(self.line_size, lines_needed, true);
            let ptr =
                Self::alloc_in_block(&mut block, total, lines_needed, type_id, self.line_size)
                    .expect("dedicated block always has enough free lines");
            self.blocks.push(block);
            self.bytes_allocated += total;
            self.bytes_since_gc += total;
            return Ok(ptr);
        }

        for block in &mut self.blocks {
            if block.dedicated {
                continue;
            }
            if let Some(ptr) =
                Self::alloc_in_block(block, total, lines_needed, type_id, self.line_size)
            {
                self.bytes_allocated += total;
                self.bytes_since_gc += total;
                return Ok(ptr);
            }
        }

        let mut block = Block::new(self.line_size, self.lines_per_block, false);
        let ptr = Self::alloc_in_block(&mut block, total, lines_needed, type_id, self.line_size)
            .expect("fresh block always has enough free lines");
        self.blocks.push(block);
        self.bytes_allocated += total;
        self.bytes_since_gc += total;
        Ok(ptr)
    }

    /// 分配一个 Rust 值作为对象负载。
    pub fn alloc_value<T>(&mut self, value: T, type_id: u64) -> Result<*mut u8, ImmixError> {
        let ptr = self.alloc(size_of::<T>(), type_id)?;
        unsafe { ptr::write(Self::payload::<T>(ptr), value) };
        Ok(ptr)
    }

    /// 把对象指针登记为 GC 根。
    pub fn push_root(&mut self, ptr: *mut u8) -> Result<(), ImmixError> {
        if ptr.is_null() {
            return Err(ImmixError::NullRoot);
        }
        self.roots.push(ptr);
        Ok(())
    }

    /// 弹出最近登记的根。返回的是当前有效地址（可能已被搬迁改写）。
    pub fn pop_root(&mut self) -> Result<*mut u8, ImmixError> {
        let root = self.roots.pop().ok_or(ImmixError::RootStackUnderflow)?;
        Ok(unsafe { Self::resolve(root) })
    }

    pub fn roots(&self) -> &[*mut u8] {
        &self.roots
    }

    /// 执行一次回收；不追踪对象内部指针，只保留直接作为根的对象。
    pub fn collect(&mut self) -> CollectStats {
        self.collect_with(|_, _| {})
    }

    /// 执行一次回收。
    ///
    /// `trace` 收到每个存活对象，以及一个可变指针槽的 visitor。回调应把对象
    /// 内部的子指针交给 visitor；收集器会顺手改写这些槽位，使搬迁后的地址
    /// 生效。
    pub fn collect_with<F>(&mut self, mut trace: F) -> CollectStats
    where
        F: FnMut(*mut u8, &mut dyn FnMut(&mut *mut u8)),
    {
        let mut stats = CollectStats {
            objects_before: self.blocks.iter().map(|block| block.objects.len()).sum(),
            bytes_before: self.bytes_allocated,
            ..Default::default()
        };

        // 1. 清空上一轮标记。
        for block in &mut self.blocks {
            for line in block.lines.iter_mut() {
                line.marked = false;
                line.live_objects = 0;
                line.live_bytes = 0;
            }
            for &object in &block.objects {
                if unsafe { Self::resolve(object) } == object {
                    unsafe { Self::header_mut(object).marked = 0 };
                }
            }
        }

        // 2. 标记阶段：从根出发追踪对象图。
        let mut worklist = Vec::new();
        let roots = self.roots.clone();
        for root in roots {
            self.mark_object(root, &mut worklist);
        }
        while let Some(object) = worklist.pop() {
            let mut children = Vec::new();
            trace(object, &mut |child| {
                children.push(unsafe { Self::resolve(*child) });
            });
            for child in children {
                self.mark_object(child, &mut worklist);
            }
        }

        // 3. 选择碎片 line 上的存活对象进行搬迁。
        let mut evacuees = Vec::new();
        for block in &self.blocks {
            if block.dedicated {
                continue;
            }
            for line_index in 0..block.lines.len() {
                let line = &block.lines[line_index];
                if line.live_objects == 0 || line.live_bytes > self.line_size / 2 {
                    continue;
                }
                for &object in &block.objects {
                    let header = unsafe { Self::header(object) };
                    if header.magic == MAGIC
                        && header.marked != 0
                        && header.pinned == 0
                        && header.line_count == 1
                        && header.start_line as usize == line_index
                    {
                        evacuees.push(object);
                    }
                }
            }
        }

        // 4. 搬迁，并在原对象上留下转发指针。
        for old in evacuees {
            let (total, payload_size, type_id, start, count) = unsafe {
                let header = Self::header(old);
                (
                    header.total_size(),
                    header.payload_size(),
                    header.type_id(),
                    header.start_line as usize,
                    header.line_count as usize,
                )
            };

            if let Some(index) = self.block_index_of(old) {
                let block = &mut self.blocks[index];
                if start + count <= block.lines.len() {
                    for line in block.lines[start..start + count].iter_mut() {
                        line.live_objects = line.live_objects.saturating_sub(1);
                        line.live_bytes = line.live_bytes.saturating_sub(total);
                    }
                }
            }

            let new = self
                .alloc(payload_size, type_id)
                .expect("evacuation allocation should never fail");
            unsafe {
                ptr::copy_nonoverlapping(
                    old.add(size_of::<Header>()),
                    new.add(size_of::<Header>()),
                    payload_size,
                );
                Self::header_mut(new).marked = 0;
            }
            let mut ignored = Vec::new();
            self.mark_object(new, &mut ignored);

            unsafe {
                let header = Self::header_mut(old);
                header.magic = FORWARD_MAGIC;
                header.total_size = new as usize as u64;
                header.marked = 0;
            }
            stats.evacuated_objects += 1;
        }

        // 搬迁时给新副本打过标记，这里重做一次标记遍历，确保新副本的字段也被改写。
        for block in &mut self.blocks {
            for line in block.lines.iter_mut() {
                line.marked = false;
                line.live_objects = 0;
                line.live_bytes = 0;
            }
            for &object in &block.objects {
                if unsafe { Self::resolve(object) } == object {
                    unsafe { Self::header_mut(object).marked = 0 };
                }
            }
        }

        // 5. 改写根与对象内指针，并补标搬迁后新发现的子对象。
        for root in &mut self.roots {
            *root = unsafe { Self::resolve(*root) };
        }
        let roots = self.roots.clone();
        let mut worklist = Vec::new();
        for root in roots {
            self.mark_object(root, &mut worklist);
        }
        while let Some(object) = worklist.pop() {
            let mut children = Vec::new();
            trace(object, &mut |slot| {
                *slot = unsafe { Self::resolve(*slot) };
                children.push(*slot);
            });
            for child in children {
                self.mark_object(child, &mut worklist);
            }
        }

        // 6. 回收：line 级复用、专用大对象块整块释放。
        let mut index = 0;
        while index < self.blocks.len() {
            let remove_dedicated = {
                let block = &self.blocks[index];
                block.dedicated && block.lines.iter().all(|line| line.live_objects == 0)
            };
            if remove_dedicated {
                self.blocks.swap_remove(index);
                continue;
            }

            let block = &mut self.blocks[index];
            for line in block.lines.iter_mut() {
                if line.live_objects == 0 {
                    if !line.free {
                        stats.reclaimed_lines += 1;
                    }
                    line.free = true;
                } else {
                    line.free = false;
                }
                line.marked = false;
            }

            let line_count = block.lines.len();
            let (first_free, free_end) = (0..line_count)
                .find(|&i| block.lines[i].free)
                .map(|start| {
                    let mut end = start + 1;
                    while end < line_count && block.lines[end].free {
                        end += 1;
                    }
                    (start, end)
                })
                .unwrap_or((line_count, line_count));
            block.cursor = first_free * self.line_size;
            block.run_end = free_end * self.line_size;

            block.objects.retain(|&object| {
                if unsafe { Self::is_live(object) } {
                    stats.objects_after += 1;
                    stats.bytes_after += unsafe { Self::header(object).total_size() };
                    true
                } else {
                    false
                }
            });
            index += 1;
        }

        self.bytes_since_gc = 0;
        stats
    }

    /// 取对象负载指针。
    ///
    /// # Safety
    /// `object` 必须是本堆分配且尚未被回收的对象起始地址，并且负载类型 `T`
    /// 必须与分配时写入的类型一致。
    pub unsafe fn payload<T>(object: *mut u8) -> *mut T {
        unsafe { object.add(size_of::<Header>()) as *mut T }
    }

    /// 取对象 header 的共享引用。
    ///
    /// # Safety
    /// `object` 必须是本堆分配且仍然存活的对象起始地址，调用期间不得有其它
    /// 可变引用指向同一 header。
    pub unsafe fn header<'a>(object: *mut u8) -> &'a Header {
        unsafe { &*(object as *const Header) }
    }

    /// 取对象 header 的可变引用。
    ///
    /// # Safety
    /// `object` 必须是本堆分配且仍然存活的对象起始地址，调用期间必须对该
    /// header 拥有独占访问权。
    pub unsafe fn header_mut<'a>(object: *mut u8) -> &'a mut Header {
        unsafe { &mut *(object as *mut Header) }
    }

    /// 解析转发指针，返回对象当前有效地址。
    ///
    /// # Safety
    /// `object` 必须是本堆分配过的对象地址（可能已被搬迁），且对应内存尚未
    /// 被后续分配覆盖。
    pub unsafe fn resolve(object: *mut u8) -> *mut u8 {
        let mut current = object;
        for _ in 0..MAX_FORWARD_HOPS {
            if current.is_null() {
                return current;
            }
            let header = unsafe { &*(current as *const Header) };
            if header.magic != FORWARD_MAGIC {
                return current;
            }
            current = header.total_size as usize as *mut u8;
        }
        current
    }

    /// 固定对象：后续收集不会搬迁它。
    ///
    /// # Safety
    /// `object` 必须是本堆分配且仍然存活的对象起始地址。
    pub unsafe fn pin(object: *mut u8) {
        let object = unsafe { Self::resolve(object) };
        unsafe { Self::header_mut(object).pinned = 1 };
    }

    /// 取消固定。
    ///
    /// # Safety
    /// `object` 必须是本堆分配且仍然存活的对象起始地址。
    pub unsafe fn unpin(object: *mut u8) {
        let object = unsafe { Self::resolve(object) };
        unsafe { Self::header_mut(object).pinned = 0 };
    }

    /// 对象是否已被固定。
    ///
    /// # Safety
    /// `object` 必须是本堆分配且仍然存活的对象起始地址。
    pub unsafe fn is_pinned(object: *mut u8) -> bool {
        let object = unsafe { Self::resolve(object) };
        unsafe { Self::header(object).is_pinned() }
    }

    fn alloc_in_block(
        block: &mut Block,
        total: usize,
        lines_needed: usize,
        type_id: u64,
        line_size: usize,
    ) -> Option<*mut u8> {
        if block.run_end > block.cursor && block.cursor + total <= block.run_end {
            return Some(Self::init_object(
                block,
                block.cursor,
                total,
                lines_needed,
                type_id,
                line_size,
            ));
        }

        let from_line = (block.cursor / line_size).min(block.lines.len());
        let (start, _) = block
            .find_free_run(lines_needed, from_line)
            .or_else(|| block.find_free_run(lines_needed, 0))?;
        let offset = start * line_size;
        Some(Self::init_object(
            block,
            offset,
            total,
            lines_needed,
            type_id,
            line_size,
        ))
    }

    fn init_object(
        block: &mut Block,
        offset: usize,
        total: usize,
        lines_needed: usize,
        type_id: u64,
        line_size: usize,
    ) -> *mut u8 {
        let base = block.storage.as_mut_ptr() as *mut u8;
        let object = unsafe { base.add(offset) };
        let header = Header {
            magic: MAGIC,
            total_size: total as u64,
            type_id,
            start_line: (offset / line_size) as u64,
            line_count: lines_needed as u64,
            marked: 0,
            pinned: 0,
        };
        unsafe {
            ptr::write(object as *mut Header, header);
            ptr::write_bytes(
                object.add(size_of::<Header>()),
                0,
                total - size_of::<Header>(),
            );
        }

        let start = offset / line_size;
        for line in block.lines[start..start + lines_needed].iter_mut() {
            line.free = false;
            line.marked = false;
            line.live_objects = 0;
            line.live_bytes = 0;
        }
        block.objects.push(object);
        block.cursor = offset + total;
        block.run_end = (start + lines_needed) * line_size;
        object
    }

    fn block_index_of(&self, object: *mut u8) -> Option<usize> {
        let address = object as usize;
        self.blocks.iter().position(|block| {
            let base = block.storage.as_ptr() as usize;
            let end = base + block.bytes();
            address >= base && address < end
        })
    }

    fn mark_object(&mut self, object: *mut u8, worklist: &mut Vec<*mut u8>) -> bool {
        let object = unsafe { Self::resolve(object) };
        if object.is_null() {
            return false;
        }
        let Some(index) = self.block_index_of(object) else {
            return false;
        };

        let header = unsafe { Self::header_mut(object) };
        if header.magic != MAGIC || header.marked != 0 {
            return false;
        }
        let start = header.start_line as usize;
        let count = header.line_count as usize;
        let total = header.total_size();
        let block = &mut self.blocks[index];
        if count == 0 || start + count > block.lines.len() {
            return false;
        }

        header.marked = 1;
        for (offset, line) in block.lines[start..start + count].iter_mut().enumerate() {
            line.marked = true;
            line.live_objects += 1;
            if offset == 0 {
                line.live_bytes += total;
            }
        }
        worklist.push(object);
        true
    }

    unsafe fn is_live(object: *mut u8) -> bool {
        let header = unsafe { &*(object as *const Header) };
        header.magic == MAGIC && header.marked != 0
    }
}

fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_aligned_distinct_objects() {
        let mut heap = ImmixHeap::new();
        let first = heap.alloc(16, 1).unwrap();
        let second = heap.alloc(16, 2).unwrap();

        assert_ne!(first, second);
        assert_eq!(first as usize % ALIGN, 0);
        assert_eq!(second as usize % ALIGN, 0);
        unsafe {
            assert_eq!(ImmixHeap::header(first).type_id(), 1);
            assert_eq!(ImmixHeap::header(second).type_id(), 2);
            assert!(ImmixHeap::header(first).payload_size() >= 16);
        }
    }

    #[test]
    fn unreachable_line_is_recycled() {
        let mut heap = ImmixHeap::new();
        // 200 字节负载 + header 超过半个 line，保证两个对象落在不同 line。
        let rooted = heap.alloc(200, 1).unwrap();
        let garbage = heap.alloc(200, 2).unwrap();
        heap.push_root(rooted).unwrap();

        let stats = heap.collect();
        assert!(stats.reclaimed_lines >= 1);

        let reused = heap.alloc(200, 3).unwrap();
        assert_eq!(reused, garbage, "freed line should be reused first");
        assert_ne!(reused, rooted);
    }

    #[test]
    fn trace_keeps_reachable_object_graph() {
        const CHILD_TYPE: u64 = 10;
        const PARENT_TYPE: u64 = 20;

        let mut heap = ImmixHeap::new();
        let child = heap.alloc(200, CHILD_TYPE).unwrap();
        let parent = heap.alloc(200, PARENT_TYPE).unwrap();
        let garbage = heap.alloc(200, 99).unwrap();
        unsafe { ptr::write(ImmixHeap::payload::<*mut u8>(parent), child) };
        heap.push_root(parent).unwrap();

        heap.collect_with(|object, visit| unsafe {
            if ImmixHeap::header(object).type_id() == PARENT_TYPE {
                visit(&mut *ImmixHeap::payload::<*mut u8>(object));
            }
        });

        let reused = heap.alloc(200, 77).unwrap();
        assert_eq!(
            reused, garbage,
            "only the untraced object's line should be recycled"
        );
        assert_ne!(reused, child);
        assert_ne!(reused, parent);
    }

    #[test]
    fn fragmented_line_object_is_evacuated() {
        let mut heap = ImmixHeap::new();
        let live = heap.alloc(8, 1).unwrap();
        let _dead = heap.alloc(200, 2).unwrap();
        heap.push_root(live).unwrap();

        let stats = heap.collect();
        assert_eq!(stats.evacuated_objects, 1);

        let moved = unsafe { ImmixHeap::resolve(live) };
        assert_ne!(moved, live);
        assert!(unsafe { ImmixHeap::header(moved).is_marked() });
        assert_eq!(unsafe { ImmixHeap::resolve(moved) }, moved);
    }

    #[test]
    fn pinned_object_is_not_evacuated() {
        let mut heap = ImmixHeap::new();
        let live = heap.alloc(8, 1).unwrap();
        unsafe { ImmixHeap::pin(live) };
        heap.push_root(live).unwrap();

        let stats = heap.collect();
        assert_eq!(stats.evacuated_objects, 0);
        assert_eq!(unsafe { ImmixHeap::resolve(live) }, live);
        assert!(unsafe { ImmixHeap::is_pinned(live) });

        unsafe { ImmixHeap::unpin(live) };
        assert!(!unsafe { ImmixHeap::is_pinned(live) });
    }

    #[test]
    fn child_pointers_are_rewritten_after_evacuation() {
        const CHILD_TYPE: u64 = 10;
        const PARENT_TYPE: u64 = 20;

        let mut heap = ImmixHeap::new();
        let child = heap.alloc(8, CHILD_TYPE).unwrap();
        let parent = heap.alloc(8, PARENT_TYPE).unwrap();
        unsafe { ptr::write(ImmixHeap::payload::<*mut u8>(parent), child) };
        heap.push_root(parent).unwrap();

        let stats = heap.collect_with(|object, visit| unsafe {
            if ImmixHeap::header(object).type_id() == PARENT_TYPE {
                visit(&mut *ImmixHeap::payload::<*mut u8>(object));
            }
        });
        assert_eq!(stats.evacuated_objects, 2);

        let new_parent = unsafe { ImmixHeap::resolve(parent) };
        let new_child = unsafe { ImmixHeap::resolve(child) };
        let stored = unsafe { *ImmixHeap::payload::<*mut u8>(new_parent) };
        assert_eq!(stored, new_child);
        assert_ne!(new_parent, parent);
        assert_ne!(new_child, child);
    }

    #[test]
    fn large_object_has_dedicated_block_and_is_reclaimed() {
        let mut heap = ImmixHeap::new();
        let size = heap.block_size() + 4096;
        let big = heap.alloc(size, 1).unwrap();
        assert_eq!(heap.block_count(), 1);
        assert!(unsafe { ImmixHeap::header(big).line_count() > 1 });

        let stats = heap.collect();
        assert_eq!(stats.objects_after, 0);
        assert_eq!(heap.block_count(), 0);
    }

    #[test]
    fn threshold_triggers_collection() {
        let mut heap = ImmixHeap::new();
        heap.set_collection_threshold(1);
        heap.alloc(8, 1).unwrap();
        assert!(heap.should_collect());

        let stats = heap.collect();
        assert_eq!(stats.objects_after, 0);
        assert!(!heap.should_collect());
    }

    #[test]
    fn root_stack_works() {
        let mut heap = ImmixHeap::new();
        let object = heap.alloc(8, 1).unwrap();
        assert!(matches!(
            heap.pop_root(),
            Err(ImmixError::RootStackUnderflow)
        ));
        heap.push_root(object).unwrap();
        assert_eq!(heap.roots().len(), 1);
        assert_eq!(heap.pop_root().unwrap(), object);
        assert!(heap.roots().is_empty());
    }
}
