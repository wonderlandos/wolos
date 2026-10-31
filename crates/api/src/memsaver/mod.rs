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
    /// Stop a [`Sys::give_back`] of this scope.
    fn cancel(&self, scope: &str);
    /// Bytes of these processes that are compressed away (swap), wherever they were charged.
    fn swapped(&self, pids: &[i32]) -> u64;
}

/// The real system: /proc, /sys/fs/cgroup and busctl.
pub struct Real {
    pub proc: PathBuf,
    pub root: PathBuf,
    /// Running page-outs by scope: their stop flags.
    paging: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Default for Real {
    fn default() -> Self {
        Real {
            proc: "/proc".into(),
            root: cgroup::ROOT.into(),
            paging: Mutex::default(),
        }
    }
}

impl Real {
    fn dir(&self, cg: &str) -> PathBuf {
        self.root.join(cg.trim_start_matches('/'))
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
        std::process::Command::new("busctl")
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    fn give_back(&self, scope: &str, bytes: u64, pids: Vec<i32>) {
        let stop = Arc::new(AtomicBool::new(false));
        self.paging
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(scope.to_owned(), stop.clone());
        let (proc, dir) = (self.proc.clone(), self.dir(scope));
        std::thread::spawn(move || {
            // The compression runs on this thread: it must not compete with what is in use.
            // SAFETY: setpriority on the calling thread (who = 0) with plain integers.
            unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 19) };
            let t = std::time::Instant::now();
            reclaim(&dir, bytes, &stop);
            let paged: usize = pids
                .iter()
                .map(|p| pageout::pageout(&proc, *p, &stop))
                .sum();
            eprintln!(
                "wold: memsaver gave back in {} ms ({} MiB of ranges){}",
                t.elapsed().as_millis(),
                paged >> 20,
                if stop.load(Ordering::Relaxed) {
                    " (stopped)"
                } else {
                    ""
                }
            );
        });
    }
    fn cancel(&self, scope: &str) {
        if let Some(stop) = self
            .paging
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(scope)
        {
            stop.store(true, Ordering::Relaxed);
        }
    }
    fn swapped(&self, pids: &[i32]) -> u64 {
        pids.iter().map(|p| pageout::swapped(&self.proc, *p)).sum()
    }
}

/// Reclaim in steps so `stop` is seen quickly; a step that barely helps means only hot or shared
/// pages are left.
fn reclaim(dir: &Path, bytes: u64, stop: &AtomicBool) {
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

/// Write the status file atomically, world readable.
pub fn write_status(path: &Path, st: &shared::memsaver::Status) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(st).map_err(std::io::Error::other)?)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(tmp, path)
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

    /// Runs forever on the calling thread.
    pub fn run_ticker(&self) {
        let _ = self.ticker.set(std::thread::current());
        let mut last = None;
        loop {
            std::thread::park_timeout(TICK);
            let st = {
                let mut e = self.engine();
                let f = self.front.swap(0, std::sync::atomic::Ordering::Relaxed);
                if f > 0 {
                    e.focus(f);
                }
                e.tick(now_ms())
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
