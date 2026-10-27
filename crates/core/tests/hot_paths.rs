//! Perf invariants (IaraOS StatsHotPathTests, ProcessSamplerTests): timing + allocation bounds.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use wol_core::system::proc::{Column, FakeProcSource, ProcessSampler};
use wol_core::system::stats::StatsSource;
use wol_core::system::{Host, PosixHost};

struct Counting;
thread_local!(static BYTES: Cell<usize> = const { Cell::new(0) });

// SAFETY: forwards to System; only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = BYTES.try_with(|b| b.set(b.get() + l.size()));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let _ = BYTES.try_with(|b| b.set(b.get() + n));
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn allocated() -> usize {
    BYTES.with(Cell::get)
}

#[test]
fn real_stats_sample_is_fast_small_and_forkless() {
    let h: Arc<dyn Host> = Arc::new(PosixHost);
    let mut src = StatsSource::new(h);
    src.read();
    let t = std::time::Instant::now();
    let before = allocated();
    for _ in 0..100 {
        src.read();
    }
    let per = (allocated() - before) / 100;
    let ms = t.elapsed().as_secs_f64() * 1000.0 / 100.0;
    assert!(ms < 2.0, "{ms:.3} ms per sample");
    assert!(per < 64 * 1024, "{per} bytes per sample");
}

#[test]
fn steady_state_process_sampling_allocates_nothing() {
    let mut s = ProcessSampler::new(FakeProcSource::default());
    let names: Vec<String> = (0..300).map(|i| format!("p{i}")).collect();
    let rows: Vec<_> = (0..300)
        .map(|i| {
            (
                i + 1,
                i64::from(i) * 1000,
                100,
                names[i as usize].as_str(),
                i % 3,
                1,
                3,
            )
        })
        .collect();
    s.source().set(&rows);
    let mut idx = vec![0usize; 300];
    for i in 0..3 {
        s.source().now += 1_000_000;
        s.sample();
        s.sort(&mut idx, Column::Cpu, true);
        let _ = s.user(i);
    }
    let lookups = s.source().user_lookups;
    let before = allocated();
    let t = std::time::Instant::now();
    for i in 0..100 {
        s.source().now += 1_000_000;
        s.sample();
        s.sort(&mut idx, Column::Cpu, true);
        s.sort(&mut idx, Column::Name, false);
        let _ = s.user(i % 3);
    }
    assert_eq!(allocated() - before, 0);
    assert_eq!(s.source().user_lookups, lookups);
    let us = t.elapsed().as_secs_f64() * 1e6 / 100.0;
    let budget = if cfg!(debug_assertions) {
        20_000.0
    } else {
        2_000.0
    };
    assert!(us < budget, "{us:.0} us per 300-row sample");
}
