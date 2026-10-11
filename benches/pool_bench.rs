use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use polloc::Pool;

/// Fast path benchmark that repeatedly reuses one slot already in cache.
fn bench_hot(c: &mut Criterion) {
    c.bench_function("pool alloc/free 64B hot", |b| {
        let mut pool = Pool::new(64, 8);
        b.iter(|| {
            let ptr = pool.alloc();
            black_box(ptr);
            pool.free(ptr);
        });
    });
}

/// Bulk throughput: fill N slots, then drain them. Measures bump + freelist
/// push/pop at scale rather than a single hot cache line.
fn bench_bulk(c: &mut Criterion) {
    for n in [64usize, 1024, 16384] {
        c.bench_with_input(BenchmarkId::new("pool bulk alloc/free 64B", n), &n, |b, &n| {
            let mut pool = Pool::new(64, 8);
            pool.reserve(n).unwrap();
            b.iter(|| {
                let mut ptrs = Vec::with_capacity(n);
                for _ in 0..n {
                    ptrs.push(pool.alloc());
                }
                black_box(&ptrs);
                for ptr in ptrs {
                    pool.free(ptr);
                }
            });
        });
    }
}

/// Churn with a live working set: half the slots stay alive while the other
/// half turns over, forcing freelist reuse mixed with live neighbors.
fn bench_churn(c: &mut Criterion) {
    c.bench_function("pool churn 64B live-512", |b| {
        let mut pool = Pool::new(64, 8);
        let mut live = Vec::new();
        for _ in 0..512 {
            live.push(pool.alloc());
        }
        b.iter(|| {
            for _ in 0..64 {
                let ptr = pool.alloc();
                black_box(ptr);
                pool.free(live.pop().unwrap());
                live.push(ptr);
            }
        });
    });
}

/// Safe typed API benchmark covering slot creation, access, and destruction.
fn bench_typed(c: &mut Criterion) {
    c.bench_function("pool Slot<u64> alloc/drop", |b| {
        let mut pool = Pool::new(size_of::<u64>(), align_of::<u64>());
        b.iter(|| {
            let slot = pool.alloc_value(black_box(0xdead_beefu64));
            black_box(*slot);
        });
    });
}

/// Cold growth: a fresh pool per iteration, so every sample pays `mmap`.
fn bench_cold_growth(c: &mut Criterion) {
    c.bench_function("pool cold first-alloc (mmap)", |b| {
        b.iter(|| {
            let mut pool = Pool::new(64, 8);
            let ptr = pool.alloc();
            black_box(ptr);
            pool.free(ptr);
        });
    });
}

fn bench_malloc(c: &mut Criterion) {
    c.bench_function("system malloc alloc/free 64B", |b| {
        b.iter(|| {
            let layout = std::alloc::Layout::from_size_align(64, 8).unwrap();
            unsafe {
                let ptr = std::alloc::alloc(layout);
                black_box(ptr);
                std::alloc::dealloc(ptr, layout);
            }
        });
    });
}

criterion_group!(
    benches,
    bench_hot,
    bench_bulk,
    bench_churn,
    bench_typed,
    bench_cold_growth,
    bench_malloc
);
criterion_main!(benches);
