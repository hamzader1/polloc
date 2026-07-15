use crate::block_source::BlockSource;

#[cfg(windows)]
use core::ffi::c_void;
#[cfg(unix)]
use core::ptr::null_mut;
#[cfg(unix)]
use libc::{
    _SC_PAGE_SIZE, MAP_ANONYMOUS, MAP_FAILED, MAP_PRIVATE, PROT_READ, PROT_WRITE, c_int, c_void,
    munmap, off_t, sysconf,
};
#[cfg(windows)]
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetSystemInfo;

/// Unix `mmap` flags used for private anonymous memory.
#[cfg(unix)]
const FLAG: c_int = MAP_PRIVATE | MAP_ANONYMOUS;
/// Unix page permissions used for allocator blocks.
#[cfg(unix)]
const PROT: c_int = PROT_READ | PROT_WRITE;
/// File descriptor value required for anonymous mappings.
#[cfg(unix)]
const FD: c_int = -1;
/// File offset for anonymous mappings.
#[cfg(unix)]
const OFFSET: off_t = 0;

/// Block source backed by libc virtual-memory calls on Unix.
///
/// The allocator asks this source for raw pages. The returned memory is not
/// initialized by Rust; the pool writes its own `Block` header and bitmap into
/// the beginning of the mapping.
#[cfg(unix)]
pub struct LibcBlockSource;

#[cfg(unix)]
impl BlockSource for LibcBlockSource {
    /// Reads the system page size through `sysconf`.
    fn page_size(&self) -> usize {
        unsafe { sysconf(_SC_PAGE_SIZE) as usize }
    }

    /// Maps readable and writable anonymous memory.
    ///
    /// POSIX reports failure as `MAP_FAILED`, but the allocator expects null on
    /// failure, so this converts the OS sentinel into a null pointer.
    fn map(&mut self, size: usize) -> *mut u8 {
        unsafe {
            let ptr = libc::mmap(null_mut(), size, PROT, FLAG, FD, OFFSET);
            if ptr == MAP_FAILED {
                null_mut()
            } else {
                ptr as *mut u8
            }
        }
    }

    /// Unmaps a block previously returned by `map`.
    fn unmap(&mut self, addr: *mut u8, size: usize) {
        unsafe {
            munmap(addr as *mut c_void, size);
        }
    }
}

/// Block source backed by Windows virtual memory.
///
/// Windows reserves and commits the block in one call here:
///
/// ```text
/// VirtualAlloc
///    |
///    v
/// +-----------------------------+
/// | committed read/write memory |
/// +-----------------------------+
/// ```
#[cfg(windows)]
pub struct WindowsBlockSource;
#[cfg(windows)]
impl BlockSource for WindowsBlockSource {
    /// Reserves and commits a readable and writable block.
    fn map(&mut self, size: usize) -> *mut u8 {
        unsafe { VirtualAlloc(None, size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) as *mut u8 }
    }

    /// Releases a block previously returned by `VirtualAlloc`.
    ///
    /// `MEM_RELEASE` requires a size of zero and the exact base pointer.
    fn unmap(&mut self, ptr: *mut u8, _size: usize) {
        unsafe {
            let _ = VirtualFree(ptr as *mut c_void, 0, MEM_RELEASE);
        }
    }

    /// Reads the Windows page size using `GetSystemInfo`.
    fn page_size(&self) -> usize {
        unsafe {
            let mut info = std::mem::zeroed();
            GetSystemInfo(&mut info);
            info.dwPageSize as usize
        }
    }
}
