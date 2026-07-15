use crate::BlockSource;
use crate::pool_guard::PoolGuard;
use std::mem::MaybeUninit;

/// Typed view over a guarded raw slot.
///
/// `Emplace` is the bridge between "the pool has reserved bytes" and "a `T`
/// has been written into those bytes".
///
/// ```text
/// raw slot (*mut u8)
///      |
///      v
/// +------------------+
/// | MaybeUninit<T>   |
/// +------------------+
///      |
///      | write(value)
///      v
/// +------------------+
/// | initialized T    |
/// +------------------+
/// ```
///
/// The type is crate-private because callers use `Pool::try_allocate_with`
/// instead of manually driving the guard protocol.
pub(crate) struct Emplace<'a, S: BlockSource, T> {
    /// Guard that frees the slot unless initialization commits.
    guard: PoolGuard<'a, S>,
    /// Slot pointer interpreted as uninitialized storage for `T`.
    ptr: *mut MaybeUninit<T>,
}

impl<'a, T, S: BlockSource> Emplace<'a, S, T> {
    /// Creates a typed emplacement wrapper around a guarded raw slot.
    pub(crate) fn with_source(guard: PoolGuard<'a, S>) -> Self {
        let ptr = guard.ptr as *mut MaybeUninit<T>;
        Self { guard, ptr }
    }
}

impl<'a, S: BlockSource, T> Emplace<'a, S, T> {
    /// Writes `value` into the guarded slot and returns the initialized pointer.
    ///
    /// After the write succeeds, the guard is committed so it will not free the
    /// slot on drop.
    pub(crate) fn write(mut self, value: T) -> *mut T {
        unsafe {
            core::ptr::write(self.ptr as *mut T, value);
        }
        self.assume_init()
    }

    /// Commits the guard and treats the slot as initialized storage for `T`.
    fn assume_init(&mut self) -> *mut T {
        self.guard.commit();
        self.ptr as *mut T
    }
}
