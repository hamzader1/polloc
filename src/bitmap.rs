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
}
