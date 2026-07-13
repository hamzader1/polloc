#[derive(Debug)]
pub struct FreeList {
    head: *mut u8,
}
impl FreeList {
    #[inline(always)]
    pub const fn new(head: *mut u8) -> Self {
        Self { head }
    }
    #[inline(always)]
    pub const fn dangling() -> Self {
        Self {
            head: core::ptr::null_mut(),
        }
    }
    #[inline(always)]
    fn is_null(&self) -> bool {
        self.head.is_null()
    }
}
