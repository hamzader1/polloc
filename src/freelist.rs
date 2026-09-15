#[expect(unused)]
use super::Pool;
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
            self.head = next;
            return Some(current);
        }
        None
    }

    /// Drops every entry, forgetting all reusable slots.
    ///
    /// Used by [`Pool::reset`] (all slots become never allocated again) and by
    /// [`Pool::shrink_to_fit`] before it rebuilds the list without released
    /// blocks. Live allocations are unaffected: they are tracked by the
    /// per block bitmaps, not by this list.
    pub fn clear(&mut self) {
        self.head = core::ptr::null_mut();
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
