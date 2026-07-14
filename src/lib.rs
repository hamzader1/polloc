#![allow(warnings)]
mod bitmap;
mod errors;
mod freelist;
mod platform;
use bitmap::BitMap;
use core::alloc::Layout;
use core::cmp::max;
use core::hash;
use core::ptr::{self, null_mut};
use errors::AllocErr;
use freelist::FreeList;
use platform::Platform;
use std::ptr::null;

const POINTER_SIZE: usize = size_of::<*mut u8>();
const MUL_CONSTANT: usize = 2;

#[derive(Debug)]
pub struct Pool {
    freelist: FreeList,
    slot_size: usize,
    slot_align: usize,
    active_block: *mut Block,
}

#[derive(Debug)]
pub struct Block {
    base: *mut u8,
    size: usize,
    hwm: *mut u8,
    end: *mut u8,
    prev: *mut Block,
    bitmap: BitMap,
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
impl Pool {
    pub fn new(size: usize, align: usize) -> Self {
        // Validate the align is power of two
        debug_assert!(
            align.is_power_of_two(),
            "alignment value:{} is not a power of two.",
            align
        );
        // Validate the size
        let aligned_size = Self::align_up(max(size, POINTER_SIZE), align)
            .unwrap_or_else(|| AllocErr::Overflow.panic());
        Self {
            freelist: FreeList::dangling(),
            slot_size: aligned_size,
            slot_align: align,
            active_block: EMPTY_BLOCK.get_inner(),
        }
    }
    pub fn alloc(&mut self) -> *mut u8 {
        self.try_allocate().unwrap_or_else(|err| err.panic())
    }
    pub fn try_allocate(&mut self) -> Result<*mut u8, AllocErr> {
        if let Some(ptr) = self.try_allocate_fast() {
            Ok(ptr)
        } else {
            self.try_allocate_slow()
        }
    }
    pub fn try_allocate_fast(&mut self) -> Option<*mut u8> {
        // First: check if there is any free slot
        if let Some(slot) = self.freelist.get_slot() {
            unsafe {
                let slot_index = self.get_slot_index(slot, &*self.active_block);
                (*self.active_block).bitmap.set(slot_index, None);
            }
            return Some(slot);
        }
        // Second: check HWM
        let Block { hwm, end, .. } = unsafe { &mut *self.active_block };
        if *hwm as usize + self.slot_size <= *end as usize {
            let slot = *hwm;
            unsafe {
                let slot_index = self.get_slot_index(slot, &*self.active_block);
                (*self.active_block).bitmap.set(slot_index, None);
                *hwm = (*hwm).add(self.slot_size);
            }
            return Some(slot);
        }

        None
    }
    pub fn try_allocate_slow(&mut self) -> Result<*mut u8, AllocErr> {
        // everyhing is valid, align, size.
        // request memory and start making calculations
        let prev_size = unsafe { (*self.active_block).size() };
        let aligned_size = {
            match Self::align_up(prev_size, Platform::get_page_size()) {
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

    pub fn new_block(&mut self, new_block_size: usize) -> Result<*mut u8, AllocErr> {
        let ptr: *mut u8 = Platform::mmap(new_block_size);
        if ptr.is_null() {
            return Err(AllocErr::OutOfMemory);
        }
        let remaining_bytes = new_block_size - size_of::<Block>();
        let bitmap_size = BitMap::required_bytes(remaining_bytes, self.slot_size);
        let header_bitmap_size = self.get_header_bitmap_size(bitmap_size);
        let mut new_block = unsafe {
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
        Ok(unsafe { ptr.add(header_bitmap_size) })
    }

    pub fn get_first_block_ptr(&self, block: &Block) -> *mut u8 {
        let Block { base, bitmap, .. } = block; // generally

        let offset = Self::align_up_unchecked(size_of::<Block>() + bitmap.size, self.slot_size);
        unsafe { base.add(offset) }
    }
    pub fn get_header_bitmap_size(&self, bitmap_size: usize) -> usize {
        Self::align_up_unchecked(size_of::<Block>() + bitmap_size, self.slot_size)
    }
    pub fn align_up_unchecked(size: usize, align: usize) -> usize {
        (size + align - 1) & !(align - 1)
    }
    pub fn align_up(size: usize, align: usize) -> Option<usize> {
        size.checked_add(align - 1).map(|res| res & !(align - 1))
        //     match size.checked_add(align - 1) {
        //         Some(s) => Some(s & !(align - 1)),
        //         _ => None,
        //     }
    }

    pub fn get_block(&self, ptr: *mut u8) -> *mut Block {
        if ptr.is_null() || ptr > unsafe { &*self.active_block }.hwm {
            return null_mut();
        }
    }
}
