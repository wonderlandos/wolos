// SPDX-License-Identifier: GPL-2.0-or-later
//! The QoS hot path (parse + switch + apply) allocates nothing in steady state.
use api::qos::{Applier, Foregrounds, serve};
use shared::qos::QosClass;
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
    // S4-1: the owner check on every pid reads the real /proc without allocating. This test
    // process is one of the pids, so the peer is its own uid; pid 1 is root's and is skipped.
    let me = i32::try_from(std::process::id()).unwrap();
    let uid = shared::sys::euid();
    let owner = |pid: i32| api::memsaver::procs::owner(std::path::Path::new("/proc"), pid);
    assert_eq!(owner(me), Some(uid));
    let mut input = Vec::new();
    for i in 0..5_000 {
        let pid = if i % 2 == 0 { me } else { 1 };
        input.extend_from_slice(format!("foreground {pid}\n").as_bytes());
    }
    let t = Mutex::new(Foregrounds::default());
    let before = ALLOCS.load(Ordering::Relaxed);
    serve(&input[..], uid, &owner, &t, &Nop);
    assert_eq!(ALLOCS.load(Ordering::Relaxed) - before, 0);
}
