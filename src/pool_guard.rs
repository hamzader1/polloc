#[cfg(unix)]
use crate::platform::LibcBlockSource;
#[cfg(windows)]
use crate::platform::WindowsBlockSource;
use crate::{BlockSource, DefaultBlockSource, Pool};
pub(crate) struct PoolGuard<'a, S: BlockSource> {
    pub pool: &'a mut Pool<S>,
    pub ptr: *mut u8,
    pub commited: bool,
}
#[cfg(unix)]
impl<'a> PoolGuard<'a, LibcBlockSource> {
    pub fn new(pool: &'a mut Pool<LibcBlockSource>, ptr: *mut u8) -> Self {
        Self::with_source(pool, ptr)
    }
}

#[cfg(windows)]
impl<'a> PoolGuard<'a, WindowsBlockSource> {
    pub fn new(pool: &'a mut Pool<WindowsBlockSource>, ptr: *mut u8) -> Self {
        Self::with_source(pool, ptr)
    }
}
impl<'a, S: BlockSource> PoolGuard<'a, S> {
    pub fn with_source(pool: &'a mut Pool<S>, ptr: *mut u8) -> Self {
        Self {
            pool,
            ptr,
            commited: false,
        }
    }
    pub fn commit(&mut self) {
        self.commited = true;
    }
}
impl<'a, S: BlockSource> Drop for PoolGuard<'a, S> {
    fn drop(&mut self) {
        if !self.commited {
            self.pool.free(self.ptr);
        }
    }
}
