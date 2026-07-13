pub const BITS_PER_BYTE: usize = 8;

#[derive(Debug)]
pub(crate) struct BitMap {
    ptr: *mut u8,
    size: usize,
}
impl BitMap {
    pub const fn new(p: *mut u8, size: usize) -> Self {
        Self { ptr: p, size }
    }
    pub const fn dangling() -> Self {
        Self {
            ptr: core::ptr::null_mut(),
            size: 0,
        }
    }

    pub fn required_bytes(total_bytes: usize, slot_size: usize) -> usize {
        (total_bytes / slot_size).div_ceil(BITS_PER_BYTE)
    }
}
