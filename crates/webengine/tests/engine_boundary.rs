// SPDX-License-Identifier: BSD-2-Clause
//! The engine library's boundary helpers (`src/engine/boundary.rs`), without the engine: a panic
//! comes back as an error, the one-engine slot is given back, a revoked waker never runs (and
//! revoking waits for a wake in progress), a late shutdown runs its fallback while a quick one
//! does not, and the content processes are told from the host's other children.
#[path = "../src/engine/boundary.rs"]
mod boundary;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

#[test]
fn a_panic_comes_back_as_an_error_with_its_message() {
    assert_eq!(boundary::catch(|| 7), Ok(7));
    let e = boundary::catch(|| -> u32 { panic!("content spawn failed") }).unwrap_err();
    assert!(e.contains("content spawn failed"), "{e}");
    let n = 3;
    let e = boundary::catch(|| -> u32 { panic!("{n} threads") }).unwrap_err();
    assert!(e.contains("3 threads"), "{e}");
}

#[test]
fn the_engine_slot_comes_back_when_its_engine_goes() {
    static SLOT: boundary::Slot = boundary::Slot::new();
    let first = SLOT.claim().expect("a free slot");
    assert!(SLOT.claim().is_none(), "one engine at a time");
    drop(first);
    let again = SLOT.claim();
    assert!(
        again.is_some(),
        "a failed or freed engine gives the slot back"
    );
    // A panic while building (the claim is dropped as it unwinds) gives it back too.
    drop(again);
    let r = boundary::catch(|| {
        let _held = SLOT.claim();
        panic!("build failed");
    });
    assert!(r.is_err());
    assert!(SLOT.claim().is_some());
}

#[test]
fn a_revoked_waker_never_runs_and_revoking_waits_for_a_wake_in_progress() {
    let woken = Arc::new(AtomicU32::new(0));
    let w = boundary::Revocable::new(5u32);
    assert!(w.call(|v| {
        woken.fetch_add(v, Ordering::SeqCst);
    }));
    assert_eq!(woken.load(Ordering::SeqCst), 5);

    // A wake on another thread is inside its call when the engine is freed; it finishes only
    // after the free has begun.
    let inside = Arc::new(Barrier::new(2));
    let revoking = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let t = {
        let (w, inside) = (w.clone(), inside.clone());
        let (revoking, finished) = (revoking.clone(), finished.clone());
        std::thread::spawn(move || {
            w.call(|_| {
                inside.wait();
                while !revoking.load(Ordering::SeqCst) {
                    std::thread::yield_now();
                }
                for _ in 0..10_000 {
                    std::thread::yield_now();
                }
                finished.store(true, Ordering::SeqCst);
            })
        })
    };
    inside.wait();
    revoking.store(true, Ordering::SeqCst);
    w.revoke();
    assert!(
        finished.load(Ordering::SeqCst),
        "revoke returned while a wake was still running"
    );
    assert!(t.join().unwrap());
    assert!(!w.call(|_| panic!("woke after the engine was freed")));
    assert!(!w.clone().call(|_| panic!("a copy woke too")));
}

#[test]
fn a_late_shutdown_runs_its_fallback_and_a_quick_one_does_not() {
    let fell_back = Arc::new(AtomicBool::new(false));
    let f = fell_back.clone();
    let (v, late) = boundary::bounded(
        Duration::from_secs(5),
        || 42,
        move || f.store(true, Ordering::SeqCst),
    );
    assert_eq!((v, late), (42, false));
    assert!(!fell_back.load(Ordering::SeqCst));

    // The fallback is what ends the wait (it ends the wedged content process).
    let ended = Arc::new(AtomicBool::new(false));
    let e = ended.clone();
    let t = Instant::now();
    let ((), late) = boundary::bounded(
        Duration::from_millis(50),
        || {
            while !ended.load(Ordering::SeqCst) {
                assert!(t.elapsed() < Duration::from_secs(10), "never ended");
                std::thread::yield_now();
            }
        },
        move || e.store(true, Ordering::SeqCst),
    );
    assert!(late);
    assert!(t.elapsed() >= Duration::from_millis(50));
}

#[test]
fn only_the_hosts_content_processes_are_ended() {
    let proc = std::env::temp_dir().join(format!("wwe-boundary-proc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&proc);
    let file = |rel: &str, body: &[u8]| {
        let p = proc.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    // Host 100: threads 100 and 101 started children 200 (content), 201 (xdg-open), 202 (content).
    file("100/task/100/children", b"200 201 ");
    file("100/task/101/children", b"202 200\n");
    file(
        "200/cmdline",
        b"/usr/bin/wol-webhost\0--content-process\0tok1\0",
    );
    file("201/cmdline", b"xdg-open\0https://example.org/\0");
    file(
        "202/cmdline",
        b"/usr/bin/wol-webhost\0--content-process\0tok2\0",
    );
    // 300 runs the content flag but is not the host's child.
    file(
        "300/cmdline",
        b"/usr/bin/wol-webhost\0--content-process\0tok3\0",
    );
    assert_eq!(boundary::content_children(&proc, 100), [200, 202]);
    assert!(boundary::content_children(&proc, 999).is_empty());
    assert_eq!(boundary::CONTENT_FLAG, webengine::process::CONTENT_FLAG);
    let _ = std::fs::remove_dir_all(&proc);
}
