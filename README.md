# Polloc

`polloc` is a small fixed-size pool allocator written in Rust.

It allocates memory in slots of one size and one alignment per pool. Freed slots
are reused through an intrusive free list, and each block keeps a bitmap so the
allocator can detect invalid states such as double frees.

This project is mainly for learning allocator internals: block layout, free
lists, bitmap tracking, typed emplacement, and platform-backed virtual memory.
The source code is documented with inline comments and ASCII diagrams.

## Basic Use

```rust
use polloc::Pool;

let mut pool = Pool::new(64, 8);

let ptr = pool.alloc();

unsafe {
    ptr.write(42);
}

pool.free(ptr);
```

Typed allocation is available through `try_allocate_with`:

```rust
use polloc::Pool;
use std::mem::{align_of, size_of};

let mut pool = Pool::new(size_of::<u64>(), align_of::<u64>());

let value = pool.try_allocate_with(|| 123_u64).unwrap();

unsafe {
    assert_eq!(*value, 123);
}

pool.free(value as *mut u8);
```

## How It Works

Each pool has a fixed slot size and slot alignment. The allocator first tries to
reuse a freed slot. If none exists, it allocates from the active block's
high-water mark. If the active block has no room left, it maps a new block.

```text
                         hwm       end
                          |         |
                          v         v
+--------+--------+---+---+---------+
| header | bitmap | A | B |  free   |
+--------+--------+---+---+---------+
                          |
                          | request fits
                          v

+--------+--------+---+---+---+------+
| header | bitmap | A | B | C | free |
+--------+--------+---+---+---+------+
                          ^
                          new allocation
```

When a slot is freed, it is pushed onto the free list:

```text
freelist.head
     |
     v
+---------+      +---------+      +---------+
| next ---+----> | next ---+----> | null    |
| payload |      | payload |      | payload |
+---------+      +---------+      +---------+
```

The bitmap tracks whether each slot is currently allocated:

```text
bit = 0  free
bit = 1  allocated

byte 0
+---+---+---+---+---+---+---+---+
| 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
+---+---+---+---+---+---+---+---+
```

## Platform Memory

`polloc` uses a `BlockSource` trait for platform memory. The default backend is:

- Unix: `mmap` / `munmap`
- Windows: `VirtualAlloc` / `VirtualFree`

The allocator logic does not directly depend on either API.

## Tests

Run the tests:

```bash
cargo test
```

The test suite stresses alignment, block growth, free list reuse, bitmap
boundaries, invalid frees, double frees, typed allocation, and repeated churn.
Some stress inputs are smaller under `cfg(miri)` so Miri runs stay practical.

Run with Miri:

```bash
cargo miri test
```

Run the fuzzer:

```bash
cd fuzz
cargo fuzz run fuzz_target_1
```

## Benchmarks

Run the Criterion benchmark:

```bash
cargo bench
```

Recent 64-byte alloc/free result:

```text
pool alloc/free 64B
time: [4.2272 ns 4.2369 ns 4.2483 ns]

system malloc alloc/free 64B
time: [16.775 ns 16.796 ns 16.821 ns]
```

That makes `polloc` about `3.96x` faster in this benchmark.

This result is expected and should not be read as "`polloc` is better than
malloc" in general. This pool handles one fixed size and one fixed alignment,
so it can do much less work. System malloc is a general-purpose allocator: it
must handle many sizes, many alignments, different lifetimes, threading,
fragmentation, and many platform/runtime details.

The benchmark is still useful because it shows the cost of this pool's fast
path when the workload matches its design.

## TODO

- [ ] Implement `reset` to clear the pool for bulk reuse.
- [ ] Implement memory poisoning to help detect bugs such as double free,
      use-after-free, and uninitialized assumptions during debug/testing.
- [ ] Implement `alloc_one`.
- [ ] Implement `alloc_val`.
- [ ] Add a typed free API that drops `T` before returning the slot.
- [ ] Add more Miri coverage for typed allocation and invalid pointer cases.
