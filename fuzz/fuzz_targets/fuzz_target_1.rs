#![no_main]
use libfuzzer_sys::fuzz_target;
use polloc::Pool;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }

    let size = (data[0] as usize % 256) + 1;
    let align = 1 << (data[1] as usize % 8); // power of two, 1..128
    let mut pool = Pool::new(size, align);
    let mut live: Vec<*mut u8> = Vec::new();

    for &byte in &data[2..] {
        match byte % 6 {
            // Valid alloc.
            0 => {
                let ptr = pool.alloc();
                if !ptr.is_null() {
                    live.push(ptr);
                }
            }
            // Valid free of a live slot.
            1 if !live.is_empty() => {
                let idx = (byte as usize) % live.len();
                let ptr = live.swap_remove(idx);
                pool.free(ptr);
            }
            // Interior pointer: must be rejected as misaligned, and the
            // slot must stay live afterwards.
            2 if !live.is_empty() => {
                let idx = (byte as usize) % live.len();
                let bad = unsafe { live[idx].add(1) };
                assert!(pool.try_free(bad).is_err());
                assert!(pool.contains(live[idx]));
            }
            // Null free: must be rejected.
            3 => {
                assert!(pool.try_free(core::ptr::null_mut()).is_err());
            }
            // Foreign (stack) pointer: must be rejected.
            4 => {
                let mut byte = 0u8;
                assert!(pool.try_free(&mut byte as *mut u8).is_err());
            }
            // Liveness oracle: every tracked slot must read as contained.
            _ => {
                for &ptr in &live {
                    assert!(pool.contains(ptr));
                }
            }
        }
    }

    for ptr in live {
        pool.free(ptr);
    }
    assert!(pool.is_empty());
});
