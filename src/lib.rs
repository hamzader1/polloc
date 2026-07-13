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
use freelist::FreeList;
use platform::Platform;

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
        let aligned_size = Self::align_up(max(size, size_of::<*mut u8>()), align)
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
            Ok(self.try_allocate_slow()?)
        }
    }
    pub fn try_allocate_fast(&mut self) -> Option<*mut u8> {
        // First: check if there is any free slot
        if let Some(slot) = self.freelist.get_slot() {
            return Some(slot);
        }
        // Second: check HWM
        let Block { hwm, end, .. } = unsafe { &mut *self.active_block };
        if *hwm as usize + self.slot_size <= *end as usize {
            let slot = *hwm;
            *hwm = unsafe { (*hwm).add(self.slot_size) };
            return Some(slot);
        }

        None
    }

    
    pub fn align_up_unchecked(size: usize, align: usize) -> usize {
        (size + align - 1) & !(align - 1)
    }
    pub fn align_up(size: usize, align: usize) -> Option<usize> {
        match size.checked_add(align - 1) {
            Some(s) => Some(s & !(align - 1)),
            _ => panic!("OVERFLOW"),
        }
    }
}
