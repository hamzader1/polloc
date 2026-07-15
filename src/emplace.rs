use crate::pool_guard::PoolGuard;
#[cfg(unix)]
use crate::LibcBlockSource;
#[cfg(windows)]
use crate::WindowsBlockSource;
use crate::{BlockSource, DefaultBlockSource};
use std::mem::MaybeUninit;

pub(crate) struct Emplace<'a, S: BlockSource, T> {
    guard: PoolGuard<'a, S>,
    ptr: *mut MaybeUninit<T>,
}

impl<'a, T, S: BlockSource> Emplace<'a, S, T> {
    pub fn with_source(guard: PoolGuard<'a, S>) -> Self {
        let ptr = guard.ptr as *mut MaybeUninit<T>;
        Self { guard, ptr }
    }
}
#[cfg(unix)]
impl<'a, T> Emplace<'a, LibcBlockSource, T> {
    pub fn new(guard: PoolGuard<'a, LibcBlockSource>) -> Self {
        Self::with_source(guard)
    }
}

#[cfg(windows)]
impl<'a, T> Emplace<'a, WindowsBlockSource, T> {
    pub fn new(guard: PoolGuard<'a, WindowsBlockSource>) -> Self {
        Self::with_source(guard)
    }
}

impl<'a, S: BlockSource, T> Emplace<'a, S, T> {
    pub fn write(mut self, value: T) -> *mut T {
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
