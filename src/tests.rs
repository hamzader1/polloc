use super::Pool;
use crate::AllocErr;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

fn assert_panics<F: FnOnce()>(f: F) {
    assert!(catch_unwind(AssertUnwindSafe(f)).is_err());
}

fn n(normal: usize, miri: usize) -> usize {
    if cfg!(miri) { miri } else { normal }
}

fn fill(ptr: *mut u8, len: usize, seed: u8) {
    unsafe {
        for i in 0..len {
            ptr.add(i).write(seed.wrapping_add(i as u8));
        }
    }
}

fn check(ptr: *mut u8, len: usize, seed: u8) {
    unsafe {
        for i in 0..len {
            assert_eq!(ptr.add(i).read(), seed.wrapping_add(i as u8));
        }
    }
}

#[test]
fn first_allocation_is_non_null_aligned_and_writable() {
    let mut pool = Pool::new(24, 16);
    let ptr = pool.alloc();
    assert!(!ptr.is_null());
    assert_eq!(ptr as usize % 16, 0);
    fill(ptr, 24, 7);
    check(ptr, 24, 7);
    pool.free(ptr);
}

#[test]
fn allocations_are_unique_until_freed() {
    let mut pool = Pool::new(32, 16);
    let mut seen = HashSet::new();
    let mut ptrs = Vec::new();
    for i in 0..n(4096, 128) {
        let ptr = pool.alloc();
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 16, 0);
        assert!(
            seen.insert(ptr as usize),
            "duplicate pointer at allocation {i}"
        );
        fill(ptr, 32, i as u8);
        ptrs.push(ptr);
    }
    for i in 0..ptrs.len() {
        check(ptrs[i], 32, i as u8);
    }
    for ptr in ptrs.into_iter().rev() {
        pool.free(ptr);
    }
}

#[test]
fn freed_slots_are_reused_in_lifo_order() {
    let mut pool = Pool::new(64, 16);
    let a = pool.alloc();
    let b = pool.alloc();
    let c = pool.alloc();
    pool.free(a);
    pool.free(b);
    pool.free(c);
    assert_eq!(pool.alloc(), c);
    assert_eq!(pool.alloc(), b);
    assert_eq!(pool.alloc(), a);
}

#[test]
fn double_free_panics() {
    let mut pool = Pool::new(8, 8);
    let ptr = pool.alloc();
    pool.free(ptr);
    assert_panics(|| pool.free(ptr));
}

#[test]
fn null_free_panics() {
    let mut pool = Pool::new(8, 8);
    assert_panics(|| pool.free(ptr::null_mut()));
}

#[test]
fn foreign_pointer_free_panics() {
    let mut pool = Pool::new(8, 8);
    let mut byte = 0u8;
    let ptr = &mut byte as *mut u8;
    assert_panics(|| pool.free(ptr));
}

#[test]
fn interior_pointer_free_panics() {
    let mut pool = Pool::new(64, 16);
    let ptr = pool.alloc();
    assert_panics(|| unsafe { pool.free(ptr.add(1)) });
    pool.free(ptr);
}

#[test]
fn supports_many_sizes_and_alignments() {
    let aligns: &[usize] = if cfg!(miri) {
        &[1, 8, 32, 128]
    } else {
        &[1, 2, 4, 8, 16, 32, 64, 128]
    };
    let sizes: &[usize] = if cfg!(miri) {
        &[1, 8, 17, 64, 255]
    } else {
        &[
            1usize, 2, 3, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 255,
        ]
    };
    for &align in aligns {
        for &size in sizes {
            let mut pool = Pool::new(size, align);
            let mut ptrs = Vec::new();
            for i in 0..n(128, 16) {
                let ptr = pool.alloc();
                assert!(!ptr.is_null());
                assert_eq!(ptr as usize % align, 0);
                fill(ptr, size, i as u8);
                ptrs.push((ptr, i as u8));
            }
            for &(ptr, seed) in &ptrs {
                check(ptr, size, seed);
            }
            for (ptr, _) in ptrs {
                pool.free(ptr);
            }
        }
    }
}

#[test]
fn allocates_across_many_blocks_and_frees_everything() {
    let mut pool = Pool::new(256, 64);
    let mut ptrs = Vec::new();
    let mut blocks = HashSet::new();
    for i in 0..n(2048, 64) {
        let ptr = pool.alloc();
        assert_eq!(ptr as usize % 64, 0);
        blocks.insert(unsafe { (*pool.active_block).base as usize });
        fill(ptr, 256, i as u8);
        ptrs.push((ptr, i as u8));
    }
    assert!(blocks.len() > 1);
    for &(ptr, seed) in &ptrs {
        check(ptr, 256, seed);
    }
    for (ptr, _) in ptrs {
        pool.free(ptr);
    }
}

#[test]
fn reuses_slots_from_older_blocks_after_growth() {
    let mut pool = Pool::new(128, 32);
    let first = pool.alloc();
    let first_block = unsafe { (*pool.active_block).base };
    let mut ptrs = vec![first];
    while unsafe { (*pool.active_block).base } == first_block {
        ptrs.push(pool.alloc());
    }
    let second_block_ptr = ptrs.pop().unwrap();
    pool.free(first);
    let reused = pool.alloc();
    assert_eq!(reused, first);
    pool.free(reused);
    pool.free(second_block_ptr);
    for ptr in ptrs.into_iter().skip(1) {
        pool.free(ptr);
    }
}

#[test]
fn repeated_full_pool_churn_keeps_returning_valid_slots() {
    let mut pool = Pool::new(48, 16);
    let mut ptrs = Vec::new();
    for round in 0..n(64, 8) {
        ptrs.clear();
        for i in 0..n(512, 64) {
            let ptr = pool.alloc();
            fill(ptr, 48, (round ^ i) as u8);
            ptrs.push((ptr, (round ^ i) as u8));
        }
        for &(ptr, seed) in &ptrs {
            check(ptr, 48, seed);
        }
        if round % 2 == 0 {
            for &(ptr, _) in ptrs.iter().rev() {
                pool.free(ptr);
            }
        } else {
            for &(ptr, _) in &ptrs {
                pool.free(ptr);
            }
        }
    }
}

#[test]
fn deterministic_randomized_stress() {
    let mut pool = Pool::new(40, 8);
    let mut live: Vec<(*mut u8, u8)> = Vec::new();
    let mut state = 0x1234_5678_9abc_def0u64;
    for step in 0..n(20000, 512) {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let allocate = live.is_empty() || ((state >> 63) == 1 && live.len() < n(4096, 128));
        if allocate {
            let ptr = pool.alloc();
            assert_eq!(ptr as usize % 8, 0);
            let seed = (step as u8).wrapping_mul(17).wrapping_add(live.len() as u8);
            fill(ptr, 40, seed);
            live.push((ptr, seed));
        } else {
            let idx = (state as usize) % live.len();
            let (ptr, seed) = live.swap_remove(idx);
            check(ptr, 40, seed);
            pool.free(ptr);
        }
        if step % 257 == 0 {
            for &(ptr, seed) in &live {
                check(ptr, 40, seed);
            }
        }
    }
    for (ptr, seed) in live {
        check(ptr, 40, seed);
        pool.free(ptr);
    }
}

#[test]
fn zero_sized_requests_still_create_distinct_pointer_sized_slots() {
    let mut pool = Pool::new(0, 8);
    assert!(pool.slot_size >= size_of::<*mut u8>());
    assert_eq!(pool.slot_size % 8, 0);
    let mut ptrs = Vec::new();
    let mut seen = HashSet::new();
    for _ in 0..n(1024, 64) {
        let ptr = pool.alloc();
        assert!(!ptr.is_null());
        assert_eq!(ptr as usize % 8, 0);
        assert!(seen.insert(ptr as usize));
        unsafe {
            ptr.write(0xaa);
        }
        ptrs.push(ptr);
    }
    for ptr in ptrs {
        pool.free(ptr);
    }
}

#[test]
fn slot_size_rounds_up_to_requested_alignment() {
    for (size, align, expected) in [
        (1usize, 1usize, size_of::<*mut u8>()),
        (1, 2, size_of::<*mut u8>()),
        (1, 4, size_of::<*mut u8>()),
        (1, 8, size_of::<*mut u8>()),
        (9, 8, 16),
        (17, 16, 32),
        (33, 32, 64),
        (65, 64, 128),
    ] {
        let pool = Pool::new(size, align);
        assert_eq!(pool.slot_size, expected);
        assert_eq!(pool.slot_size % align, 0);
    }
}

#[test]
fn contiguous_hwm_allocations_advance_by_slot_size() {
    let mut pool = Pool::new(13, 8);
    let first = pool.alloc();
    let second = pool.alloc();
    let third = pool.alloc();
    assert_eq!(second as usize - first as usize, pool.slot_size);
    assert_eq!(third as usize - second as usize, pool.slot_size);
    pool.free(third);
    pool.free(second);
    pool.free(first);
}

#[test]
fn bitmap_boundary_double_free_detection() {
    let mut pool = Pool::new(32, 8);
    let mut ptrs = Vec::new();
    for _ in 0..24 {
        ptrs.push(pool.alloc());
    }
    for &idx in &[0usize, 7, 8, 15, 16, 23] {
        pool.free(ptrs[idx]);
        assert_panics(|| pool.free(ptrs[idx]));
    }
    for (idx, ptr) in ptrs.into_iter().enumerate() {
        if ![0usize, 7, 8, 15, 16, 23].contains(&idx) {
            pool.free(ptr);
        }
    }
}

#[test]
fn freeing_some_slots_does_not_corrupt_live_neighbors() {
    let mut pool = Pool::new(96, 32);
    let mut ptrs = Vec::new();
    for i in 0..n(256, 48) {
        let ptr = pool.alloc();
        fill(ptr, 96, i as u8);
        ptrs.push((ptr, i as u8));
    }
    for i in (0..ptrs.len()).step_by(3) {
        pool.free(ptrs[i].0);
    }
    for i in 0..ptrs.len() {
        if i % 3 != 0 {
            check(ptrs[i].0, 96, ptrs[i].1);
        }
    }
    let mut replacements = Vec::new();
    for i in (0..ptrs.len()).step_by(3) {
        let ptr = pool.alloc();
        fill(ptr, 96, i as u8 ^ 0x5a);
        replacements.push((ptr, i as u8 ^ 0x5a));
    }
    for i in 0..ptrs.len() {
        if i % 3 != 0 {
            check(ptrs[i].0, 96, ptrs[i].1);
            pool.free(ptrs[i].0);
        }
    }
    for (ptr, seed) in replacements {
        check(ptr, 96, seed);
        pool.free(ptr);
    }
}

#[test]
fn old_block_reused_slots_can_be_freed_again_after_growth() {
    let mut pool = Pool::new(128, 32);
    let first = pool.alloc();
    let first_block = unsafe { (*pool.active_block).base };
    let mut ptrs = vec![first];
    while unsafe { (*pool.active_block).base } == first_block {
        ptrs.push(pool.alloc());
    }
    let new_block_ptr = ptrs.pop().unwrap();
    pool.free(first);
    let reused = pool.alloc();
    assert_eq!(reused, first);
    pool.free(reused);
    assert_panics(|| pool.free(reused));
    pool.free(new_block_ptr);
    for ptr in ptrs.into_iter().skip(1) {
        pool.free(ptr);
    }
}

#[test]
fn many_pools_can_allocate_and_drop_independently() {
    for pool_idx in 0..n(128, 8) {
        let mut pool = Pool::new(24 + pool_idx % 17, 8);
        let mut ptrs = Vec::new();
        for i in 0..n(128, 16) {
            let ptr = pool.alloc();
            fill(ptr, 24 + pool_idx % 17, (pool_idx ^ i) as u8);
            ptrs.push((ptr, (pool_idx ^ i) as u8));
        }
        for &(ptr, seed) in &ptrs {
            check(ptr, 24 + pool_idx % 17, seed);
        }
        for (ptr, _) in ptrs {
            pool.free(ptr);
        }
    }
}

#[test]
fn alternating_lifetimes_stress_freelist_and_hwm() {
    let mut pool = Pool::new(72, 8);
    let mut retained = Vec::new();
    for wave in 0..n(256, 16) {
        let mut temporary = Vec::new();
        for i in 0..n(64, 16) {
            let ptr = pool.alloc();
            fill(ptr, 72, (wave + i) as u8);
            if i % 4 == 0 {
                retained.push((ptr, (wave + i) as u8));
            } else {
                temporary.push((ptr, (wave + i) as u8));
            }
        }
        for &(ptr, seed) in &temporary {
            check(ptr, 72, seed);
        }
        for (ptr, _) in temporary {
            pool.free(ptr);
        }
        if retained.len() > n(512, 64) {
            let drained: Vec<_> = retained.drain(0..n(128, 16)).collect();
            for (ptr, seed) in drained {
                check(ptr, 72, seed);
                pool.free(ptr);
            }
        }
    }
    for (ptr, seed) in retained {
        check(ptr, 72, seed);
        pool.free(ptr);
    }
}

#[test]
fn large_slot_allocations_cross_blocks_cleanly() {
    let mut pool = Pool::new(4096, 256);
    let mut ptrs = Vec::new();
    let mut blocks = HashSet::new();
    for i in 0..n(128, 8) {
        let ptr = pool.alloc();
        assert_eq!(ptr as usize % 256, 0);
        blocks.insert(unsafe { (*pool.active_block).base as usize });
        fill(ptr, 4096, i as u8);
        ptrs.push((ptr, i as u8));
    }
    assert!(blocks.len() > 1);
    for &(ptr, seed) in &ptrs {
        check(ptr, 4096, seed);
    }
    for (ptr, _) in ptrs.into_iter().rev() {
        pool.free(ptr);
    }
}

#[test]
fn free_in_permutation_then_reallocate_all_same_addresses() {
    let mut pool = Pool::new(56, 8);
    let mut ptrs = Vec::new();
    let mut original = HashSet::new();
    for i in 0..n(1024, 96) {
        let ptr = pool.alloc();
        fill(ptr, 56, i as u8);
        original.insert(ptr as usize);
        ptrs.push(ptr);
    }
    let mut state = 0xfedc_ba98_7654_3210u64;
    while !ptrs.is_empty() {
        state = state
            .wrapping_mul(2862933555777941757)
            .wrapping_add(3037000493);
        let idx = state as usize % ptrs.len();
        pool.free(ptrs.swap_remove(idx));
    }
    let mut reused = HashSet::new();
    for _ in 0..n(1024, 96) {
        let ptr = pool.alloc();
        assert!(original.contains(&(ptr as usize)));
        reused.insert(ptr as usize);
    }
    assert_eq!(reused.len(), original.len());
}

#[test]
fn foreign_pointer_from_different_pool_panics() {
    // Ensure a pointer from another pool is rejected, even if their address
    // ranges happen to be close together.
    let mut pool_a = Pool::new(64, 16);
    let mut pool_b = Pool::new(64, 16);
    let ptr_from_b = pool_b.alloc();
    assert_panics(|| pool_a.free(ptr_from_b));
    pool_b.free(ptr_from_b);
}

#[test]
fn free_pointer_just_before_slots_start_panics() {
    // The block base is inside the mapping but does not point to a slot.
    let mut pool = Pool::new(64, 16);
    let ptr = pool.alloc();
    let block_base = unsafe { (*pool.active_block).base };
    if block_base != ptr {
        assert_panics(|| pool.free(block_base));
    }
    pool.free(ptr);
}

#[test]
fn free_pointer_at_exact_block_end_panics() {
    // The address at the end of the mapping is outside the block.
    let mut pool = Pool::new(64, 16);
    let ptr = pool.alloc();
    let block_end = unsafe { (*pool.active_block).base.add((*pool.active_block).size) };
    assert_panics(|| pool.free(block_end));
    pool.free(ptr);
}

#[test]
fn alloc_free_alloc_same_slot_immediately_reuses_correctly() {
    // Repeatedly reuse one slot to exercise the bitmap and free list.
    let mut pool = Pool::new(32, 8);
    let first = pool.alloc();
    for i in 0..n(10000, 200) {
        pool.free(first);
        let again = pool.alloc();
        assert_eq!(again, first);
        fill(again, 32, i as u8);
        check(again, 32, i as u8);
    }
    pool.free(first);
}

#[test]
fn bitmap_survives_exact_multiple_of_8_slot_counts() {
    // Check counts around bitmap byte boundaries, including full bytes.
    for &count in &[7usize, 8, 9, 15, 16, 17, 63, 64, 65] {
        let mut pool = Pool::new(16, 8);
        let mut ptrs = Vec::new();
        for i in 0..count {
            let ptr = pool.alloc();
            fill(ptr, 16, i as u8);
            ptrs.push(ptr);
        }
        for (i, &ptr) in ptrs.iter().enumerate() {
            check(ptr, 16, i as u8);
        }
        for ptr in ptrs {
            pool.free(ptr);
        }
    }
}

#[test]
fn max_alignment_slot_size_relationship_holds_under_growth() {
    // Check slot alignment after the pool grows beyond its first block.
    let mut pool = Pool::new(100, 128);
    let mut ptrs = Vec::new();
    for i in 0..n(200, 24) {
        let ptr = pool.alloc();
        assert_eq!(ptr as usize % 128, 0);
        fill(ptr, 100, i as u8);
        ptrs.push((ptr, i as u8));
    }
    for &(ptr, seed) in &ptrs {
        check(ptr, 100, seed);
    }
    for (ptr, _) in ptrs {
        pool.free(ptr);
    }
}

#[test]
fn interleaved_alloc_free_across_block_boundary() {
    // Keep live and freed slots in the old block while a new block is added.
    let mut pool = Pool::new(64, 16);
    let mut live = Vec::new();
    for i in 0..n(64, 16) {
        let ptr = pool.alloc();
        fill(ptr, 64, i as u8);
        if i % 2 == 0 {
            pool.free(ptr);
        } else {
            live.push((ptr, i as u8));
        }
    }
    for &(ptr, seed) in &live {
        check(ptr, 64, seed);
    }
    for (ptr, _) in live {
        pool.free(ptr);
    }
}

#[test]
fn drop_with_live_allocations_does_not_leak_or_crash() {
    // Dropping the pool releases its blocks, even when slots remain live.
    let mut pool = Pool::new(48, 16);
    for i in 0..n(512, 64) {
        let ptr = pool.alloc();
        fill(ptr, 48, i as u8);
    }
    drop(pool);
}

#[test]
fn drop_stress_many_pools_created_and_dropped() {
    // Repeatedly create and drop pools to catch resource leaks.
    for _ in 0..n(2000, 100) {
        let mut pool = Pool::new(64, 16);
        for _ in 0..8 {
            pool.alloc();
        }
        drop(pool);
    }
}

// Tests for the 0.2 API.

#[test]
fn try_new_rejects_bad_alignment() {
    use std::mem::{align_of, size_of};
    // Zero and alignments that are not powers of two are invalid.
    for bad in [0usize, 3, 6, 12, 24, 100] {
        assert!(matches!(
            Pool::try_new(8, bad),
            Err(AllocErr::InvalidAlignment)
        ));
    }
    // Alignment wider than a page cannot be promised by page mappings.
    let probe = Pool::new(8, 8);
    let page = probe.page_size();
    assert!(matches!(
        Pool::try_new(8, page * 2),
        Err(AllocErr::InvalidAlignment)
    ));
    // Sane inputs still construct, with accessors agreeing with layout.
    let pool = Pool::try_new(size_of::<u64>(), align_of::<u64>()).unwrap();
    assert!(pool.slot_size() >= size_of::<u64>());
    assert_eq!(pool.slot_size() % pool.slot_align(), 0);
    assert_eq!(pool.page_size(), page);
    assert!(pool.is_empty());
}

#[test]
fn huge_slots_grow_the_first_block_instead_of_panicking() {
    // Regression test: this used to panic on `Option::unwrap()` in the slow
    // path because one 1 MiB slot never fit the default first block.
    let mut pool = Pool::new(1 << 20, 8);
    assert_eq!(pool.num_blocks(), 0);
    let ptr = pool.try_alloc().expect("huge slot must map a fitting block");
    assert!(!ptr.is_null());
    assert_eq!(ptr as usize % 8, 0);
    assert_eq!(pool.num_blocks(), 1);
    assert!(pool.capacity_slots() >= 1);
    fill(ptr, 1024, 0xab);
    check(ptr, 1024, 0xab);
    pool.free(ptr);
    assert!(pool.is_empty());
}

#[test]
fn fallible_free_reports_error_kinds_without_panicking() {
    let mut pool = Pool::new(64, 16);
    let ptr = pool.alloc();
    // Interior pointer: slot region found, boundary check fails.
    assert!(matches!(
        unsafe { pool.try_free(ptr.add(1)) },
        Err(AllocErr::MisalignedFree)
    ));
    // Null and foreign pointers never enter any block's slot region.
    assert!(matches!(
        pool.try_free(ptr::null_mut()),
        Err(AllocErr::InvalidPointer)
    ));
    let mut byte = 0u8;
    assert!(matches!(
        pool.try_free(&mut byte as *mut u8),
        Err(AllocErr::InvalidPointer)
    ));
    // Double free is still caught by the bitmap.
    pool.free(ptr);
    assert!(matches!(pool.try_free(ptr), Err(AllocErr::DoubleFree)));
}

#[test]
fn contains_tracks_liveness() {
    let mut pool = Pool::new(32, 8);
    let ptr = pool.alloc();
    assert!(pool.contains(ptr));
    assert!(!pool.contains(unsafe { ptr.add(1) }));
    assert!(!pool.contains(ptr::null_mut()));
    pool.free(ptr);
    assert!(!pool.contains(ptr));
}

#[test]
fn stats_count_live_slots_blocks_and_bytes() {
    let mut pool = Pool::new(32, 8);
    assert_eq!((pool.live_slots(), pool.num_blocks(), pool.memory_bytes()), (0, 0, 0));
    let a = pool.alloc();
    let b = pool.alloc();
    assert_eq!(pool.live_slots(), 2);
    assert!(pool.num_blocks() >= 1);
    assert!(pool.memory_bytes() >= pool.page_size());
    assert!(pool.capacity_slots() >= 2);
    pool.free(a);
    assert_eq!(pool.live_slots(), 1);
    pool.free(b);
    assert!(pool.is_empty());
}

#[test]
fn reset_reuses_warm_blocks_for_bulk_reuse() {
    let mut pool = Pool::new(48, 8);
    let mut first_gen = Vec::new();
    for _ in 0..n(256, 32) {
        first_gen.push(pool.alloc());
    }
    let blocks_before = pool.num_blocks();
    assert!(blocks_before >= 1);
    pool.reset();
    assert!(pool.is_empty());
    assert_eq!(pool.live_slots(), 0);
    // No new mapping should be needed to hand out the same amount again.
    let mut second_gen = Vec::new();
    for _ in 0..n(256, 32) {
        second_gen.push(pool.alloc());
    }
    assert_eq!(pool.num_blocks(), blocks_before);
    for (i, &ptr) in second_gen.iter().enumerate() {
        fill(ptr, 48, i as u8);
    }
    for (i, &ptr) in second_gen.iter().enumerate() {
        check(ptr, 48, i as u8);
        pool.free(ptr);
    }
    for ptr in first_gen {
        // Stale pointers from before the reset are dead: the bitmap was
        // cleared, so freeing one now looks like a double free, never a
        // silent corruption.
        assert!(matches!(pool.try_free(ptr), Err(AllocErr::DoubleFree)));
    }
}

#[test]
fn reserve_maps_capacity_up_front() {
    let mut pool = Pool::new(32, 8);
    pool.reserve(1000).unwrap();
    assert!(pool.capacity_slots() >= 1000);
    assert_eq!(pool.live_slots(), 0);
    let blocks = pool.num_blocks();
    let mut ptrs = Vec::new();
    for _ in 0..1000 {
        ptrs.push(pool.alloc());
    }
    // Everything fit in the reserved blocks: no growth while filling.
    assert_eq!(pool.num_blocks(), blocks);
    for ptr in ptrs {
        pool.free(ptr);
    }
}

#[test]
fn shrink_to_fit_releases_fully_free_blocks() {
    let mut pool = Pool::new(64, 8);
    let mut ptrs = Vec::new();
    for _ in 0..n(2000, 200) {
        ptrs.push(pool.alloc());
    }
    assert!(pool.num_blocks() > 1);
    for ptr in ptrs {
        pool.free(ptr);
    }
    pool.shrink_to_fit();
    // Only the active block is kept; the rest go back to the OS.
    assert_eq!(pool.num_blocks(), 1);
    // The pool is fully usable afterwards (freelist was pruned, not trashed).
    let mut again = Vec::new();
    for i in 0..n(512, 64) {
        let ptr = pool.alloc();
        fill(ptr, 64, i as u8);
        again.push((ptr, i as u8));
    }
    for (ptr, seed) in again {
        check(ptr, 64, seed);
        pool.free(ptr);
    }
}

#[test]
fn slot_value_deref_mut_and_auto_free() {
    let mut pool = Pool::new(size_of::<u64>(), align_of::<u64>());
    let raw = {
        let mut slot = pool.alloc_value(123u64);
        assert_eq!(*slot, 123);
        *slot = 456;
        assert_eq!(*slot, 456);
        slot.as_ptr() as *mut u8
        // Slot borrows the pool exclusively, so no pool call is possible
        // while it is alive; the borrow checker enforces single ownership.
    };
    // Dropping the Slot freed it: no leak, no manual free needed.
    assert!(pool.is_empty());
    assert!(!pool.contains(raw));
}

#[test]
fn slot_uninit_write_initializes_without_pool_roundtrip() {
    use std::mem::MaybeUninit;
    let mut pool = Pool::new(size_of::<String>(), align_of::<String>());
    assert!(pool.is_empty());
    {
        let uninit: crate::Slot<'_, MaybeUninit<String>, _> = pool.alloc_uninit();
        let mut slot = uninit.write(String::from("hello"));
        assert_eq!(slot.as_str(), "hello");
        slot.push_str(" world");
        assert_eq!(&**slot, "hello world");
    }
    assert!(pool.is_empty());
    // Allocating, initializing, and dropping used exactly one block and one
    // slot: `write` performs no extra pool round trip.
    assert_eq!(pool.num_blocks(), 1);
    assert!(pool.capacity_slots() >= 1);
}

#[test]
fn slot_drop_runs_destructor_and_free_value_matches() {
    use std::cell::Cell;
    use std::rc::Rc;
    struct Droppable {
        drops: Rc<Cell<u32>>,
    }
    impl Drop for Droppable {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }
    let drops = Rc::new(Cell::new(0u32));
    let mut pool = Pool::new(size_of::<Droppable>(), align_of::<Droppable>());
    {
        let _slot = pool.alloc_value(Droppable { drops: drops.clone() });
        assert_eq!(drops.get(), 0);
    }
    assert_eq!(drops.get(), 1);
    assert!(pool.is_empty());

    // into_raw + free_value path drops exactly once as well.
    let raw = pool.alloc_value(Droppable { drops: drops.clone() }).into_raw();
    assert_eq!(drops.get(), 1);
    unsafe {
        pool.free_value(raw);
    }
    assert_eq!(drops.get(), 2);
    assert!(pool.is_empty());
}

#[test]
fn typed_allocation_rejects_oversized_types() {
    let mut pool = Pool::new(8, 8);
    assert!(matches!(
        pool.try_alloc_value([0u64; 16]),
        Err(AllocErr::InvalidSizeOrAlignment)
    ));
    #[repr(align(64))]
    struct OverAligned;
    assert!(matches!(
        pool.try_alloc_uninit::<OverAligned>(),
        Err(AllocErr::InvalidSizeOrAlignment)
    ));
}

#[test]
fn fast_path_reuses_freed_slots_without_new_mappings() {
    // The freelist path must not map: churn within one block keeps the
    // block count (and therefore mmap traffic) flat.
    let mut pool = Pool::new(32, 8);
    pool.reserve(64).unwrap();
    let blocks = pool.num_blocks();
    for _ in 0..1000 {
        let ptr = pool.try_alloc().unwrap();
        pool.free(ptr);
    }
    assert_eq!(pool.num_blocks(), blocks);
    assert!(pool.is_empty());
}
