pub const BITS_PER_BYTE: usize = 8;

#[derive(Debug)]
pub(crate) struct BitMap {
    pub(crate) ptr: *mut u8,
    pub(crate) size: usize,
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
    fn get_byte_ptr(&self, idx: usize) -> (*mut u8, u8) {
        let byte_index = idx / BITS_PER_BYTE;
        let byte_offset = idx % BITS_PER_BYTE;
        unsafe { (self.ptr.add(byte_index), byte_offset as u8) }
        // 0
    }
    pub fn is_free(&self, idx: usize) -> (bool, (*mut u8, u8)) {
        let (byte_ptr, offset) = self.get_byte_ptr(idx);
        let byte = unsafe { core::ptr::read(byte_ptr) };
        (byte & (1 << offset) == 0, (byte_ptr, offset))
    }
    pub fn set(&mut self, idx: usize, cached_byte: Option<(*mut u8, u8)>) {
        if let Some((byte_ptr, offset)) = cached_byte {
            let byte = unsafe { core::ptr::read(byte_ptr) };
            let new_byte = byte | (1 << offset);
            unsafe { core::ptr::write(byte_ptr, new_byte) };
        }
        // IN CASE WE DON'T HAVE PRE-COMPUTED RESULTS
        let (byte_ptr, offset) = self.get_byte_ptr(idx);
        let byte = unsafe { core::ptr::read(byte_ptr) };
        let new_byte = byte | (1 << offset);
        unsafe { core::ptr::write(byte_ptr, new_byte) };
    }
    pub fn clear(&mut self, idx: usize, cached_byte: Option<(*mut u8, u8)>) {
        if let Some((byte_ptr, offset)) = cached_byte {
            let byte = unsafe { core::ptr::read(byte_ptr) };
            let new_byte = byte & !(1 << offset);
            unsafe { core::ptr::write(byte_ptr, new_byte) };
        }
        // IN CASE WE DON'T HAVE PRE-COMPUTED RESULTS
        let (byte_ptr, offset) = self.get_byte_ptr(idx);
        let byte = unsafe { core::ptr::read(byte_ptr) };

        let new_byte = byte & !(1 << offset);
        unsafe { core::ptr::write(byte_ptr, new_byte) };
    }
}
