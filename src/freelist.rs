/// Intrusive stack of slots that were freed and can be reused.
///
/// "Intrusive" means there is no separate node allocation. The first pointer
/// sized bytes of each freed slot store the next free slot address.
///
/// ```text
/// freelist.head
///      |
///      v
/// +---------+      +---------+      +---------+
/// | next ---+----> | next ---+----> | null    |
/// | slot    |      | slot    |      |         |
/// +---------+      +---------+      +---------+
///   slot C          slot B          slot A
///
/// alloc reuses slot C first, so reuse order is LIFO.
/// ```
///
/// A slot must be at least pointer-sized before it can enter the freelist.
/// `Pool::new` enforces that by rounding every slot size up to pointer size.
#[derive(Debug)]
pub struct FreeList {
    /// Top of the free-slot stack.
    head: *mut u8,
}
impl FreeList {
    /// Creates a freelist with an explicit head pointer.
    #[inline(always)]
    pub const fn new(head: *mut u8) -> Self {
        Self { head }
    }

    /// Creates an empty freelist.
    #[inline(always)]
    pub const fn dangling() -> Self {
        Self {
            head: core::ptr::null_mut(),
        }
    }

    /// Returns whether the freelist has no reusable slots.
    #[inline(always)]
    fn is_null(&self) -> bool {
        self.head.is_null()
    }

    /// Pops one reusable slot from the freelist.
    ///
    /// The slot itself stores the next pointer:
    ///
    /// ```text
    /// before pop:
    ///
    /// head
    ///  |
    ///  v
    /// +------+     +------+
    /// | next +---> | next |
    /// +------+     +------+
    ///
    /// after pop:
    ///
    /// returned slot
    ///  |
    ///  v
    /// +------+
    /// | next |
    /// +------+
    ///
    /// head --------> second slot
    /// ```
    pub fn get_slot(&mut self) -> Option<*mut u8> {
        if !self.is_null() {
            let current = self.head;
            let next = unsafe { *(self.head as *mut *mut u8) };
            self.head = next as *mut u8;
            return Some(current);
        }
        None
    }

    /// Pops a slot without checking whether the freelist is empty.
    ///
    /// This is only useful for tests or experiments that deliberately want the
    /// unchecked behavior. Calling it when `head` is null is undefined behavior.
    unsafe fn get_slot_unchecked(&mut self) -> *mut u8 {
        let current = self.head;
        let next = *(self.head as *mut *mut u8);
        self.head = next as *mut u8;
        current
    }

    /// Pushes a freed slot onto the front of the freelist.
    ///
    /// ```text
    /// before:
    ///
    /// head ----> old first
    ///
    /// add_slot(new)
    ///
    /// new slot stores old head:
    ///
    /// +-----------+
    /// | old head -+----> old first
    /// +-----------+
    ///
    /// after:
    ///
    /// head ----> new slot ----> old first
    /// ```
    pub fn add_slot(&mut self, ptr: *mut u8) {
        unsafe {
            *(ptr as *mut *mut u8) = self.head;
            self.head = ptr;
        }
    }
}
