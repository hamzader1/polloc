/// Supplies and releases raw blocks of virtual memory for `Pool`.
///
/// `Pool` does not care whether memory came from `mmap`, `VirtualAlloc`, a
/// fixed test buffer, or some future platform backend. It only needs three
/// operations:
///
/// ```text
/// +------+     map(size)      +--------------------+
/// | Pool | -----------------> | raw block of bytes |
/// +------+                    +--------------------+
///      ^                              |
///      |          unmap(ptr, size)    |
///      +------------------------------+
/// ```
///
/// Implementations must return a block that is readable and writable. On
/// allocation failure, `map` should return a null pointer.
pub trait BlockSource {
    /// Reserves or maps `size` bytes and returns the base address.
    ///
    /// The returned pointer must be the same pointer later accepted by
    /// `unmap`. A null pointer means the backend could not provide memory.
    fn map(&mut self, size: usize) -> *mut u8;

    /// Releases a block previously returned by `map`.
    ///
    /// `ptr` must be the original mapping base. `size` is the size originally
    /// requested from the backend, except for backends whose OS API ignores it
    /// during release.
    fn unmap(&mut self, ptr: *mut u8, size: usize);

    /// Returns the platform allocation granularity used for block growth.
    ///
    /// `Pool` rounds block sizes up to this value so it asks the OS for
    /// naturally sized chunks instead of odd byte counts.
    fn page_size(&self) -> usize;
}
