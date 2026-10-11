# Polloc

`polloc` is a high-performance fixed size pool allocator written in Rust.

It allocates memory in slots of one size and one alignment per pool. Freed slots
are reused through an intrusive free list, and each block keeps a bitmap so the
allocator can detect invalid states such as double frees.

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

Fallible APIs exist for every panicking one (`try_new`, `try_alloc`,
`try_free`), so out of memory and bad pointer cases are recoverable without
unwinding:

```rust
use polloc::{AllocErr, Pool};

let mut pool = Pool::try_new(64, 8).unwrap();
let ptr = pool.try_alloc()?;
assert!(pool.contains(ptr));
pool.try_free(ptr)?;
# Ok::<(), AllocErr>(())
```

## Safe Typed API

Raw pointers cannot use after free by accident when you use the owning
`Slot<T>` handle instead. It dereferences like `Box<T>` and automatically
drops the value and frees the slot when it goes out of scope:

```rust
use polloc::Pool;
use std::mem::{align_of, size_of};

let mut pool = Pool::new(size_of::<String>(), align_of::<String>());

{
    let mut hello = pool.alloc_value(String::from("hello"));
    hello.push_str(" world");
    assert_eq!(&*hello, "hello world");
} // `String` is dropped and its slot is freed here.

assert!(pool.is_empty());
```

Two phase initialization is explicit via `MaybeUninit`:

```rust
use polloc::Pool;
use std::mem::MaybeUninit;

let mut pool = Pool::new(size_of::<u64>(), 8);
let slot = pool.alloc_uninit::<u64>().write(7);
assert_eq!(*slot, 7);
```

Escaping the handle is possible with `Slot::into_raw`, in which case the
slot must be returned with `Pool::free_value` (an `unsafe` function, since
only the caller knows the slot still holds that `T`).

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

## Pool Lifecycle

```rust
use polloc::Pool;

let mut pool = Pool::new(32, 8);

// Map capacity up front to avoid growth during a hot loop.
pool.reserve(10_000).unwrap();

// ... allocate and free ...

// Arena style bulk reuse: forget all live slots at once, keep the mappings.
pool.reset();

// Give fully free blocks back to the OS (the active block is always kept).
pool.shrink_to_fit();

// Introspection for capacity planning and tests.
assert!(pool.is_empty());
let _ = (pool.live_slots(), pool.capacity_slots(), pool.num_blocks(), pool.memory_bytes());
```

## Platform Memory

`polloc` uses a `BlockSource` trait for platform memory. The default backend is:

- Unix: `mmap` / `munmap`
- Windows: `VirtualAlloc` / `VirtualFree`

The allocator logic does not directly depend on either API. Custom sources
work through `Pool::with_source` / `Pool::try_with_source`, but note the
contract: fresh mappings are bitmap zeroed by the pool, alignments wider than
the source page size are rejected, and the page size must be a power of two.

## Threading

`Pool` (and `Slot`) are single threaded by design: both are `!Send` and
`!Sync`. The compiler rejects sharing them across threads. If you need
threads, put the pool behind your own lock, e.g. `Mutex<Pool>`.

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
pool alloc/free 64B hot
time: [5.2196 ns 5.6229 ns 6.4494 ns]

system malloc alloc/free 64B
time: [19.730 ns 19.961 ns 20.219 ns]
```

That makes `polloc` about `3.5x` faster in this benchmark.

This result is expected and should not be read as "`polloc` is better than
malloc" in general. This pool handles one fixed size and one fixed alignment,
so it can do much less work. System malloc is a general-purpose allocator: it
must handle many sizes, many alignments, different lifetimes, threading,
fragmentation, and many platform/runtime details.

The benchmark is still useful because it shows the cost of this pool's fast
path when the workload matches its design.

## Production Notes

- Pointer validation is a runtime guarantee, not `debug_assert` only: freeing
  an interior pointer returns `MisalignedFree` in release builds too, instead
  of corrupting the bitmap.
- All allocator invariants are behind private fields; public access goes
  through `slot_size()`, `live_slots()`, `capacity_slots()`, `num_blocks()`,
  `memory_bytes()`, and `contains()`.
- `Pool::try_new` validates alignment up front (power of two, at most the
  page size) and reports size overflow as `AllocErr` instead of panicking.
- Huge slots (larger than the default first block) grow the block to fit
  instead of panicking in the slow path.
- The fast path never re queries the OS for the page size and never touches
  the sentinel block; block geometry (`first`, `capacity`) is cached in the
  header.

## TODO

- [x] Implement `reset` to clear the pool for bulk reuse.
- [x] Implement `alloc_uninit` / `alloc_value` (`alloc_one` / `alloc_val`).
- [x] Add a typed free API that drops `T` before returning the slot.
- [ ] Implement memory poisoning to help detect bugs such as double free,
      use after free, and uninitialized assumptions during debug/testing.
- [ ] Add more Miri coverage for typed allocation and invalid pointer cases.
- [ ] Per block freelists so freeing never walks the block chain.
- [ ] `no_std` support (remove the `Rc` marker dependency).
