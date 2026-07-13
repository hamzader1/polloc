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