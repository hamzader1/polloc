use crate::{BlockSource, Pool};
use core::fmt;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

/// Owning handle for one pool slot holding a `T`.
///
/// `Slot` is the safe face of the allocator: it dereferences to `T` like a
/// `Box<T>`, and on drop it runs `T`'s destructor (if any) and returns the
/// slot to the pool. Because the handle borrows the pool mutably for its
/// whole lifetime, the borrow checker guarantees:
///
/// ```text
/// Pool ──mut borrow──> Slot<T> ──deref──> T
///   ^                                          |
///   |           Drop: drop(T) + Pool.free()    |
///   +──────────────────────────────────────────+
/// ```
///
/// * the pool outlives every outstanding slot,
/// * no second mutable borrow of the pool exists while a slot is alive
///   (so a slot can never be freed twice or out from under its owner),
/// * the pool itself cannot be dropped while slots are alive.
///
/// Raw [`Pool::alloc`] / [`Pool::free`] remain available for `no_std` style
/// or FFI use, but new code should prefer `alloc_value` / `alloc_uninit`.
///
/// `Slot` is single threaded exactly like [`Pool`]: it is `!Send + !Sync`.
pub struct Slot<'a, T, S: BlockSource> {
    ptr: NonNull<T>,
    pool: *mut Pool<S>,
    _borrow: PhantomData<&'a mut Pool<S>>,
}

impl<'a, T, S: BlockSource> Slot<'a, T, S> {
    /// Wraps a freshly allocated, already initialized slot.
    ///
    /// `ptr` must be a live allocation of `pool` holding a valid `T`.
    pub(crate) fn new(pool: &'a mut Pool<S>, ptr: *mut T) -> Self {
        Self {
            // `Pool` never hands out null on success.
            ptr: unsafe { NonNull::new_unchecked(ptr) },
            pool: pool as *mut Pool<S>,
            _borrow: PhantomData,
        }
    }

    /// Returns the raw pointer to the value in the slot.
    pub fn as_ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    /// Gives up ownership without dropping `T` or freeing the slot.
    ///
    /// The caller takes responsibility for the value: eventually either
    /// rebuild the handle (no safe API exists on purpose) or call
    /// [`Pool::free_value`] to run the destructor and recycle the slot.
    /// Forgetting the pointer entirely leaks the value but never frees the
    /// backing block while the pool lives, so this is a leak, not UB.
    pub fn into_raw(self) -> *mut T {
        let this = ManuallyDrop::new(self);
        this.ptr.as_ptr()
    }
}

impl<T, S: BlockSource> Deref for Slot<'_, T, S> {
    type Target = T;

    /// Shared access to the slotted value.
    fn deref(&self) -> &T {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T, S: BlockSource> DerefMut for Slot<'_, T, S> {
    /// Exclusive access to the slotted value.
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.ptr.as_mut() }
    }
}

impl<T: fmt::Debug, S: BlockSource> fmt::Debug for Slot<'_, T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Slot")
            .field("ptr", &self.ptr)
            .field("value", &**self)
            .finish()
    }
}

impl<T, S: BlockSource> Drop for Slot<'_, T, S> {
    /// Drops the value in place, then returns the slot to the pool.
    ///
    /// The free cannot fail for a handle minted by this pool: the slot is
    /// live and correctly aligned by construction. A failure would mean pool
    /// memory was corrupted through raw pointer misuse elsewhere, so debug
    /// builds assert loudly while release builds avoid a double panic during
    /// unwinding.
    fn drop(&mut self) {
        unsafe {
            self.ptr.drop_in_place();
            let result = (*self.pool).try_free(self.ptr.as_ptr() as *mut u8);
            debug_assert!(result.is_ok());
            let _ = result;
        }
    }
}

impl<'a, T, S: BlockSource> Slot<'a, MaybeUninit<T>, S> {
    /// Writes `value` into an uninitialized slot, yielding `Slot<T>`.
    ///
    /// The pool is not touched: the slot is already owned by this handle,
    /// so initialization is just a placement write plus a type level state
    /// change from `MaybeUninit<T>` to `T`.
    pub fn write(self, value: T) -> Slot<'a, T, S> {
        let this = ManuallyDrop::new(self);
        unsafe {
            (this.ptr.as_ptr().cast::<T>()).write(value);
        }
        Slot {
            ptr: this.ptr.cast::<T>(),
            pool: this.pool,
            _borrow: PhantomData,
        }
    }
}
