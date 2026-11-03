// SPDX-License-Identifier: BSD-2-Clause
//! The plain-Rust half of the engine library's C boundary (`src/engine/lib.rs`): what keeps a fault
//! in the engine from taking the host down or hanging it. A panic is caught before it reaches
//! `extern "C"` ([`catch`]); one engine at a time, and the slot comes back when the engine goes
//! ([`Slot`]); the host's waker cannot run once the engine is freed ([`Revocable`]); and the
//! engine's shutdown is watched, so a wedged content process is ended instead of holding the
//! host's thread forever ([`bounded`], [`content_children`]). Only std: `tests/engine_boundary.rs`
//! includes this file and tests it in the WolOS workspace, without the engine.
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

/// The argument the engine starts its content process with (`webengine::process::CONTENT_FLAG`).
pub const CONTENT_FLAG: &str = "--content-process";

/// Runs `f`; a panic comes back as its message instead of unwinding (across `extern "C"` it
/// would abort the host, with no error returned).
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| panic_message(p.as_ref()))
}

fn panic_message(p: &(dyn Any + Send)) -> String {
    let what = p
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| p.downcast_ref::<String>().map(String::as_str));
    match what {
        Some(m) => format!("the engine panicked: {m}"),
        None => "the engine panicked".to_owned(),
    }
}

/// The one engine a process may hold at a time.
pub struct Slot(AtomicBool);

/// The slot, held by the engine that took it; dropping it gives the slot back.
pub struct Claim<'a>(&'a AtomicBool);

impl Slot {
    pub const fn new() -> Slot {
        Slot(AtomicBool::new(false))
    }

    /// The slot, or `None` while another engine holds it.
    pub fn claim(&self) -> Option<Claim<'_>> {
        (!self.0.swap(true, Ordering::SeqCst)).then_some(Claim(&self.0))
    }
}

impl Default for Slot {
    fn default() -> Slot {
        Slot::new()
    }
}

// Releases the slot: an engine that failed to build or was freed lets the next one be made.
impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// A value other threads call through until it is revoked: [`Revocable::revoke`] waits for a
/// call in progress and stops every later one. The host's waker goes through one, so the engine's
/// threads cannot wake a context the host has freed after `wwe_engine_free`.
pub struct Revocable<T>(Arc<RwLock<Option<T>>>);

impl<T> Clone for Revocable<T> {
    fn clone(&self) -> Self {
        Revocable(self.0.clone())
    }
}

impl<T: Copy> Revocable<T> {
    pub fn new(v: T) -> Revocable<T> {
        Revocable(Arc::new(RwLock::new(Some(v))))
    }

    /// Calls `f` with the value unless it was revoked; whether it ran.
    pub fn call(&self, f: impl FnOnce(T)) -> bool {
        let held = self.0.read().unwrap_or_else(PoisonError::into_inner);
        match *held {
            Some(v) => {
                f(v);
                true
            }
            None => false,
        }
    }

    /// No call after this returns; one in progress is waited for.
    pub fn revoke(&self) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Runs `work` on this thread. When it has not returned after `limit`, `late` runs once on a
/// watcher thread while `work` goes on (it is what can end what `work` waits for). Returns what
/// `work` returned and whether `late` ran. Without a watcher thread, `work` runs unwatched.
pub fn bounded<T>(
    limit: Duration,
    work: impl FnOnce() -> T,
    late: impl FnOnce() + Send + 'static,
) -> (T, bool) {
    let (done, wait) = mpsc::channel::<()>();
    let fired = Arc::new(AtomicBool::new(false));
    let flag = fired.clone();
    let watcher = std::thread::Builder::new()
        .name("wwe-shutdown".into())
        .spawn(move || {
            if wait.recv_timeout(limit) == Err(RecvTimeoutError::Timeout) {
                flag.store(true, Ordering::SeqCst);
                late();
            }
        });
    let out = work();
    drop(done);
    if let Ok(w) = watcher {
        let _ = w.join();
    }
    (out, fired.load(Ordering::SeqCst))
}

/// The engine's content processes among `pid`'s children: the children listed in
/// `<proc>/<pid>/task/*/children` whose command line is `<exe> --content-process TOKEN`.
pub fn content_children(proc: &Path, pid: u32) -> Vec<u32> {
    let Ok(tasks) = std::fs::read_dir(proc.join(pid.to_string()).join("task")) else {
        return Vec::new();
    };
    let mut kids: Vec<u32> = tasks
        .flatten()
        .filter_map(|t| std::fs::read_to_string(t.path().join("children")).ok())
        .flat_map(|s| {
            s.split_ascii_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect::<Vec<u32>>()
        })
        .filter(|kid| {
            std::fs::read(proc.join(kid.to_string()).join("cmdline"))
                .is_ok_and(|c| c.split(|b| *b == 0).nth(1) == Some(CONTENT_FLAG.as_bytes()))
        })
        .collect();
    kids.sort_unstable();
    kids.dedup();
    kids
}
