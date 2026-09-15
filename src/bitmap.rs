#[expect(unused)]
use super::Pool;
/// Number of allocation slots represented by one bitmap byte.
pub const BITS_PER_BYTE: usize = 8;

/// Compact allocation-state table for the slots inside one block.
///
/// Each bit describes one slot:
///
/// ```text
/// bit = 0  slot is free
/// bit = 1  slot is allocated
///
/// byte 0
/// +---+---+---+---+---+---+---+---+
/// | 7 | 6 | 5 | 4 | 3 | 2 | 1 | 0 |
/// +---+---+---+---+---+---+---+---+
///                         ^       ^
///                         |       |
///                        slot 2  slot 0
///
/// byte_index = slot_index / BITS_PER_BYTE
/// bit_index  = slot_index % BITS_PER_BYTE
/// ```
///
/// The bitmap bytes live at the beginning of the same mapped block they
/// describe, directly after the `Block` header.
#[derive(Debug)]
pub(crate) struct BitMap {
    /// Pointer to the first bitmap byte.
    pub(crate) ptr: *mut u8,
    /// Number of bytes reserved for the bitmap.
    pub(crate) size: usize,
}
impl BitMap {
    /// Creates a bitmap over an already reserved byte range.
    pub const fn new(p: *mut u8, size: usize) -> Self {
        Self { ptr: p, size }
    }

    /// Creates an empty sentinel bitmap used by the static empty block.
    ///
    /// No allocation operation should ever read or write this bitmap. It only
    /// exists so the sentinel block can have a complete `Block` value.
    pub const fn dangling() -> Self {
        Self {
            ptr: core::ptr::null_mut(),
            size: 0,
        }
    }

    /// Computes how many bitmap bytes are needed to describe a payload region.
    ///
    /// If a block has `total_bytes` available for slots and each slot has
    /// `slot_size`, then `total_bytes / slot_size` slots can exist. The bitmap
    /// needs one bit per slot, rounded up to whole bytes.
    pub fn required_bytes(total_bytes: usize, slot_size: usize) -> usize {
        (total_bytes / slot_size).div_ceil(BITS_PER_BYTE)
    }

    /// Finds the byte and bit offset that store `idx`.
    ///
    /// For example, slot 13 is byte 1, bit 5:
    ///
    /// ```text
    ///
    /// bytes: [      byte 0    ] [        byte 1        ]
    /// slots:  0 1 2 3 4 5 6 7  | 8 9 10 11 12 13 14 15
    ///                                         ^
    ///                                        slot 13
    /// ```
    fn get_byte_ptr(&self, idx: usize) -> (*mut u8, u8) {
        let byte_index = idx / BITS_PER_BYTE;
        let byte_offset = idx % BITS_PER_BYTE;
        debug_assert!(
            byte_index < self.size,
            "bitmap index {idx} out of bounds (size {})",
            self.size
        );
        unsafe { (self.ptr.add(byte_index), byte_offset as u8) }
    }

    /// Clears every bit. Used by [`Pool::reset`] and block initialization.
    pub(crate) fn zero(&mut self) {
        unsafe {
            self.ptr.write_bytes(0, self.size);
        }
    }

    /// Returns `true` when the first `capacity` bits are all clear.
    ///
    /// [`Pool::shrink_to_fit`] uses this to prove a block holds no live slots
    /// before releasing it. Only the capacity prefix is inspected; padding
    /// bits in the final byte are always zero by construction.
    pub(crate) fn all_clear(&self, capacity: usize) -> bool {
        let full_bytes = capacity / BITS_PER_BYTE;
        for i in 0..full_bytes {
            if unsafe { core::ptr::read(self.ptr.add(i)) } != 0 {
                return false;
            }
        }
        let remainder = capacity % BITS_PER_BYTE;
        if remainder > 0 {
            let mask = (1u8 << remainder) - 1;
            if unsafe { core::ptr::read(self.ptr.add(full_bytes)) } & mask != 0 {
                return false;
            }
        }
        true
    }

    /// Returns whether a slot is free and also returns the byte lookup result.
    ///
    /// The cached byte pointer and bit offset let the caller avoid doing the
    /// same index math again when it immediately flips the same bit.
    pub(crate) fn is_free(&self, idx: usize) -> (bool, (*mut u8, u8)) {
        let (byte_ptr, offset) = self.get_byte_ptr(idx);
        let byte = unsafe { core::ptr::read(byte_ptr) };
        (byte & (1 << offset) == 0, (byte_ptr, offset))
    }

    /// Marks a slot as allocated by setting its bit to 1.
    ///
    /// ```text
    /// before: 0010_0000
    /// mask:   0000_0100
    /// after:  0010_0100
    /// ```
    pub(crate) fn set(&mut self, idx: usize, cached_byte: Option<(*mut u8, u8)>) {
        if let Some((byte_ptr, offset)) = cached_byte {
            let byte = unsafe { core::ptr::read(byte_ptr) };
            let new_byte = byte | (1 << offset);
            unsafe { core::ptr::write(byte_ptr, new_byte) };
            return;
        }

        let (byte_ptr, offset) = self.get_byte_ptr(idx);
        let byte = unsafe { core::ptr::read(byte_ptr) };
        let new_byte = byte | (1 << offset);
        unsafe { core::ptr::write(byte_ptr, new_byte) };
    }

    /// Marks a slot as free by clearing its bit to 0.
    ///
    /// ```text
    /// before: 0010_0100
    /// mask:   1111_1011
    /// after:  0010_0000
    /// ```
    pub(crate) fn clear(&mut self, idx: usize, cached_byte: Option<(*mut u8, u8)>) {
        if let Some((byte_ptr, offset)) = cached_byte {
            let byte = unsafe { core::ptr::read(byte_ptr) };
            let new_byte = byte & !(1 << offset);
            unsafe { core::ptr::write(byte_ptr, new_byte) };
            return;
        }
        let (byte_ptr, offset) = self.get_byte_ptr(idx);
        let byte = unsafe { core::ptr::read(byte_ptr) };

        let new_byte = byte & !(1 << offset);
        unsafe { core::ptr::write(byte_ptr, new_byte) };
    }
}
