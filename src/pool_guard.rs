use crate::{BlockSource, Pool};

/// RAII guard for an allocated slot that has not been committed yet.
///
/// This guard exists to make typed allocation panic-safe. The pool first
/// allocates a raw slot, then constructs the user's value. If construction
/// panics before `commit`, `Drop` returns the slot to the pool.
///
/// ```text
/// raw slot allocated
///      |
///      v
/// +-----------+
/// | PoolGuard |
/// +-----------+
///      |
///      +-- commit() called --> keep allocation
///      |
///      +-- dropped uncommitted --> free slot
/// ```
pub(crate) struct PoolGuard<'a, S: BlockSource> {
    /// Pool that owns the guarded slot.
    pool: &'a mut Pool<S>,
    /// Raw slot being protected until typed initialization succeeds.
    pub(crate) ptr: *mut u8,
    /// Whether the slot has been handed to the caller.
    commited: bool,
}

impl<'a, S: BlockSource> PoolGuard<'a, S> {
    /// Creates an uncommitted guard for `ptr`.
    pub(crate) fn with_source(pool: &'a mut Pool<S>, ptr: *mut u8) -> Self {
        Self {
            pool,
            ptr,
            commited: false,
        }
    }

    /// Marks the slot as successfully initialized and owned by the caller.
    pub(crate) fn commit(&mut self) {
        self.commited = true;
    }
}

impl<'a, S: BlockSource> Drop for PoolGuard<'a, S> {
    /// Returns the slot to the pool if initialization never committed.
    fn drop(&mut self) {
        if !self.commited {
            self.pool.free(self.ptr);
        }
    }
}
