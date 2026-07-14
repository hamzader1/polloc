///
///
/// This module isolates the operating-system interface from the allocator
/// logic. The arena asks for anonymous private memory mappings through `mmap`
/// and releases them with `munmap`. Keeping this behind `Platform` makes the
/// allocator core independent from the exact platform API and leaves room for a
/// future Windows implementation using a different backend.
use core::ptr::null_mut;
use libc::c_int;
use libc::c_void;
use libc::munmap;
use libc::off_t;
use libc::sysconf;
use libc::_SC_PAGE_SIZE;
use libc::MAP_ANONYMOUS;
use libc::MAP_FAILED;
use libc::MAP_PRIVATE;
use libc::PROT_READ;
use libc::PROT_WRITE;

#[cfg(unix)]
use core::ptr::null_mut;
#[cfg(windows)]
use core::ffi::c_void;
#[cfg(unix)]
use libc::{
    c_int, c_void, munmap, off_t, sysconf, MAP_ANONYMOUS, MAP_FAILED, MAP_PRIVATE, PROT_READ,
    PROT_WRITE, _SC_PAGE_SIZE,
};
#[cfg(windows)]
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
#[cfg(windows)]
use windows::Win32::System::SystemInformation::GetSystemInfo;

#[cfg(unix)]
const FLAG: c_int = MAP_PRIVATE | MAP_ANONYMOUS;
const PROT: c_int = PROT_READ | PROT_WRITE;
const FD: c_int = -1;
const OFFSET: off_t = 0;

#[cfg(unix)]
pub struct LibcBlockSource;

#[cfg(unix)]
impl BlockSource for LibcBlockSource {
    fn page_size(&self) -> usize {
        unsafe { sysconf(_SC_PAGE_SIZE) as usize }
    }

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

    fn unmap(&mut self, addr: *mut u8, size: usize) {
        unsafe {
            munmap(addr as *mut c_void, size);
        }
    }
}

#[cfg(windows)]
pub struct WindowsBlockSource;
#[cfg(windows)]
impl BlockSource for WindowsBlockSource {
    fn map(&mut self, size: usize) -> *mut u8 {
        unsafe {
            VirtualAlloc(None, size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) as *mut u8
        }
    }

    fn unmap(&mut self, ptr: *mut u8, _size: usize) {
        unsafe {
            let _ = VirtualFree(ptr as *mut c_void, 0, MEM_RELEASE);
        }
    }

    fn page_size(&self) -> usize {
        unsafe {
            let mut info = std::mem::zeroed();
            GetSystemInfo(&mut info);
            info.dwPageSize as usize
        }
    }
}
