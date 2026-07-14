pub trait BlockSource {
    fn map(&mut self, size: usize) -> *mut u8;
    fn unmap(&mut self, ptr: *mut u8, size: usize);
    fn page_size(&self) -> usize;
}
