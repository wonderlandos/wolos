//! Memory Saver in wold (ADR 0022): every app gets its own delegated systemd scope with one child
//! cgroup per process role; background apps give memory back to zram (memory.reclaim) under
//! adaptive role ceilings (memory.high) and get it back on focus (ceilings off, prefetch).
//! Policy: `wol_core::memsaver`. Session facts: `shared::memsaver` snapshots from wol-realtime.
pub mod cgroup;
pub mod engine;
pub mod prefetch;
pub mod procs;

use std::path::{Path, PathBuf};

/// Everything the engine does to the system, so its logic runs against a model in tests.
/// Cgroup arguments are cgroup paths (`/user.slice/...`), not filesystem paths.
pub trait Sys {
    fn cgroup(&self, pid: i32) -> Option<String>;
    fn owner_and_parent(&self, pid: i32) -> Option<(u32, i32)>;
    fn comm(&self, pid: i32) -> String;
    fn argv(&self, pid: i32) -> Vec<String>;
    fn is_game(&self, pid: i32) -> bool;
    fn parents(&self) -> Vec<(i32, i32)>;
    fn exists(&self, cg: &str) -> bool;
    fn procs(&self, cg: &str) -> Vec<i32>;
    fn mkdir(&self, cg: &str) -> bool;
    fn attach(&self, cg: &str, pid: i32) -> bool;
    fn enable_memory(&self, cg: &str) -> bool;
    fn current(&self, cg: &str) -> u64;
    fn swap(&self, cg: &str) -> u64;
    fn cpu_usec(&self, cg: &str) -> u64;
    fn pressure(&self, cg: &str) -> f32;
    fn set_high(&self, cg: &str, bytes: Option<u64>) -> bool;
    fn reclaim(&self, cg: &str, bytes: u64) -> bool;
    fn freeze(&self, cg: &str, frozen: bool) -> bool;
    /// Ask systemd for a scope (busctl argv from [`procs::adopt_args`]).
    fn adopt(&self, args: &[String]) -> bool;
    /// Prefetch these processes' memory (in the background; returns at once).
    fn prefetch(&self, pids: Vec<i32>);
}

/// The real system: /proc, /sys/fs/cgroup and busctl.
pub struct Real {
    pub proc: PathBuf,
    pub root: PathBuf,
}

impl Default for Real {
    fn default() -> Self {
        Real {
            proc: "/proc".into(),
            root: cgroup::ROOT.into(),
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
    fn argv(&self, pid: i32) -> Vec<String> {
        procs::argv(&self.proc, pid)
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
    fn mkdir(&self, cg: &str) -> bool {
        let d = self.dir(cg);
        d.is_dir() || std::fs::create_dir(&d).is_ok()
    }
    fn attach(&self, cg: &str, pid: i32) -> bool {
        cgroup::attach(&self.dir(cg), pid)
    }
    fn enable_memory(&self, cg: &str) -> bool {
        cgroup::enable_memory(&self.dir(cg))
    }
    fn current(&self, cg: &str) -> u64 {
        cgroup::current(&self.dir(cg))
    }
    fn swap(&self, cg: &str) -> u64 {
        cgroup::swap(&self.dir(cg))
    }
    fn cpu_usec(&self, cg: &str) -> u64 {
        cgroup::cpu_usec(&self.dir(cg))
    }
    fn pressure(&self, cg: &str) -> f32 {
        cgroup::pressure(&self.dir(cg))
    }
    fn set_high(&self, cg: &str, bytes: Option<u64>) -> bool {
        cgroup::set_high(&self.dir(cg), bytes)
    }
    fn reclaim(&self, cg: &str, bytes: u64) -> bool {
        cgroup::reclaim(&self.dir(cg), bytes).is_some()
    }
    fn freeze(&self, cg: &str, frozen: bool) -> bool {
        cgroup::freeze(&self.dir(cg), frozen)
    }
    fn adopt(&self, args: &[String]) -> bool {
        std::process::Command::new("busctl")
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    fn prefetch(&self, pids: Vec<i32>) {
        let proc = self.proc.clone();
        std::thread::spawn(move || {
            let t = std::time::Instant::now();
            let bytes: usize = pids.iter().map(|p| prefetch::prefetch(&proc, *p)).sum();
            eprintln!(
                "wold: memsaver prefetched {} MiB in {} ms",
                bytes >> 20,
                t.elapsed().as_millis()
            );
        });
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
            engine: std::sync::Mutex::new(engine::Engine::new(
                Real::default(),
                Default::default(),
                Default::default(),
            )),
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

    /// wold is stopping: no app keeps a ceiling or stays frozen.
    pub fn shutdown(&self) {
        let st = self.engine().disconnect(now_ms());
        let _ = write_status(&self.status, &st);
    }
}
