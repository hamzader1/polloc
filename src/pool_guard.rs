use crate::{BlockSource, Pool};

pub(crate) struct PoolGuard<'a, S: BlockSource> {
    pool: &'a mut Pool<S>,
    pub(crate) ptr: *mut u8,
    commited: bool,
}

impl<'a, S: BlockSource> PoolGuard<'a, S> {
    pub(crate) fn with_source(pool: &'a mut Pool<S>, ptr: *mut u8) -> Self {
        Self {
            pool,
            ptr,
            commited: false,
        }
    }
    pub(crate) fn commit(&mut self) {
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
