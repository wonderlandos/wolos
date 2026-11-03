// SPDX-License-Identifier: GPL-2.0-or-later
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
/// SIGHUP and SIGPIPE are here so neither can end wold (audit S4-2): wold has no terminal, but a
/// terminal opened without O_NOCTTY would become its own and its hang-up would kill the
/// supervisor and every session under it. Taken, they are ignored (`wold` main loop); a SIGPIPE
/// raised by a write stays pending on the writing thread and the write returns EPIPE.
pub const SIGNALS: [i32; 5] = [
    libc::SIGTERM,
    libc::SIGINT,
    libc::SIGCHLD,
    libc::SIGHUP,
    libc::SIGPIPE,
];

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

/// `cmd` as account `uid`, in its home, with nothing of wold's environment. Std drops root's
/// supplementary groups with the uid.
fn as_account(cmd: &mut std::process::Command, uid: u32) -> std::io::Result<()> {
    let passwd = std::fs::read_to_string("/etc/passwd")?;
    let who = crate::services::identity(&passwd, uid).ok_or(std::io::ErrorKind::NotFound)?;
    cmd.uid(uid)
        .gid(who.gid)
        .env_clear()
        .env("HOME", &who.home)
        .env("USER", &who.name)
        .env("LOGNAME", &who.name)
        .env("PATH", "/usr/local/bin:/usr/bin")
        .env("XDG_RUNTIME_DIR", format!("/run/user/{uid}"))
        .current_dir(&who.home)
        .stdin(std::process::Stdio::null());
    Ok(())
}

/// Soft descriptor limit every job starts with: the usual default, so programs that use
/// `select()` (FD_SETSIZE 1024) keep working. wold itself runs with more (wold.service).
pub const CHILD_NOFILE_SOFT: u64 = 1024;

/// A job's (soft, hard) descriptor limit from wold's: soft down to [`CHILD_NOFILE_SOFT`] when
/// higher, hard unchanged (a job may still raise its soft limit up to it, as games do).
pub fn child_nofile(cur: u64, max: u64) -> (u64, u64) {
    (cur.min(CHILD_NOFILE_SOFT), max)
}

impl Runner for OsRunner {
    fn spawn(&self, spec: &JobSpec) -> std::io::Result<i32> {
        let mut cmd = std::process::Command::new(&spec.program[0]);
        cmd.args(&spec.program[1..]).process_group(0);
        if let Some(uid) = spec.user {
            as_account(&mut cmd, uid)?;
        }
        // SAFETY: only async-signal-safe calls between fork and exec (sigprocmask, getrlimit,
        // setrlimit on a local struct).
        unsafe {
            cmd.pre_exec(|| {
                let set = sigset();
                libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
                // S4-26: wold's own descriptor limit (wold.service LimitNOFILE) is not the
                // session's: jobs start from the usual soft limit, the hard one unchanged.
                let mut r: libc::rlimit = std::mem::zeroed();
                if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) == 0 {
                    let (cur, max) = child_nofile(r.rlim_cur, r.rlim_max);
                    if cur != r.rlim_cur {
                        r.rlim_cur = cur;
                        r.rlim_max = max;
                        libc::setrlimit(libc::RLIMIT_NOFILE, &r);
                    }
                }
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
    let ps = crate::privileged::capture(
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

/// SIGTERM: whole reaper subtree (3 s grace), else each job tree.
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

#[cfg(test)]
mod account_tests {
    use super::*;

    /// Needs root, so not in the normal run: `sudo <test binary> --ignored runs_as_the_account`.
    #[test]
    #[ignore]
    fn runs_as_the_account() {
        let out = std::env::temp_dir().join(format!("wol-as-{}", std::process::id()));
        let _ = std::fs::write(&out, "");
        std::fs::set_permissions(&out, std::os::unix::fs::PermissionsExt::from_mode(0o666))
            .unwrap();
        let mut c = std::process::Command::new("/bin/sh");
        let script = format!(
            "{{ id -u; id -g; id -G; env | wc -l; }} > {}",
            out.display()
        );
        c.args(["-c", &script]);
        as_account(&mut c, 65534).unwrap();
        c.current_dir("/");
        assert!(c.status().unwrap().success());
        let t = std::fs::read_to_string(&out).unwrap();
        let _ = std::fs::remove_file(&out);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0], "65534", "{t}");
        assert_eq!(lines[1], lines[2], "no group but its own: {t}");
        assert!(
            lines[3].parse::<u32>().unwrap() <= 8,
            "clean environment: {t}"
        );
    }
}
