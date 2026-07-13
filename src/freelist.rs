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
    pub fn get_slot(&mut self) -> Option<*mut u8> {
        if !self.is_null() {
            let current = self.head;
            let next = unsafe {
                *(self.head as *mut *mut u8) /* ptr to the first 8 bytes before deref */
            }; // after deref read first 8 bytes
            self.head = next as *mut u8;
            return Some(current);
        }
        None
    }
    unsafe fn get_slot_unchecked(&mut self) -> *mut u8 {
        let current = self.head;
        // will cause SegFault; used for testing only
        let next = *(self.head as *mut *mut u8);
        self.head = next as *mut u8;
        current
    }
}
