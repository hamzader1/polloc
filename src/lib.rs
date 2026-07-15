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
    pub freelist: FreeList,
    /// Size of each slot after rounding up for pointer storage and alignment.
    pub slot_size: usize,
    /// Alignment promised for every slot returned by this pool.
    pub slot_align: usize,
    /// Most recent block. New high-water allocations come from this block.
    pub active_block: *mut Block,
    /// Backend used to map and unmap raw memory blocks.
    source: S,
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
#[derive(Debug)]
pub struct Block {
    /// Base address returned by the block source.
    pub base: *mut u8,
    /// Total mapped block size in bytes.
    pub size: usize,
    /// High-water mark: the first never-allocated byte in the slot area.
    pub hwm: *mut u8,
    /// One-past-the-end address of this mapped block.
    pub end: *mut u8,
    /// Previous block in the pool's block chain.
    pub prev: *mut Block,
    /// Allocation-state bitmap for slots in this block.
    pub bitmap: BitMap,
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
    pub fn new(
        base: *mut u8,
        size: usize,
        hwm: *mut u8,
        end: *mut u8,
        prev: *mut Block,
        bitmap: BitMap,
    ) -> Self {
        Self {
            base,
            size,
            hwm,
            end,
            prev,
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
    pub fn new(size: usize, align: usize) -> Self {
        Self::with_source(size, align, LibcBlockSource)
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
    fn with_source(size: usize, align: usize, source: S) -> Self {
        debug_assert!(
            align.is_power_of_two(),
            "alignment value:{} is not a power of two.",
            align
        );
        let align = max(align, POINTER_ALIGN);

        let size = max(size, POINTER_SIZE);
        let aligned_size =
            Self::align_up(size, align).unwrap_or_else(|| AllocErr::Overflow.panic());
        Self {
            freelist: FreeList::dangling(),
            slot_size: aligned_size,
            slot_align: align,
            active_block: EMPTY_BLOCK.get_inner(),
            source,
        }
    }

    /// Allocates one raw slot and panics if allocation fails.
    ///
    /// The returned pointer owns `self.slot_size` bytes until it is returned
    /// with `free`.
    pub fn alloc(&mut self) -> *mut u8 {
        self.try_allocate().unwrap_or_else(|err| err.panic())
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

    /// Fallible raw allocation entry point used by `alloc` and typed helpers.
    fn try_allocate(&mut self) -> Result<*mut u8, AllocErr> {
        if let Some(ptr) = self.try_allocate_fast() {
            Ok(ptr)
        } else {
            self.try_allocate_slow()
        }
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
                let slot_index = self.get_slot_index(slot, block);
                (block).bitmap.set(slot_index, None);
            }
            return Some(slot);
        }

        // If no slot was freed, hand out the next never-used slot.
        unsafe {
            let block = self.active_block;

            let hwm = (*block).hwm;
            let end = (*block).end;

            if hwm as usize + self.slot_size <= end as usize {
                let slot = hwm;
                let slot_index = self.get_slot_index(slot, &*block);

                (*block).bitmap.set(slot_index, None);
                (*block).hwm = hwm.add(self.slot_size);

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
        let prev_size = unsafe { (*self.active_block).size() };
        let aligned_size = {
            match Self::align_up(prev_size, self.source.page_size()) {
                Some(size) => size,
                None => return Err(AllocErr::Overflow),
            }
        };

        let new_block_size: usize = match prev_size.checked_mul(MUL_CONSTANT) {
            Some(s) => s.max(aligned_size),
            _ => aligned_size,
        };
        self.new_block(new_block_size)?;
        Ok(self.try_allocate_fast().unwrap())
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
        let remaining_bytes = new_block_size - size_of::<Block>();
        let bitmap_size = BitMap::required_bytes(remaining_bytes, self.slot_size);
        let header_bitmap_size = self.get_header_bitmap_size(bitmap_size);
        let new_block = unsafe {
            Block::new(
                ptr,
                new_block_size,
                ptr.add(header_bitmap_size),
                ptr.add(new_block_size),
                self.active_block,
                BitMap::new(ptr.add(size_of::<Block>()), bitmap_size),
            )
        };
        unsafe {
            (ptr as *mut Block).write(new_block);
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
    /// Callers use this only after sizes have already been bounded by mapped
    /// block sizes.
    fn align_up_unchecked(size: usize, align: usize) -> usize {
        (size + align - 1) & !(align - 1)
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
    fn get_slot_index(&self, ptr: *mut u8, block: &Block) -> usize {
        let first_block = self.get_first_block_ptr(block) as usize;
        debug_assert_eq!(
            (ptr as usize - first_block) % self.slot_size,
            0,
            "{}",
            AllocErr::MisalignedFree.panic()
        );
        ((ptr as usize) - (first_block)) / self.slot_size
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
