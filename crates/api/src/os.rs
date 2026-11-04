// SPDX-License-Identifier: GPL-2.0-or-later
//! Real OS side of wold: spawn jobs (QoS at spawn), stop process trees, signal handling.
use crate::qos::Applier;
use crate::supervisor::Runner;
use shared::jobs::JobSpec;
use shared::qos::QosClass;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
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
                // QA L2-30: wold runs at OOMScoreAdjust=-900 (wold.service) and the value is
                // inherited. This write only works for a root job (the uid is already the
                // account's here for the others, and /proc/self/oom_score_adj of a process that
                // changed uid belongs to root); `spawn` resets the account jobs from wold's side
                // right after the fork (`reset_oom_adj`), where wold's own privilege writes it.
                let fd = libc::open(
                    c"/proc/self/oom_score_adj".as_ptr(),
                    libc::O_WRONLY | libc::O_CLOEXEC,
                );
                if fd >= 0 {
                    libc::write(fd, b"0".as_ptr().cast(), 1);
                    libc::close(fd);
                }
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
        reset_oom_adj(Path::new("/proc"), pid);
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

    fn adopt(&self, spec: &JobSpec) -> Option<i32> {
        adopt_stale(&self.pid_dir, spec, Path::new("/proc"))
    }

    fn alive(&self, pid: i32) -> bool {
        alive(pid)
    }
}

/// A job's process back to `oom_score_adj` 0 when it came out of the fork at wold's own value.
/// Raising a value needs no privilege, but wold (root) is the one that can write an account's
/// process file after its uid changed.
fn reset_oom_adj(proc_dir: &Path, pid: i32) {
    let f = proc_dir.join(pid.to_string()).join("oom_score_adj");
    if std::fs::read_to_string(&f).is_ok_and(|v| v.trim() != "0") {
        let _ = std::fs::write(f, "0");
    }
}

/// What a pid file left by a previous wold points at.
#[derive(Debug, PartialEq, Eq)]
pub enum Stale {
    /// The job's own program, still running: the new wold takes it over.
    Adopt,
    /// The job's program, but replaced on disk since (an update): stop it, start the new one.
    Outdated,
    /// Anything else, a recycled pid: never signalled (wold is root).
    Foreign,
}

/// `program` is the job's `program[0]`, `exe` what `/proc/<pid>/exe` links to.
pub fn classify_stale(program: &str, exe: &str) -> Stale {
    let canonical = std::fs::canonicalize(program)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| program.to_owned());
    let (exe, deleted) = match exe.strip_suffix(" (deleted)") {
        Some(e) => (e, true),
        None => (exe, false),
    };
    match (exe == canonical || exe == program, deleted) {
        (true, false) => Stale::Adopt,
        (true, true) => Stale::Outdated,
        _ => Stale::Foreign,
    }
}

/// The pid file a previous wold left for `spec` and what its pid is now: a live process (with the
/// executable it runs) classified by [`classify_stale`]; None without a pid file, or when the pid
/// is gone.
pub fn stale_of(pid_dir: &Path, spec: &JobSpec, proc_dir: &Path) -> Option<(i32, Stale)> {
    let f = crate::jobfiles::pid_file(pid_dir, &spec.label)?;
    let pid = crate::jobfiles::read_pid(&f)?;
    let exe = std::fs::read_link(proc_dir.join(pid.to_string()).join("exe")).ok()?;
    if !alive(pid) {
        return None;
    }
    Some((
        pid,
        classify_stale(spec.program.first()?, &exe.to_string_lossy()),
    ))
}

/// The pid a previous wold left running for `spec` when it is the same program (QA-SYS-2: a
/// crash or `systemctl restart wold` used to kill the whole login tree, session included).
/// Nothing is signalled or removed here; `wold` settles the rest at start (it says what it does).
pub fn adopt_stale(pid_dir: &Path, spec: &JobSpec, proc_dir: &Path) -> Option<i32> {
    match stale_of(pid_dir, spec, proc_dir)? {
        (pid, Stale::Adopt) => Some(pid),
        _ => None,
    }
}

/// `systemctl list-jobs --no-legend` says wold's own stop is the first half of a restart.
pub fn restart_pending(list_jobs: &str) -> bool {
    list_jobs.lines().any(|l| {
        let mut w = l.split_whitespace().skip(1);
        w.next() == Some("wold.service") && w.next().is_some_and(|t| t.contains("restart"))
    })
}

/// Ask systemd whether wold is being restarted (SIGTERM comes for a stop and a restart alike).
/// Unanswered is a stop.
pub fn restarting() -> bool {
    crate::privileged::capture(
        &["systemctl", "list-jobs", "--no-legend", "--no-pager"],
        std::time::Duration::from_secs(2),
    )
    .is_some_and(|out| restart_pending(&out))
}

pub fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Bytes free to ordinary writes and in all, on the filesystem that holds `path` or its nearest
/// existing parent. None when the kernel will not say (the copy is then refused).
#[allow(clippy::useless_conversion)] // statvfs field widths differ by target (u32 on some)
pub fn space_at(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let mut at = path;
    while !at.exists() {
        at = at.parent()?;
    }
    let c = std::ffi::CString::new(at.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is a NUL-terminated path and `st` a zeroed statvfs the call fills in.
    let st = unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(c.as_ptr(), &mut st) == 0).then_some(st)?
    };
    let unit = u64::from(st.f_frsize);
    Some((
        u64::from(st.f_bavail).saturating_mul(unit),
        u64::from(st.f_blocks).saturating_mul(unit),
    ))
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

/// SIGTERM: the whole reaper subtree and each job tree (3 s grace, then SIGKILL). A job wold
/// adopted from its predecessor is no descendant of this wold, so the job trees are stopped
/// explicitly as well as the subtree.
pub fn stop_all(running: &[i32]) {
    let subtree = crate::reaper::kill_all(libc::SIGTERM);
    std::thread::scope(|s| {
        for pid in running {
            s.spawn(|| stop_tree(*pid));
        }
    });
    if subtree {
        if running.is_empty() {
            std::thread::sleep(Duration::from_secs(3));
        }
        crate::reaper::kill_all(libc::SIGKILL);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `systemctl list-jobs --no-legend` while `systemctl restart wold` stops it, and while
    /// `stop` or a shutdown does.
    #[test]
    fn a_restart_of_wold_is_told_from_a_stop() {
        let restart = "  842 wold.service restart running\n  843 foo.service start waiting\n";
        let stop = "  842 wold.service stop running\n  850 reboot.target start waiting\n";
        let other =
            "  842 wolos-hw.service restart running\n  843 wold.service-x restart running\n";
        assert!(restart_pending(restart));
        assert!(!restart_pending(stop));
        assert!(!restart_pending(other));
        assert!(!restart_pending(""));
    }

    /// QA L2-30: a job comes out of the fork at wold's own OOM score and starts again from 0, so
    /// the supervisor's protection does not become every app's.
    #[test]
    fn a_job_does_not_inherit_the_oom_protection_of_wold() {
        let d = std::env::temp_dir().join(format!("wold-oomjob-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let out = d.join("adj");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&out)
                .status()
                .unwrap()
                .success()
        );
        // Raising is allowed without privilege: this process stands in for wold at a non-zero
        // score; the job must still start at 0.
        std::fs::write("/proc/self/oom_score_adj", "100").unwrap();
        let spec = JobSpec::parse(&format!(
            r#"{{"label":"oom","program":["/bin/sh","-c","cat /proc/self/oom_score_adj > {}"]}}"#,
            out.display()
        ))
        .unwrap();
        let runner = OsRunner {
            qos: Box::new(crate::qos::Sched),
            pid_dir: d.clone(),
        };
        let spawned = runner.spawn(&spec);
        std::fs::write("/proc/self/oom_score_adj", "0").unwrap();
        spawned.unwrap();
        // The job opens the pipe for writing; reading blocks until it has written.
        assert_eq!(std::fs::read_to_string(&out).unwrap().trim(), "0");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `reset_oom_adj` (wold's side, root writing an account's process): only a value that is not
    /// 0 is written.
    #[test]
    fn reset_oom_adj_writes_only_what_differs() {
        let d = std::env::temp_dir().join(format!("wold-oomfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (pid, adj) in [(100, "-900\n"), (101, "0\n"), (102, "300\n")] {
            std::fs::create_dir_all(d.join(pid.to_string())).unwrap();
            std::fs::write(d.join(pid.to_string()).join("oom_score_adj"), adj).unwrap();
        }
        for pid in [100, 101, 102, 103] {
            reset_oom_adj(&d, pid); // 103 has no file: nothing happens
        }
        let adj = |pid: i32| {
            std::fs::read_to_string(d.join(pid.to_string()).join("oom_score_adj")).unwrap()
        };
        assert_eq!(adj(100).trim(), "0");
        assert_eq!(adj(101), "0\n", "left as it was");
        assert_eq!(adj(102).trim(), "0");
        assert!(!d.join("103").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
