//! Real OS side of wold: spawn jobs (QoS at spawn), stop process trees, signal handling.
use crate::qos::Applier;
use crate::supervisor::Runner;
use shared::jobs::JobSpec;
use shared::qos::QosClass;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct OsRunner {
    pub qos: Box<dyn Applier>,
    pub pid_dir: PathBuf,
}

/// Signals wold handles synchronously (blocked in every thread, taken by `wait_signal`).
pub const SIGNALS: [i32; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGCHLD];

fn sigset() -> libc::sigset_t {
    // SAFETY: sigset ops on a local, zero-initialised set.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in SIGNALS {
            libc::sigaddset(&mut set, s);
        }
        set
    }
}

/// Block [`SIGNALS`]; call before spawning any thread.
/// Also gives them a no-op handler: an ignored action (SIGCHLD's default, or SIG_IGN inherited across
/// exec) makes FreeBSD discard the signal at delivery even while blocked, and `sigwait` never sees it.
pub fn block_signals() {
    extern "C" fn noop(_: libc::c_int) {}
    for s in SIGNALS {
        // SAFETY: installs an async-signal-safe no-op handler before any thread exists.
        unsafe {
            let mut a: libc::sigaction = std::mem::zeroed();
            a.sa_sigaction = noop as *const () as libc::sighandler_t;
            a.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut a.sa_mask);
            libc::sigaction(s, &a, std::ptr::null_mut());
        }
    }
    let set = sigset();
    // SAFETY: valid set pointer; old mask not needed.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
}

/// Next blocked signal.
pub fn wait_signal() -> i32 {
    let set = sigset();
    let mut sig = 0;
    // SAFETY: valid set and out pointer.
    unsafe { libc::sigwait(&set, &mut sig) };
    sig
}

impl Runner for OsRunner {
    fn spawn(&self, spec: &JobSpec) -> std::io::Result<i32> {
        let mut cmd = std::process::Command::new(&spec.program[0]);
        cmd.args(&spec.program[1..]).process_group(0);
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                let set = sigset();
                libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
                Ok(())
            });
        }
        let pid = cmd.spawn()?.id() as i32; // never waited here: the reaper owns waitpid
        let class = spec
            .qos
            .as_deref()
            .and_then(QosClass::parse)
            .unwrap_or(QosClass::Default);
        if class != QosClass::Default && !self.qos.apply(pid, class) {
            eprintln!("wold: {} qos {} failed", spec.label, class.name());
        }
        if let Some(f) = crate::jobfiles::pid_file(&self.pid_dir, &spec.label) {
            let _ = std::fs::create_dir_all(&self.pid_dir);
            let _ = std::fs::write(f, pid.to_string());
        }
        Ok(pid)
    }

    fn stop_tree(&self, root: i32) {
        stop_tree(root);
    }
}

pub fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn send(pid: i32, sig: i32) -> bool {
    // SAFETY: plain syscall.
    pid > 1 && unsafe { libc::kill(pid, sig) == 0 }
}

/// TERM a process and its descendants (children first), KILL survivors after 3 s.
pub fn stop_tree(root: i32) {
    let ps = wol_core::system::run_with_timeout(
        &[&["ps"], &wol_core::jobs::PS_ARGS[..]].concat(),
        wol_core::system::RUN_TIMEOUT,
    )
    .unwrap_or_default();
    let mut tree = wol_core::jobs::process_tree(root, &wol_core::jobs::parse_ps_pairs(&ps));
    if tree.is_empty() {
        tree.push(root);
    }
    let sent = tree.iter().filter(|p| send(**p, libc::SIGTERM)).count();
    eprintln!(
        "wold: stop tree {root}: {} processes, {sent} signalled",
        tree.len()
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && tree.iter().any(|p| alive(*p)) {
        std::thread::sleep(Duration::from_millis(50));
    }
    for p in tree {
        send(p, libc::SIGKILL);
    }
}

/// Stop a job tree left by a previous wold (crash restart), from its pid file.
pub fn stop_stale(pid_dir: &std::path::Path, label: &str) {
    let Some(f) = crate::jobfiles::pid_file(pid_dir, label) else {
        return;
    };
    if let Some(root) = crate::jobfiles::read_pid(&f)
        && alive(root)
    {
        eprintln!("wold: stopping stale {label}");
        stop_tree(root);
    }
    let _ = std::fs::remove_file(f);
}

/// SIGTERM: whole reaper subtree on FreeBSD (3 s grace), else each job tree.
pub fn stop_all(running: &[i32]) {
    if crate::reaper::kill_all(libc::SIGTERM) {
        std::thread::sleep(Duration::from_secs(3));
        crate::reaper::kill_all(libc::SIGKILL);
    } else {
        std::thread::scope(|s| {
            for pid in running {
                s.spawn(|| stop_tree(*pid));
            }
        });
    }
    eprintln!("wold: stopped");
}
