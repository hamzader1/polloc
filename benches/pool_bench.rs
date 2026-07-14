use criterion::{Criterion, black_box, criterion_group, criterion_main};
use polloc::Pool;

fn bench_pool(c: &mut Criterion) {
    c.bench_function("pool alloc/free 64B", |b| {
        let mut pool = Pool::new(64, 8);
        b.iter(|| {
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

criterion_group!(benches, bench_pool, bench_malloc);
criterion_main!(benches);
