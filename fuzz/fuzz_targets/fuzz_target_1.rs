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
        match byte % 3 {
            0 => {
                let ptr = pool.alloc();
                if !ptr.is_null() {
                    live.push(ptr);
                }
            }
            1 if !live.is_empty() => {
                let idx = (byte as usize) % live.len();
                let ptr = live.swap_remove(idx);
                pool.free(ptr);
            }
            _ => {}
        }
    }

    for ptr in live {
        pool.free(ptr);
    }
});
