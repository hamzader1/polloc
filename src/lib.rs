#![allow(warnings)]
mod bitmap;
mod freelist;
mod platform;
use bitmap::BitMap;
use core::alloc::Layout;
use core::cmp::max;
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
            .unwrap_or_else(|| panic!("SIZE OVERFLOW, TRY SMALLER SIZE"));
        Self {
            freelist: FreeList::dangling(),
            slot_size: aligned_size,
            slot_align: align,
            active_block: EMPTY_BLOCK.get_inner(),
        }
    }
}
