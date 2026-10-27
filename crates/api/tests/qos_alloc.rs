//! The QoS hot path (parse + switch + apply) allocates nothing in steady state.
use api::qos::{Applier, serve};
use shared::qos::{ForegroundTracker, QosClass};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

struct Nop;
impl Applier for Nop {
    fn apply(&self, _: i32, _: QosClass) -> bool {
        true
    }
}

#[test]
fn qos_serve_allocates_nothing() {
    let mut input = Vec::new();
    for i in 0..5_000 {
        input.extend_from_slice(if i % 2 == 0 {
            b"foreground 11\n"
        } else {
            b"foreground 12\n"
        });
    }
    let t = Mutex::new(ForegroundTracker::default());
    let before = ALLOCS.load(Ordering::Relaxed);
    serve(&input[..], &t, &Nop);
    assert_eq!(ALLOCS.load(Ordering::Relaxed) - before, 0);
}
