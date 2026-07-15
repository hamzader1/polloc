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
use errors::AllocErr;
use freelist::FreeList;

pub mod block_source;
pub use block_source::BlockSource;

const POINTER_SIZE: usize = size_of::<*mut u8>();
const POINTER_ALIGN: usize = align_of::<*mut u8>();
const MUL_CONSTANT: usize = 2;

#[cfg(unix)]
pub type DefaultBlockSource = LibcBlockSource;

#[cfg(windows)]
pub type DefaultBlockSource = WindowsBlockSource;

#[derive(Debug)]
pub struct Pool<S: BlockSource = DefaultBlockSource> {
    pub freelist: FreeList,
    pub slot_size: usize,
    pub slot_align: usize,
    pub active_block: *mut Block,
    source: S,
}

#[derive(Debug)]
pub struct Block {
    pub base: *mut u8,
    pub size: usize,
    pub hwm: *mut u8,
    pub end: *mut u8,
    pub prev: *mut Block,
    pub bitmap: BitMap,
}

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
    fn get_inner(&self) -> *mut Block {
        &self.0 as *const Block as *mut Block
    }
}
impl Block {
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
    fn ptr(&self) -> *mut u8 {
        self.base
    }
    fn size(&self) -> usize {
        self.size
    }
    fn hwm(&self) -> *mut u8 {
        self.hwm
    }
    fn prev(&self) -> *mut Block {
        self.prev
    }
}
#[cfg(unix)]
impl Pool<LibcBlockSource> {
    pub fn new(size: usize, align: usize) -> Self {
        Self::with_source(size, align, LibcBlockSource)
    }
}

#[cfg(windows)]
impl Pool<WindowsBlockSource> {
    pub fn new(size: usize, align: usize) -> Self {
        Self::with_source(size, align, WindowsBlockSource)
    }
}

impl<S: BlockSource> Pool<S> {
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
    pub fn alloc(&mut self) -> *mut u8 {
        self.try_allocate().unwrap_or_else(|err| err.panic())
    }
    pub fn try_allocate_with<T, F>(&mut self, f: F) -> Result<*mut T, AllocErr>
    where
        F: FnOnce() -> T,
    {
        self.validate_size_align::<T>()?;
        Ok((self.try_emplace()?).write(f()))
    }

    fn try_allocate(&mut self) -> Result<*mut u8, AllocErr> {
        if let Some(ptr) = self.try_allocate_fast() {
            Ok(ptr)
        } else {
            self.try_allocate_slow()
        }
    }
    pub fn try_emplace<'a, T>(&'a mut self) -> Result<Emplace<'a, S, T>, AllocErr> {
        let ptr = self.try_allocate()?;
        let guard = PoolGuard::with_source(self, ptr);
        Ok(Emplace::with_source(guard))
    }

    fn try_allocate_fast(&mut self) -> Option<*mut u8> {
        // First: check if there is any free slot
        if let Some(slot) = self.freelist.get_slot() {
            unsafe {
                let block = &mut *self.get_block(slot);
                let slot_index = self.get_slot_index(slot, block);
                (block).bitmap.set(slot_index, None);
            }
            return Some(slot);
        }
        // Second: check HWM
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
    fn try_allocate_slow(&mut self) -> Result<*mut u8, AllocErr> {
        // everyhing is valid, align, size.
        // request memory and start making calculations
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

        // Ok(null_mut())
    }

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
        // Ok(unsafe { ptr.add(header_bitmap_size) })
    }

    fn get_first_block_ptr(&self, block: &Block) -> *mut u8 {
        let Block { base, bitmap, .. } = block; // generally

        let offset = Self::align_up_unchecked(size_of::<Block>() + bitmap.size, self.slot_size);
        unsafe { base.add(offset) }
    }
    fn get_header_bitmap_size(&self, bitmap_size: usize) -> usize {
        Self::align_up_unchecked(size_of::<Block>() + bitmap_size, self.slot_size)
    }
    fn align_up_unchecked(size: usize, align: usize) -> usize {
        (size + align - 1) & !(align - 1)
    }
    fn align_up(size: usize, align: usize) -> Option<usize> {
        size.checked_add(align - 1).map(|res| res & !(align - 1))
        //     match size.checked_add(align - 1) {
        //         Some(s) => Some(s & !(align - 1)),
        //         _ => None,
        //     }
    }

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
    // unsafe: only valid when pointer lies within the region
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

    pub fn free(&mut self, ptr: *mut u8) {
        self.try_free(ptr).map_err(|err| err.panic());
    }
    fn try_free(&mut self, ptr: *mut u8) -> Result<(), AllocErr> {
        // is the pointer valid for future use
        let block_ptr = self.get_block(ptr);
        if block_ptr.is_null() {
            return Err(AllocErr::InvalidPointer);
        }
        let block = unsafe { &mut *block_ptr };
        // safety: ensure the slot is free; detect double free
        let slot_index = self.get_slot_index(ptr, block);
        let (is_slot_free, cached_byte) = block.bitmap.is_free(slot_index);
        if is_slot_free {
            return Err(AllocErr::DoubleFree);
        }
        block.bitmap.clear(slot_index, Some(cached_byte));
        self.freelist.add_slot(ptr);
        Ok(())
    }
    pub fn validate_size_align<T>(&self) -> Result<(), AllocErr> {
        if size_of::<T>() <= self.slot_size && align_of::<T>() <= self.slot_align {
            Ok(())
        } else {
            Err(AllocErr::InvalidAlignment)
        }
    }
}

impl<S: BlockSource> Drop for Pool<S> {
    fn drop(&mut self) {
        while self.active_block != EMPTY_BLOCK.get_inner() {
            let current = unsafe { &*self.active_block };
            self.active_block = current.prev;
            self.source.unmap(current.base as *mut u8, current.size);
        }
    }
}
