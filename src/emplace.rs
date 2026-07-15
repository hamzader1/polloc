use crate::pool_guard::PoolGuard;
use crate::BlockSource;
use std::mem::MaybeUninit;

pub(crate) struct Emplace<'a, S: BlockSource, T> {
    guard: PoolGuard<'a, S>,
    ptr: *mut MaybeUninit<T>,
}

impl<'a, T, S: BlockSource> Emplace<'a, S, T> {
    pub(crate) fn with_source(guard: PoolGuard<'a, S>) -> Self {
        let ptr = guard.ptr as *mut MaybeUninit<T>;
        Self { guard, ptr }
    }
}

impl<'a, S: BlockSource, T> Emplace<'a, S, T> {
    pub(crate) fn write(mut self, value: T) -> *mut T {
        unsafe {
            core::ptr::write(self.ptr as *mut T, value);
        }
        self.assume_init()
    }
    fn assume_init(&mut self) -> *mut T {
        self.guard.commit();
        self.ptr as *mut T
    }
}
