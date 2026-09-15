#![allow(warnings)]
mod bitmap;
mod emplace;
mod errors;
mod freelist;
mod platform;
mod pool_guard;
#[cfg(test)]
mod tests;
use self::emplace::Emplace;
use self::pool_guard::PoolGuard;
#[cfg(unix)]
use crate::platform::LibcBlockSource;
#[cfg(windows)]
use crate::platform::WindowsBlockSource;
use bitmap::BitMap;
use core::cmp::max;
use core::ptr::null_mut;
use freelist::FreeList;

pub mod block_source;
pub use block_source::BlockSource;
pub use errors::AllocErr;

const POINTER_SIZE: usize = size_of::<*mut u8>();
const POINTER_ALIGN: usize = align_of::<*mut u8>();
const MUL_CONSTANT: usize = 2;

/// Platform block source used by `Pool::new` on Unix targets.
#[cfg(unix)]
pub type DefaultBlockSource = LibcBlockSource;

/// Platform block source used by `Pool::new` on Windows targets.
#[cfg(windows)]
pub type DefaultBlockSource = WindowsBlockSource;

/// Fixed-size slot allocator backed by one or more mapped blocks.
///
/// A `Pool` serves allocations of exactly one slot size. Requests are rounded
/// up when the pool is created, so every returned pointer has enough room for
/// `slot_size` bytes and is aligned to `slot_align`.
///
/// Allocation uses two fast paths before asking the OS for another block:
///
/// ```text
/// 1. Reuse a freed slot from the freelist.
///
/// freelist.head
///      |
///      v
/// +---------+      +---------+
/// | next ---+----> | null    |
/// +---------+      +---------+
///
/// 2. Bump the active block high-water mark.
///
///                      hwm       end
///                       |         |
///                       v         v
/// +--------+--------+---+---------+
/// | header | bitmap | A |  free   |
/// +--------+--------+---+---------+
///                      ^
///                      new allocation
/// ```
///
/// If neither path can provide a slot, the pool maps a larger block and makes
/// that block the new active block.
#[derive(Debug)]
pub struct Pool<S: BlockSource = DefaultBlockSource> {
    /// Stack of freed slots ready to be reused.
    freelist: FreeList,
    /// Size of each slot after rounding up for pointer storage and alignment.
    slot_size: usize,
    /// Alignment promised for every slot returned by this pool.
    slot_align: usize,
    /// Most recent block. New high-water allocations come from this block.
    active_block: *mut Block,
    /// Backend used to map and unmap raw memory blocks.
    source: S,
    /// Cached platform granularity, so growth never re-queries the OS.
    page_size: usize,
    /// Number of slots currently handed out and not yet freed.
    live_slots: usize,
    /// Marker that removes the auto `Send`/`Sync` impls raw pointers bring.
    _single_thread: PhantomData<Rc<()>>,
}

/// Header stored at the beginning of every mapped block.
///
/// A block is laid out as:
///
/// ```text
/// base
///  |
///  v
/// +--------+--------+-----+-----+-----+---------+
/// | Block  | bitmap |  A  |  B  |  C  |  free   |
/// +--------+--------+-----+-----+-----+---------+
/// ^                 ^                 ^         ^
/// |                 |                 |         |
/// base              first slot        hwm       end
/// ```
///
/// `hwm` is the cursor for bump allocation. Slots below `hwm` are either live
/// or on the freelist. Slots at or above `hwm` have never been handed out.
/// `first`/`capacity` cache the slot region geometry so allocation and freeing
/// never recompute header math on the hot path.
#[derive(Debug)]
pub struct Block {
    /// Base address returned by the block source.
    base: *mut u8,
    /// Total mapped block size in bytes.
    size: usize,
    /// High-water mark: the first never-allocated byte in the slot area.
    hwm: *mut u8,
    /// One-past-the-end address of this mapped block.
    end: *mut u8,
    /// Previous block in the pool's block chain.
    prev: *mut Block,
    /// Address of the first usable slot (header + bitmap rounded up).
    first: *mut u8,
    /// Number of slots that fit between `first` and `end`.
    capacity: usize,
    /// Allocation-state bitmap for slots in this block.
    bitmap: BitMap,
}

/// Wrapper that lets the empty sentinel block be `Sync`.
///
/// The sentinel is never mutated as a real block. It is only used as the end
/// marker for the block linked list:
///
/// ```text
/// active_block ---> block 3 ---> block 2 ---> block 1 ---> EMPTY_BLOCK
/// ```
struct EmptyBlockWrapper(Block);

static EMPTY_BLOCK: EmptyBlockWrapper = EmptyBlockWrapper(Block {
    base: null_mut(),
    size: 1,
    hwm: null_mut(),
    end: null_mut(),
    prev: null_mut(),
    first: null_mut(),
    capacity: 0,
    bitmap: BitMap::dangling(),
});

unsafe impl Sync for EmptyBlockWrapper {}
impl EmptyBlockWrapper {
    /// Returns a raw pointer to the sentinel block.
    fn get_inner(&self) -> *mut Block {
        &self.0 as *const Block as *mut Block
    }
}
impl Block {
    /// Builds a block header value before it is written into mapped memory.
    ///
    /// Crate private: blocks only ever come from the pool's block source.
    /// Eight fields (base, size, hwm, end, prev, first, capacity, bitmap) is
    /// one over the default lint threshold, hence the allow below.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        base: *mut u8,
        size: usize,
        hwm: *mut u8,
        end: *mut u8,
        prev: *mut Block,
        first: *mut u8,
        capacity: usize,
        bitmap: BitMap,
    ) -> Self {
        Self {
            base,
            size,
            hwm,
            end,
            prev,
            first,
            capacity,
            bitmap,
        }
    }

    /// Returns the block base pointer.
    fn ptr(&self) -> *mut u8 {
        self.base
    }

    /// Returns the mapped size for this block.
    fn size(&self) -> usize {
        self.size
    }

    /// Returns the current high-water mark.
    fn hwm(&self) -> *mut u8 {
        self.hwm
    }

    /// Returns the previous block in the chain.
    fn prev(&self) -> *mut Block {
        self.prev
    }
}
#[cfg(unix)]
impl Pool<LibcBlockSource> {
    /// Creates a pool using the Unix libc-backed block source.
    ///
    /// Panics on invalid alignment or size overflow; use [`Pool::try_new`]
    /// for a fallible version.
    pub fn new(size: usize, align: usize) -> Self {
        Self::with_source(size, align, LibcBlockSource)
    }

    /// Fallible version of [`Pool::new`].
    pub fn try_new(size: usize, align: usize) -> Result<Self, AllocErr> {
        Self::try_with_source(size, align, LibcBlockSource)
    }
}

#[cfg(windows)]
impl Pool<WindowsBlockSource> {
    /// Creates a pool using the Windows virtual-memory block source.
    pub fn new(size: usize, align: usize) -> Self {
        Self::with_source(size, align, WindowsBlockSource)
    }
}

impl<S: BlockSource> Pool<S> {
    /// Creates a pool with an explicit block source.
    ///
    /// The public constructors choose a platform source. This function keeps
    /// the allocator logic generic, which lets tests or future backends provide
    /// memory without changing the allocation algorithm.
    ///
    /// Panics on invalid input; use [`Pool::try_with_source`] to handle
    /// errors instead.
    pub fn with_source(size: usize, align: usize, source: S) -> Self {
        Self::try_with_source(size, align, source).unwrap_or_else(|err| err.panic())
    }

    /// Fallible pool construction with an explicit block source.
    ///
    /// Fails with [`AllocErr::InvalidAlignment`] when `align` is not a power
    /// of two, when it exceeds the source page size (mappings cannot promise
    /// wider absolute alignment), or when the source reports an unusable page
    /// size. Fails with [`AllocErr::Overflow`] when the rounded slot size
    /// overflows `usize` or `isize::MAX`.
    pub fn try_with_source(size: usize, align: usize, source: S) -> Result<Self, AllocErr> {
        if !align.is_power_of_two() {
            return Err(AllocErr::InvalidAlignment);
        }
        let page_size = source.page_size();
        if !page_size.is_power_of_two() {
            // A zero or non power of two granularity means the source itself
            // is misconfigured; growth math (`align_up`) requires powers of two.
            return Err(AllocErr::InvalidAlignment);
        }
        if align > page_size {
            // Mappings are only page aligned, so absolute alignment wider
            // than a page cannot be promised for the first slot.
            return Err(AllocErr::InvalidAlignment);
        }
        let align = max(align, POINTER_ALIGN);

        let size = max(size, POINTER_SIZE);
        let aligned_size = Self::align_up(size, align).ok_or(AllocErr::Overflow)?;
        if aligned_size > isize::MAX as usize {
            return Err(AllocErr::Overflow);
        }
        Ok(Self {
            freelist: FreeList::dangling(),
            slot_size: aligned_size,
            slot_align: align,
            active_block: EMPTY_BLOCK.get_inner(),
            source,
            page_size,
            live_slots: 0,
            _single_thread: PhantomData,
        })
    }

    /// Size in bytes of every slot handed out by this pool.
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// Alignment guaranteed for every slot handed out by this pool.
    pub fn slot_align(&self) -> usize {
        self.slot_align
    }

    /// Platform allocation granularity backing this pool.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Number of slots currently handed out and not yet freed.
    pub fn live_slots(&self) -> usize {
        self.live_slots
    }

    /// Number of mapped blocks owned by this pool.
    pub fn num_blocks(&self) -> usize {
        let mut count = 0;
        let mut current = self.active_block;
        while current != EMPTY_BLOCK.get_inner() {
            count += 1;
            current = unsafe { (*current).prev };
        }
        count
    }

    /// Total slot capacity across all mapped blocks.
    pub fn capacity_slots(&self) -> usize {
        let mut total = 0;
        let mut current = self.active_block;
        while current != EMPTY_BLOCK.get_inner() {
            total += unsafe { (*current).capacity };
            current = unsafe { (*current).prev };
        }
        total
    }

    /// Total mapped bytes across all blocks owned by this pool.
    pub fn memory_bytes(&self) -> usize {
        let mut total = 0;
        let mut current = self.active_block;
        while current != EMPTY_BLOCK.get_inner() {
            total += unsafe { (*current).size };
            current = unsafe { (*current).prev };
        }
        total
    }

    /// Returns `true` when no slots are currently handed out.
    pub fn is_empty(&self) -> bool {
        self.live_slots == 0
    }

    /// Checks whether `ptr` is a live, slot aligned allocation of this pool.
    ///
    /// This is a lookup helper for diagnostics and tests. It returns `false`
    /// for freed slots, header/bitmap bytes, foreign pointers, and null.
    pub fn contains(&self, ptr: *mut u8) -> bool {
        if ptr.is_null() {
            return false;
        }
        let mut current = self.active_block;
        while current != EMPTY_BLOCK.get_inner() {
            let block = unsafe { &*current };
            if ptr >= block.first && ptr < block.hwm {
                let offset = (ptr as usize).wrapping_sub(block.first as usize);
                if !offset.is_multiple_of(self.slot_size) {
                    return false;
                }
                let idx = offset / self.slot_size;
                return !block.bitmap.is_free(idx).0;
            }
            current = block.prev;
        }
        false
    }

    /// Allocates one raw slot and panics if allocation fails.
    ///
    /// The returned pointer owns `self.slot_size` bytes until it is returned
    /// with `free`. Prefer [`Pool::try_alloc`] when out of memory is
    /// recoverable.
    pub fn alloc(&mut self) -> *mut u8 {
        self.try_alloc().unwrap_or_else(|err| err.panic())
    }

    /// Fallible raw allocation: reuse a freed slot or bump the active block,
    /// mapping a new block only when both fast paths are exhausted.
    pub fn try_alloc(&mut self) -> Result<*mut u8, AllocErr> {
        if let Some(ptr) = self.try_allocate_fast() {
            Ok(ptr)
        } else {
            self.try_allocate_slow()
        }
    }

    /// Allocates one slot and constructs a `T` inside it.
    ///
    /// The value is only written after a guard has been installed. If `f`
    /// panics while building the value, the guard returns the raw slot to the
    /// pool instead of leaking it.
    ///
    /// ```text
    /// allocate raw slot
    ///       |
    ///       v
    /// +------------+
    /// | PoolGuard  |  if construction panics, Drop frees the slot
    /// +------------+
    ///       |
    ///       v
    /// write T into slot
    ///       |
    ///       v
    /// commit guard and return *mut T
    /// ```
    pub fn try_allocate_with<T, F>(&mut self, f: F) -> Result<*mut T, AllocErr>
    where
        F: FnOnce() -> T,
    {
        self.validate_size_align::<T>()?;
        Ok((self.try_emplace()?).write(f()))
    }

    /// Allocates a slot and moves `value` into it, returning an owning handle.
    ///
    /// The [`Slot`] dereferences to `T` and automatically drops `T` and frees
    /// the slot when it goes out of scope. This is the preferred safe API:
    /// unlike raw [`Pool::alloc`], it cannot leak or use-after-free by
    /// accident. Panics on failure; use [`Pool::try_alloc_value`] to handle
    /// errors.
    pub fn alloc_value<T>(&mut self, value: T) -> Slot<'_, T, S> {
        self.try_alloc_value(value)
            .unwrap_or_else(|err| err.panic())
    }

    /// Fallible version of [`Pool::alloc_value`].
    pub fn try_alloc_value<T>(&mut self, value: T) -> Result<Slot<'_, T, S>, AllocErr> {
        self.validate_size_align::<T>()?;
        let ptr = self.try_alloc()? as *mut T;
        unsafe {
            ptr.write(value);
        }
        Ok(Slot::new(self, ptr))
    }

    /// Reserves an uninitialized slot for `T`, returning an owning handle.
    ///
    /// Finish initialization with [`Slot::write`], which converts the handle
    /// into `Slot<T>` without touching the pool again. Panics on failure;
    /// use [`Pool::try_alloc_uninit`] for the fallible version.
    pub fn alloc_uninit<T>(&mut self) -> Slot<'_, MaybeUninit<T>, S> {
        self.try_alloc_uninit().unwrap_or_else(|err| err.panic())
    }

    /// Fallible version of [`Pool::alloc_uninit`].
    pub fn try_alloc_uninit<T>(&mut self) -> Result<Slot<'_, MaybeUninit<T>, S>, AllocErr> {
        self.validate_size_align::<T>()?;
        let ptr = self.try_alloc()? as *mut MaybeUninit<T>;
        Ok(Slot::new(self, ptr))
    }

    /// Frees a slot previously leaked via [`Slot::into_raw`], dropping `T`.
    ///
    /// Runs the destructor of the value in place, then returns the slot to
    /// the pool. Panics on invalid pointers; use [`Pool::try_free_value`]
    /// for a fallible version.
    ///
    /// # Safety
    ///
    /// `ptr` must have come from [`Slot::into_raw`] for a `Slot<T>` of this
    /// pool, must not have been freed already, and the slot must still hold
    /// a valid `T`. A pointer cast from an unrelated type with a matching
    /// size would run the wrong destructor, which is undefined behavior that
    /// no runtime check can catch.
    pub unsafe fn free_value<T>(&mut self, ptr: *mut T) {
        unsafe { self.try_free_value(ptr) }.unwrap_or_else(|err| err.panic())
    }

    /// Fallible version of [`Pool::free_value`].
    ///
    /// # Safety
    ///
    /// Same contract as [`Pool::free_value`]: the pointer must designate a
    /// live `T` owned by this pool.
    pub unsafe fn try_free_value<T>(&mut self, ptr: *mut T) -> Result<(), AllocErr> {
        if ptr.is_null() {
            return Err(AllocErr::InvalidPointer);
        }
        self.validate_size_align::<T>()?;
        unsafe {
            ptr.drop_in_place();
        }
        self.try_free(ptr as *mut u8)
    }

    /// Allocates a raw slot and wraps it in an emplacement guard.
    ///
    /// This is private because callers should use `try_allocate_with`, which
    /// performs the size and alignment validation before exposing the typed
    /// pointer.
    fn try_emplace<'a, T>(&'a mut self) -> Result<Emplace<'a, S, T>, AllocErr> {
        let ptr = self.try_allocate()?;
        let guard = PoolGuard::with_source(self, ptr);
        Ok(Emplace::with_source(guard))
    }

    /// Attempts allocation without mapping a new block.
    ///
    /// The fast path first reuses a freed slot. If the freelist is empty, it
    /// bumps the active block's high-water mark.
    ///
    /// ```text
    /// active block
    ///
    ///                    hwm       end
    ///                     |         |
    ///                     v         v
    /// +--------+--------+---+---+---+---------+
    /// | header | bitmap | A | B | C |  free   |
    /// +--------+--------+---+---+---+---------+
    ///                    ^
    ///                    next slot if freelist is empty
    /// ```
    fn try_allocate_fast(&mut self) -> Option<*mut u8> {
        // Freed slots are preferred because they reuse existing block space.
        if let Some(slot) = self.freelist.get_slot() {
            unsafe {
                let block = &mut *self.get_block(slot);
                // A slot coming from our own freelist must name a valid slot
                // of its block. Anything else means pool memory was corrupted.
                let idx = self
                    .try_slot_index(slot, &*block)
                    .unwrap_or_else(|err| err.panic());
                block.bitmap.set(idx, None);
            }
            return Some(slot);
        }

        // The sentinel has no slot area; skip straight to mapping a block.
        if self.active_block == EMPTY_BLOCK.inner() {
            return None;
        }

        // If no slot was freed, hand out the next never used slot.
        unsafe {
            let block = self.active_block;

            let hwm = (*block).hwm;
            let end = (*block).end;
            let remaining = (end as usize).checked_sub(hwm as usize);
            if remaining.is_some_and(|remaining| self.slot_size <= remaining) {
                let slot = hwm;
                let slot_index = self
                    .try_slot_index(slot, &*block)
                    .unwrap_or_else(|err| err.panic());

                (*block).bitmap.set(slot_index, None);
                (*block).hwm = hwm.add(self.slot_size);
                self.live_slots += 1;

                return Some(slot);
            }
        }

        None
    }

    /// Maps a new block, links it as active, then retries the fast path.
    ///
    /// Growth is based on the previous block size and rounded to page size.
    /// The new block becomes the front of the block chain:
    ///
    /// ```text
    /// before:
    ///
    /// active_block ---> old block ---> EMPTY_BLOCK
    ///
    /// after:
    ///
    /// active_block ---> new block ---> old block ---> EMPTY_BLOCK
    /// ```
    fn try_allocate_slow(&mut self) -> Result<*mut u8, AllocErr> {
        self.new_block(self.next_block_size()?)?;
        self.try_allocate_fast().ok_or(AllocErr::OutOfMemory)
    }

    /// Computes the size of the next block to map.
    ///
    /// Growth doubles the previous block (rounded up to page granularity) but
    /// is forced large enough to hold the header plus at least one slot, so
    /// huge slot sizes grow the first block instead of panicking.
    fn next_block_size(&self) -> Result<usize, AllocErr> {
        let prev_size = unsafe { (*self.active_block).size };
        let mut size = match prev_size.checked_mul(MUL_CONSTANT) {
            Some(doubled) => {
                doubled.max(Self::align_up(prev_size, self.page_size).ok_or(AllocErr::Overflow)?)
            }
            None => Self::align_up(prev_size, self.page_size).ok_or(AllocErr::Overflow)?,
        };
        // The first block starts from the 1-byte sentinel, so growth alone
        // already yields a page. The loop below only ever triggers when a
        // single slot does not fit the growth candidate (huge slots).
        loop {
            let remaining = size
                .checked_sub(size_of::<Block>())
                .ok_or(AllocErr::Overflow)?;
            let bitmap_size = BitMap::required_bytes(remaining, self.slot_size);
            let header = self.header_for(bitmap_size)?;
            let need = header
                .checked_add(self.slot_size)
                .ok_or(AllocErr::Overflow)?;
            if need <= size {
                return Ok(size);
            }
            size = Self::align_up(need, self.page_size).ok_or(AllocErr::Overflow)?;
        }
    }

    /// Requests a block from the source and writes the `Block` header into it.
    ///
    /// The beginning of a block stores allocator metadata. The first usable
    /// slot is rounded up to `slot_size`, so every slot boundary stays aligned
    /// with the pool's slot layout.
    ///
    /// ```text
    /// ptr/base
    ///  |
    ///  v
    /// +--------+----------+---------+---------+
    /// | Block  | bitmap   | padding | slots   |
    /// +--------+----------+---------+---------+
    /// ^                              ^
    /// |                              |
    /// base                           hwm starts here
    /// ```
    fn new_block(&mut self, new_block_size: usize) -> Result<(), AllocErr> {
        let ptr: *mut u8 = self.source.map(new_block_size);
        if ptr.is_null() {
            return Err(AllocErr::OutOfMemory);
        }
        let remaining_bytes = new_block_size
            .checked_sub(size_of::<Block>())
            .ok_or(AllocErr::Overflow)?;
        let bitmap_size = BitMap::required_bytes(remaining_bytes, self.slot_size);
        let header_size = self.header_for(bitmap_size)?;
        let first = unsafe { ptr.add(header_size) };
        let end = unsafe { ptr.add(new_block_size) };
        let capacity = new_block_size
            .checked_sub(header_size)
            .ok_or(AllocErr::Overflow)?
            / self.slot_size;
        debug_assert!(capacity >= 1, "block must fit at least one slot");
        let new_block = unsafe {
            Block::new(
                ptr,
                new_block_size,
                first,
                end,
                self.active_block,
                first,
                capacity,
                BitMap::new(ptr.add(size_of::<Block>()), bitmap_size),
            )
        };
        unsafe {
            (ptr as *mut Block).write(new_block);
            // Mappings from custom sources are not guaranteed zeroed, and a
            // stale bitmap bit would misreport fresh slots as allocated.
            ptr.add(size_of::<Block>()).write_bytes(0, bitmap_size);
        }
        self.active_block = ptr as *mut Block;
        Ok(())
    }

    /// Returns the first slot address in `block`.
    ///
    /// This skips the block header and bitmap, then rounds up to the slot size.
    fn get_first_block_ptr(&self, block: &Block) -> *mut u8 {
        let Block { base, bitmap, .. } = block;

        let offset = Self::align_up_unchecked(size_of::<Block>() + bitmap.size, self.slot_size);
        unsafe { base.add(offset) }
    }

    /// Returns how many bytes the block header and bitmap occupy together.
    fn get_header_bitmap_size(&self, bitmap_size: usize) -> usize {
        Self::align_up_unchecked(size_of::<Block>() + bitmap_size, self.slot_size)
    }

    /// Rounds `size` up to `align` without overflow checking.
    ///
    /// The bitmap is sized (possibly generously) for the whole block, then
    /// the combined header is rounded up so the first slot stays on a slot
    /// boundary, which also preserves the pool alignment guarantee.
    fn header_for(&self, bitmap_size: usize) -> Result<usize, AllocErr> {
        let header = size_of::<Block>()
            .checked_add(bitmap_size)
            .ok_or(AllocErr::Overflow)?;
        Self::align_up(header, self.slot_size).ok_or(AllocErr::Overflow)
    }

    /// Rounds `size` up to `align`, returning `None` on integer overflow.
    ///
    /// `align` must be a power of two.
    fn align_up(size: usize, align: usize) -> Option<usize> {
        size.checked_add(align - 1).map(|res| res & !(align - 1))
    }

    /// Finds which block owns `ptr`.
    ///
    /// The search walks backward through the block chain. A pointer belongs to
    /// a block only if it is inside that block's slot region and below `hwm`.
    ///
    /// ```text
    /// active ---> block C ---> block B ---> block A ---> EMPTY
    ///              check        check        check
    /// ```
    fn get_block(&self, ptr: *mut u8) -> *mut Block {
        if ptr.is_null() {
            return null_mut();
        }
        let mut current = self.active_block;
        while current != EMPTY_BLOCK.get_inner() {
            let block = unsafe { &*current };
            let region_start_ptr = self.get_first_block_ptr(block);
            if ptr >= region_start_ptr && ptr < block.hwm {
                return current;
            }
            current = block.prev;
        }

        null_mut()
    }

    /// Computes the slot index for a pointer known to belong to `block`.
    ///
    /// ```text
    /// first slot
    ///    |
    ///    v
    /// +-----+-----+-----+-----+
    /// |  0  |  1  |  2  |  3  |
    /// +-----+-----+-----+-----+
    ///             ^
    ///             ptr -> index 2
    /// ```
    ///
    /// Unlike the old debug-only assertion, misalignment and out-of-range
    /// indices are runtime errors in every build profile, so release builds
    /// can never turn a bad free into out-of-bounds bitmap writes.
    fn try_slot_index(&self, ptr: *mut u8, block: &Block) -> Result<usize, AllocErr> {
        let first = block.first as usize;
        let offset = (ptr as usize).wrapping_sub(first);
        if !offset.is_multiple_of(self.slot_size) {
            return Err(AllocErr::MisalignedFree);
        }
        let idx = offset / self.slot_size;
        if idx >= block.capacity {
            return Err(AllocErr::InvalidPointer);
        }
        Ok(idx)
    }

    /// Frees a raw slot and panics if the pointer is invalid.
    pub fn free(&mut self, ptr: *mut u8) {
        self.try_free(ptr).map_err(|err| err.panic());
    }

    /// Validates and frees a raw slot.
    ///
    /// Freeing clears the owning block's bitmap bit and pushes the slot into
    /// the freelist:
    ///
    /// ```text
    /// before free:
    ///
    /// bitmap bit = 1
    /// slot is live
    ///
    /// after free:
    ///
    /// bitmap bit = 0
    /// slot bytes store previous freelist head
    /// freelist.head points at this slot
    /// ```
    fn try_free(&mut self, ptr: *mut u8) -> Result<(), AllocErr> {
        let block_ptr = self.get_block(ptr);
        if block_ptr.is_null() {
            return Err(AllocErr::InvalidPointer);
        }
        let block = unsafe { &mut *block_ptr };
        let slot_index = self.get_slot_index(ptr, block);
        let (is_slot_free, cached_byte) = block.bitmap.is_free(slot_index);
        if is_slot_free {
            return Err(AllocErr::DoubleFree);
        }
        block.bitmap.clear(slot_index, Some(cached_byte));
        self.freelist.add_slot(ptr);
        Ok(())
    }

    /// Drops all allocations at once without unmapping any blocks.
    ///
    /// Bitmaps are cleared, high-water marks rewind, the freelist is emptied,
    /// and the live count returns to zero. Mapped blocks are kept, so
    /// refilling the pool after `reset` reuses warm memory instead of asking
    /// the OS again. This is the arena-style bulk-reuse entry point.
    pub fn reset(&mut self) {
        self.freelist.clear();
        let mut current = self.active_block;
        while current != EMPTY_BLOCK.get_inner() {
            let block = unsafe { &mut *current };
            block.hwm = block.first;
            block.bitmap.zero();
            current = block.prev;
        }
        self.live_slots = 0;
    }

    /// Ensures room for `additional` more slots, mapping blocks if needed.
    ///
    /// Returns `OutOfMemory` if the source cannot back the growth.
    pub fn reserve(&mut self, additional: usize) -> Result<(), AllocErr> {
        while self.capacity_slots().saturating_sub(self.live_slots) < additional {
            let size = self.next_block_size()?;
            self.new_block(size)?;
        }
        Ok(())
    }

    /// Releases fully-free non-active blocks back to the block source.
    ///
    /// A block is releasable only when every bitmap bit is clear, which means
    /// none of its slots are live. Freed slots of released blocks are pruned
    /// from the freelist before unmapping. The active block is always kept,
    /// even when empty, so the pool stays ready for the next allocation.
    pub fn shrink_to_fit(&mut self) {
        let active = self.active_block;
        // Pass 1: identify releasable blocks by (base, size).
        let mut victims: Vec<(*mut u8, usize)> = Vec::new();
        let mut current = active;
        while current != EMPTY_BLOCK.get_inner() {
            let block = unsafe { &*current };
            if current != active && block.bitmap.all_clear(block.capacity) {
                victims.push((block.base, block.size));
            }
            current = block.prev;
        }
        if victims.is_empty() {
            return;
        }
        // Pass 2: prune freelist entries that point into released blocks.
        // The entries live inside the victim mappings, so filtering must
        // happen before any unmap.
        let mut kept: Vec<*mut u8> = Vec::new();
        while let Some(slot) = self.freelist.get_slot() {
            let in_victim = victims.iter().any(|&(base, size)| {
                let base = base as usize;
                let slot = slot as usize;
                slot >= base && slot.wrapping_sub(base) < size
            });
            if !in_victim {
                kept.push(slot);
            }
        }
        for slot in kept {
            self.freelist.add_slot(slot);
        }
        // Pass 3: relink the chain around victims, then unmap them.
        let mut live_chain: Vec<*mut Block> = Vec::new();
        current = active;
        while current != EMPTY_BLOCK.get_inner() {
            let next = unsafe { (*current).prev };
            let is_victim = victims
                .iter()
                .any(|&(base, _)| base == unsafe { (*current).base });
            if !is_victim {
                live_chain.push(current);
            }
            current = next;
        }
        for (i, &block) in live_chain.iter().enumerate() {
            unsafe {
                (*block).prev = live_chain
                    .get(i + 1)
                    .copied()
                    .unwrap_or_else(|| EMPTY_BLOCK.get_inner());
            }
        }
        for (base, size) in victims {
            self.source.unmap(base, size);
        }
    }

    /// Checks whether a value of type `T` fits inside this pool's slots.
    ///
    /// Both requirements must hold:
    ///
    /// ```text
    /// size_of::<T>()  <= slot_size
    /// align_of::<T>() <= slot_align
    /// ```
    pub fn validate_size_align<T>(&self) -> Result<(), AllocErr> {
        if size_of::<T>() <= self.slot_size && align_of::<T>() <= self.slot_align {
            Ok(())
        } else {
            Err(AllocErr::InvalidSizeOrAlignement)
        }
    }
}

impl<S: BlockSource> Drop for Pool<S> {
    /// Unmaps every block owned by the pool.
    ///
    /// Individual live allocations do not need to be freed before the pool is
    /// dropped. Dropping the pool releases the backing blocks in chain order.
    fn drop(&mut self) {
        while self.active_block != EMPTY_BLOCK.get_inner() {
            let current = unsafe { &*self.active_block };
            self.active_block = current.prev;
            self.source.unmap(current.base as *mut u8, current.size);
        }
    }
}
