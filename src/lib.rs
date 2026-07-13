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
