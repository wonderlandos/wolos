// SPDX-License-Identifier: GPL-2.0-or-later
//! Memory Saver in wold (ADR 0026): every app gets its own systemd scope; a background app gives
//! its memory back to zram once (memory.reclaim on the scope, process_madvise on each process) and
//! faults it in as it runs. Policy: `wol_core::memsaver`. Session facts: `shared::memsaver`
//! snapshots from wol-realtime.
pub mod cgroup;
pub mod engine;
pub mod pageout;
pub mod procs;
pub mod storm;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Everything the engine does to the system, so its logic runs against a model in tests.
/// Cgroup arguments are cgroup paths (`/user.slice/...`), not filesystem paths.
pub trait Sys {
    fn cgroup(&self, pid: i32) -> Option<String>;
    fn owner_and_parent(&self, pid: i32) -> Option<(u32, i32)>;
    fn comm(&self, pid: i32) -> String;
    fn is_game(&self, pid: i32) -> bool;
    fn parents(&self) -> Vec<(i32, i32)>;
    fn exists(&self, cg: &str) -> bool;
    fn procs(&self, cg: &str) -> Vec<i32>;
    fn current(&self, cg: &str) -> u64;
    fn cpu_usec(&self, cg: &str) -> u64;
    fn freeze(&self, cg: &str, frozen: bool) -> bool;
    /// pids.events `max`.
    fn refused(&self, cg: &str) -> u64;
    /// cgroup.kill.
    fn kill(&self, cg: &str) -> bool;
    /// memory.events `oom_kill`.
    fn oom_kills(&self, cg: &str) -> u64;
    /// pids.current.
    fn tasks(&self, cg: &str) -> u64;
    /// Ask systemd for a scope (busctl argv from [`procs::adopt_args`]).
    fn adopt(&self, args: &[String]) -> bool;
    /// Reclaim `bytes` from the scope and page out `pids`, in the background: returns at once.
    fn give_back(&self, scope: &str, bytes: u64, pids: Vec<i32>);
    /// Page out `pids` (every private mapping, code included: [`pageout::Pageout::All`]) in the
    /// background, without a reclaim: returns at once. The budget's page-out while behind.
    fn page_out(&self, scope: &str, pids: Vec<i32>);
    /// Stop a [`Sys::give_back`] or [`Sys::page_out`] of this scope.
    fn cancel(&self, scope: &str);
    /// Bytes of these processes that are compressed away (swap), wherever they were charged.
    fn swapped(&self, pids: &[i32]) -> u64;
    /// memory.high of the cgroup, in bytes; None: no limit (`max`).
    fn set_high(&self, cg: &str, bytes: Option<u64>) -> bool;
}

/// The real system: /proc, /sys/fs/cgroup and busctl.
pub struct Real {
    pub proc: PathBuf,
    pub root: PathBuf,
    /// Running page-outs by scope: their stop flags. One per scope; a page-out takes its own out
    /// when it ends.
    paging: Arc<Paging>,
}

/// The page-outs running, by scope.
#[derive(Default)]
struct Paging(Mutex<HashMap<String, Arc<AtomicBool>>>);

impl Paging {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A new page-out of `scope`: the stop flag it runs under. One still running for the scope (a
    /// Nap over a Save) is stopped: two would page out the same processes, and the first could no
    /// longer be stopped.
    fn start(&self, scope: &str) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(old) = self.map().insert(scope.to_owned(), stop.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        stop
    }

    /// The page-out under `stop` ended: its entry goes, unless a newer one took the scope.
    fn end(&self, scope: &str, stop: &Arc<AtomicBool>) {
        let mut m = self.map();
        if m.get(scope).is_some_and(|s| Arc::ptr_eq(s, stop)) {
            m.remove(scope);
        }
    }

    /// Stop the page-out of `scope`, if one runs.
    fn cancel(&self, scope: &str) {
        if let Some(stop) = self.map().remove(scope) {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

impl Default for Real {
    fn default() -> Self {
        Real {
            proc: "/proc".into(),
            root: cgroup::ROOT.into(),
            paging: Arc::default(),
        }
    }
}

impl Real {
    fn dir(&self, cg: &str) -> PathBuf {
        self.root.join(cg.trim_start_matches('/'))
    }

    /// The one worker of [`Sys::give_back`] (a reclaim of `reclaim` bytes first, then the
    /// [`pageout::Pageout::Anon`] ranges) and [`Sys::page_out`] (no reclaim, every private range):
    /// a named thread at the lowest priority, under the scope's stop flag, logged once.
    fn spawn_pageout(
        &self,
        scope: &str,
        reclaim: Option<u64>,
        pids: Vec<i32>,
        what: pageout::Pageout,
    ) {
        let stop = self.paging.start(scope);
        let (proc, dir) = (self.proc.clone(), self.dir(scope));
        let (paging, key) = (self.paging.clone(), scope.to_owned());
        let scope = scope.to_owned();
        let work = move || {
            // The compression runs on this thread: it must not compete with what is in use.
            // SAFETY: setpriority on the calling thread (who = 0) with plain integers.
            unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 19) };
            let t = std::time::Instant::now();
            if let Some(bytes) = reclaim {
                reclaim_steps(&dir, bytes, &stop);
            }
            let paged: usize = pids
                .iter()
                .map(|p| pageout::pageout_what(&proc, *p, &stop, what))
                .sum();
            eprintln!(
                "wold: memsaver {} {scope} in {} ms ({} MiB of ranges){}",
                if reclaim.is_some() {
                    "gave back"
                } else {
                    "paged out"
                },
                t.elapsed().as_millis(),
                paged >> 20,
                if stop.load(Ordering::Relaxed) {
                    " (stopped)"
                } else {
                    ""
                }
            );
            paging.end(&scope, &stop);
        };
        if std::thread::Builder::new()
            .name("wol-memsaver-pageout".into())
            .spawn(work)
            .is_err()
        {
            // No thread: nothing is paged out this time, and the scope's entry goes with it.
            self.paging.cancel(&key);
        }
    }
}

impl Sys for Real {
    fn cgroup(&self, pid: i32) -> Option<String> {
        procs::cgroup(&self.proc, pid)
    }
    fn owner_and_parent(&self, pid: i32) -> Option<(u32, i32)> {
        procs::owner_and_parent(&self.proc, pid)
    }
    fn comm(&self, pid: i32) -> String {
        procs::comm(&self.proc, pid)
    }
    fn is_game(&self, pid: i32) -> bool {
        procs::is_game(&self.proc, pid)
    }
    fn parents(&self) -> Vec<(i32, i32)> {
        procs::parents(&self.proc)
    }
    fn exists(&self, cg: &str) -> bool {
        self.dir(cg).is_dir()
    }
    fn procs(&self, cg: &str) -> Vec<i32> {
        cgroup::procs(&self.dir(cg))
    }
    fn current(&self, cg: &str) -> u64 {
        cgroup::current(&self.dir(cg))
    }
    fn cpu_usec(&self, cg: &str) -> u64 {
        cgroup::cpu_usec(&self.dir(cg))
    }
    fn freeze(&self, cg: &str, frozen: bool) -> bool {
        cgroup::freeze(&self.dir(cg), frozen)
    }
    fn refused(&self, cg: &str) -> u64 {
        cgroup::refused(&self.dir(cg))
    }
    fn kill(&self, cg: &str) -> bool {
        cgroup::kill(&self.dir(cg))
    }
    fn oom_kills(&self, cg: &str) -> u64 {
        cgroup::oom_kills(&self.dir(cg))
    }
    fn tasks(&self, cg: &str) -> u64 {
        cgroup::tasks(&self.dir(cg))
    }
    fn adopt(&self, args: &[String]) -> bool {
        crate::privileged::privileged_command("busctl")
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    fn give_back(&self, scope: &str, bytes: u64, pids: Vec<i32>) {
        self.spawn_pageout(scope, Some(bytes), pids, pageout::Pageout::Anon);
    }
    fn page_out(&self, scope: &str, pids: Vec<i32>) {
        self.spawn_pageout(scope, None, pids, pageout::Pageout::All);
    }
    fn cancel(&self, scope: &str) {
        self.paging.cancel(scope);
    }
    fn swapped(&self, pids: &[i32]) -> u64 {
        pids.iter().map(|p| pageout::swapped(&self.proc, *p)).sum()
    }
    fn set_high(&self, cg: &str, bytes: Option<u64>) -> bool {
        cgroup::set_high(&self.dir(cg), bytes)
    }
}

/// Reclaim in steps so `stop` is seen quickly; a step that barely helps means only hot or shared
/// pages are left.
fn reclaim_steps(dir: &Path, bytes: u64, stop: &AtomicBool) {
    const STEP: u64 = 64 << 20;
    let mut left = bytes;
    while left > 0 && !stop.load(Ordering::Relaxed) {
        let before = cgroup::current(dir);
        cgroup::reclaim(dir, left.min(STEP));
        if before.saturating_sub(cgroup::current(dir)) < STEP / 4 {
            break;
        }
        left = left.saturating_sub(STEP);
    }
}

/// Write the status file atomically, world readable. The ticker, a finishing `serve` and
/// `shutdown` may write at once: each gets its own temporary (S4-10), so a reader never sees a
/// truncated file and no writer renames another's half-written one.
pub fn write_status(path: &Path, st: &shared::memsaver::Status) -> std::io::Result<()> {
    let body = serde_json::to_vec(st).map_err(std::io::Error::other)?;
    shared::atomic::atomic_write(path, 0o644, &body)
}

/// The running service: the engine behind a lock, a ticker thread, and an allocation-free
/// `focus` for wold's QoS hot path.
pub struct Service {
    engine: std::sync::Mutex<engine::Engine<Real>>,
    front: std::sync::atomic::AtomicI32,
    ticker: std::sync::OnceLock<std::thread::Thread>,
    status: PathBuf,
}

/// Background apps are looked at this often (focus and snapshots wake the ticker at once).
pub const TICK: std::time::Duration = std::time::Duration::from_secs(2);

/// After a round that panicked the ticker waits this much longer, so a fault that repeats every
/// round does not fill the journal.
const PANIC_PAUSE: std::time::Duration = std::time::Duration::from_secs(20);

/// One ticker round: the frontmost app, then the tick. None when it panicked (the engine's lock is
/// poisoned and its tables may be half way; the next round starts from what the system shows).
fn guarded_tick<S: Sys>(
    engine: &std::sync::Mutex<engine::Engine<S>>,
    front: i32,
    now: u64,
) -> Option<shared::memsaver::Status> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut e = engine.lock().unwrap_or_else(|p| p.into_inner());
        if front > 0 {
            e.focus(front);
        }
        e.tick(now)
    }))
    .ok()
}

/// Thaw every frozen cgroup under `root`/user.slice, whatever the engine believes: after a panic
/// its table of apps may have lost the very scope it had just frozen. Returns how many were thawed.
pub fn thaw_all(root: &Path) -> usize {
    fn walk(dir: &Path, depth: u32, thawed: &mut usize) {
        if std::fs::read_to_string(dir.join("cgroup.freeze")).is_ok_and(|v| v.trim() == "1")
            && cgroup::freeze(dir, false)
        {
            *thawed += 1;
        }
        if depth == 0 {
            return;
        }
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&e.path(), depth - 1, thawed);
            }
        }
    }
    let mut thawed = 0;
    walk(&root.join("user.slice"), 6, &mut thawed);
    thawed
}

/// How long the ticker sleeps: one [`TICK`], or less when a budget's duty cycle is due sooner
/// (a frozen app is thawed on time, so its messages wait at most the cycle's stop time).
fn next_wait(due: Option<u64>, now: u64) -> std::time::Duration {
    due.map_or(TICK, |d| {
        TICK.min(std::time::Duration::from_millis(d.saturating_sub(now)))
    })
}

fn now_ms() -> u64 {
    shared::sys::monotonic_ns() / 1_000_000
}

impl Service {
    pub fn new(status: impl Into<PathBuf>) -> Self {
        Service {
            engine: std::sync::Mutex::new(engine::Engine::new(Real::default(), Default::default())),
            front: Default::default(),
            ticker: Default::default(),
            status: status.into(),
        }
    }

    fn engine(&self) -> std::sync::MutexGuard<'_, engine::Engine<Real>> {
        self.engine.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn wake(&self) {
        if let Some(t) = self.ticker.get() {
            t.unpark();
        }
    }

    /// The frontmost app changed. Does not allocate or block.
    pub fn focus(&self, pid: i32) {
        self.front.store(pid, std::sync::atomic::Ordering::Relaxed);
        self.wake();
    }

    /// Runs forever on the calling thread. A panic in one round is caught: every frozen app is
    /// thawed (nothing watches them any more) and the ticker goes on (QA-SYS-9).
    pub fn run_ticker(&self) {
        let _ = self.ticker.set(std::thread::current());
        let mut last = None;
        loop {
            std::thread::park_timeout(next_wait(self.engine().due(), now_ms()));
            let front = self.front.swap(0, std::sync::atomic::Ordering::Relaxed);
            let Some(st) = guarded_tick(&self.engine, front, now_ms()) else {
                // The panic itself is in the journal (the panic hook); what stays frozen is not.
                thaw_all(Path::new(cgroup::ROOT));
                last = None;
                std::thread::park_timeout(PANIC_PAUSE);
                continue;
            };
            if last.as_ref() != Some(&st) {
                if let Err(e) = write_status(&self.status, &st) {
                    eprintln!("wold: memsaver status: {e}");
                }
                last = Some(st);
            }
        }
    }

    /// One session agent connection: snapshot lines until EOF, then everything is restored.
    pub fn serve(&self, conn: impl std::io::Read, peer: shared::sys::Peer) {
        use std::io::BufRead;
        if !self.engine().connect(peer.uid, peer.pid) {
            eprintln!(
                "wold: memsaver: peer {} is not in a login session",
                peer.pid
            );
            return;
        }
        let mut r = std::io::BufReader::new(conn);
        let mut line = Vec::new();
        loop {
            line.clear();
            let n = match std::io::Read::take(&mut r, shared::memsaver::MAX_LINE as u64)
                .read_until(b'\n', &mut line)
            {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if line.last() != Some(&b'\n') {
                // Overlong: skip to the end of this line.
                if n == shared::memsaver::MAX_LINE {
                    let mut sink = Vec::new();
                    if r.read_until(b'\n', &mut sink).unwrap_or(0) == 0 {
                        break;
                    }
                }
                continue;
            }
            if let Some(s) = std::str::from_utf8(&line)
                .ok()
                .and_then(shared::memsaver::Snapshot::parse)
            {
                self.engine().snapshot(s);
                self.wake();
            }
        }
        let st = self.engine().disconnect(now_ms());
        let _ = write_status(&self.status, &st);
    }

    /// wold is stopping: no app stays frozen.
    pub fn shutdown(&self) {
        let st = self.engine().disconnect(now_ms());
        let _ = write_status(&self.status, &st);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::memsaver::{AppState, AppStatus, Status};

    fn paging_of(r: &Real) -> Vec<String> {
        r.paging.map().keys().cloned().collect()
    }

    /// A Nap over a Save: the first page-out is stopped, not orphaned, and the entry left is the
    /// second's; an ended page-out takes only its own entry out.
    #[test]
    fn a_second_give_back_stops_the_first_and_ended_ones_are_forgotten() {
        let p = Paging::default();
        let save = p.start("/a");
        let nap = p.start("/a");
        assert!(
            save.load(Ordering::Relaxed),
            "the Save's page-out was stopped"
        );
        assert!(!nap.load(Ordering::Relaxed));
        p.end("/a", &save);
        assert!(
            p.map().contains_key("/a"),
            "the Save ending leaves the Nap's entry"
        );
        p.end("/a", &nap);
        assert!(p.map().is_empty());
        let other = p.start("/b");
        p.cancel("/b");
        assert!(other.load(Ordering::Relaxed));
        assert!(p.map().is_empty());
    }

    /// A saved scope that exits: its give-back ends and its entry goes, with no cancel.
    #[test]
    fn a_finished_give_back_leaves_no_entry() {
        let d = std::env::temp_dir().join(format!("wol-msgive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("proc")).unwrap();
        let r = Real {
            proc: d.join("proc"),
            root: d.join("cg"),
            paging: Arc::default(),
        };
        r.give_back("/user.slice/gone.scope", 1 << 20, vec![i32::MAX]);
        let t = std::time::Instant::now();
        while !paging_of(&r).is_empty() {
            assert!(
                t.elapsed() < std::time::Duration::from_secs(10),
                "{:?}",
                paging_of(&r)
            );
            std::thread::yield_now();
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// S4-10: the ticker, `serve` and `shutdown` write at once; a reader always parses a whole
    /// [`Status`] and the file stays world readable.
    #[test]
    fn concurrent_status_writes_never_tear() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        let d = std::env::temp_dir().join(format!("wol-msstatus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("wol-memsaver.json");
        write_status(&p, &Status::default()).unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let reader = {
            let (p, stop) = (p.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut n = 0u32;
                while !stop.load(Ordering::Relaxed) {
                    let t = std::fs::read(&p).unwrap();
                    serde_json::from_slice::<Status>(&t).unwrap();
                    n += 1;
                }
                n
            })
        };
        let writers: Vec<_> = (0..8u64)
            .map(|w| {
                let p = p.clone();
                std::thread::spawn(move || {
                    for i in 0..100u64 {
                        let st = Status {
                            enabled: true,
                            saved: i,
                            apps: (0..(i % 7))
                                .map(|k| AppStatus {
                                    app: format!("app-{w}-{k}"),
                                    state: AppState::Saving,
                                    saved: k,
                                    memory: k << 20,
                                    swap: 0,
                                    pageouts: 0,
                                })
                                .collect(),
                        };
                        write_status(&p, &st).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A duty cycle due in 300 ms wakes the ticker in 300 ms, not a whole tick later; one already
    /// due wakes it at once; none, one tick.
    #[test]
    fn the_ticker_wakes_for_a_due_duty_cycle() {
        use std::time::Duration;
        assert_eq!(next_wait(None, 1_000), TICK);
        assert_eq!(next_wait(Some(1_300), 1_000), Duration::from_millis(300));
        assert_eq!(next_wait(Some(900), 1_000), Duration::ZERO);
        assert_eq!(next_wait(Some(60_000), 1_000), TICK);
    }

    /// A `Sys` whose every question is a panic: the model of a bug inside the engine.
    struct Boom;
    impl Sys for Boom {
        fn cgroup(&self, _pid: i32) -> Option<String> {
            panic!("engine bug")
        }
        fn owner_and_parent(&self, _pid: i32) -> Option<(u32, i32)> {
            panic!("engine bug")
        }
        fn comm(&self, _pid: i32) -> String {
            panic!("engine bug")
        }
        fn is_game(&self, _pid: i32) -> bool {
            panic!("engine bug")
        }
        fn parents(&self) -> Vec<(i32, i32)> {
            panic!("engine bug")
        }
        fn exists(&self, _cg: &str) -> bool {
            panic!("engine bug")
        }
        fn procs(&self, _cg: &str) -> Vec<i32> {
            panic!("engine bug")
        }
        fn current(&self, _cg: &str) -> u64 {
            panic!("engine bug")
        }
        fn cpu_usec(&self, _cg: &str) -> u64 {
            panic!("engine bug")
        }
        fn freeze(&self, _cg: &str, _frozen: bool) -> bool {
            panic!("engine bug")
        }
        fn refused(&self, _cg: &str) -> u64 {
            panic!("engine bug")
        }
        fn kill(&self, _cg: &str) -> bool {
            panic!("engine bug")
        }
        fn oom_kills(&self, _cg: &str) -> u64 {
            panic!("engine bug")
        }
        fn tasks(&self, _cg: &str) -> u64 {
            panic!("engine bug")
        }
        fn adopt(&self, _args: &[String]) -> bool {
            panic!("engine bug")
        }
        fn give_back(&self, _scope: &str, _bytes: u64, _pids: Vec<i32>) {
            panic!("engine bug")
        }
        fn page_out(&self, _scope: &str, _pids: Vec<i32>) {
            panic!("engine bug")
        }
        fn cancel(&self, _scope: &str) {
            panic!("engine bug")
        }
        fn swapped(&self, _pids: &[i32]) -> u64 {
            panic!("engine bug")
        }
        fn set_high(&self, _cg: &str, _bytes: Option<u64>) -> bool {
            panic!("engine bug")
        }
    }

    /// QA-SYS-9: a panic in a tick leaves an app frozen with nobody to thaw it while wold lives
    /// on. The round reports it (None), and `thaw_all` thaws every frozen cgroup under the user
    /// slice, also one no table of the engine names.
    #[test]
    fn ticker_panic_thaws() {
        let engine = Mutex::new(engine::Engine::new(Boom, Default::default()));
        engine.lock().unwrap().snapshot(shared::memsaver::Snapshot {
            enabled: true,
            windows: vec![shared::memsaver::Window {
                pid: 4242,
                app: "notes".into(),
                visible: false,
            }],
            ..Default::default()
        });
        assert!(
            guarded_tick(&engine, 0, 1000).is_none(),
            "the panic is caught, not propagated"
        );
        // The lock is poisoned now; the next round still runs (and, here, panics again).
        assert!(guarded_tick(&engine, 7, 3000).is_none());

        let d = std::env::temp_dir().join(format!("wol-msthaw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let user = d.join("user.slice").join("user-1000.slice");
        let frozen = user.join("app-wol-notes-4242.scope");
        let deep = user
            .join("user@1000.service")
            .join("app.slice")
            .join("app-x.scope");
        let awake = user.join("app-wol-files-1.scope");
        for (dir, v) in [(&frozen, "1"), (&deep, "1"), (&awake, "0")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("cgroup.freeze"), v).unwrap();
        }
        // Outside the user slice nothing is touched.
        let system = d.join("system.slice").join("x.service");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join("cgroup.freeze"), "1").unwrap();
        assert_eq!(thaw_all(&d), 2);
        let read = |p: &Path| std::fs::read_to_string(p.join("cgroup.freeze")).unwrap();
        assert_eq!(read(&frozen), "0");
        assert_eq!(read(&deep), "0");
        assert_eq!(read(&awake), "0");
        assert_eq!(read(&system), "1");
        let _ = std::fs::remove_dir_all(&d);
    }
}
