// SPDX-License-Identifier: GPL-2.0-or-later
//! Who is on the other end of a socket, captured once at accept (ADR 0020, audit B1/A2). The peer
//! uid and pid come from SO_PEERCRED; `/proc/<pid>` is opened as a directory right away, so the
//! descriptor pins that process: if it dies and the pid is reused, reads through it fail instead of
//! describing the newcomer. The executable is read at accept through that descriptor and read again
//! before anything is granted: a peer that sent its request and then exec()ed into another binary
//! is refused, never identified as the binary it became.
use crate::portal::{Caller, SHOWERS};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

/// Longest executable path we accept (bytes): PATH_MAX.
const MAX_EXE: usize = 4096;

/// Most bytes of a peer's `environ` or `maps` we read; a process past it is not trusted as a shower.
const MAX_PROC_FILE: u64 = 4 << 20;

/// Environment variables that put foreign code into a program before its `main` (the dynamic
/// loader, GTK, GIO, GStreamer, Qt and pixbuf plug-in paths). Every `LD_*` counts as well.
const INJECTING_ENV: [&str; 9] = [
    "GTK_MODULES",
    "GTK3_MODULES",
    "GTK_PATH",
    "GIO_EXTRA_MODULES",
    "GDK_PIXBUF_MODULE_FILE",
    "GST_PLUGIN_PATH",
    "GST_PLUGIN_SYSTEM_PATH",
    "QT_PLUGIN_PATH",
    "GSETTINGS_SCHEMA_DIR",
];

/// Where code the shell runs may be mapped from: the sealed system. A library from a home, a
/// temporary directory or a memfd is code a same-account process put there. (A `/usr` file
/// replaced since the shell started shows as deleted but is still the system's, so it passes.)
const CODE_ROOTS: [&str; 3] = ["/usr/", "/lib/", "/lib64/"];

/// What the kernel appends to `/proc/<pid>/exe` once the file was replaced or removed (an update
/// replaced the binary the process still runs).
const DELETED: &str = " (deleted)";

/// Executables believed by their path, each held to [`PeerIdentity::injected`] (QA-100 L3): the
/// notification shower, the helper gate that may raise an administrator sheet, and the web app
/// host whose scope names the app.
pub const PATH_TRUSTED: [&str; 3] = [SHOWERS[0], shared::portal::HELPER_GATE, crate::webapp::HOST];

/// The kernel's account of a connected peer.
#[derive(Debug)]
pub struct PeerIdentity {
    pub uid: u32,
    pub pid: i32,
    /// Canonical executable at accept time; None if the kernel would not say.
    exe: Option<String>,
    /// `/proc/<pid>` opened at accept.
    dir: Option<std::fs::File>,
    /// Executables identified by path alone ([`PATH_TRUSTED`]); a seam for tests.
    showers: &'static [&'static str],
}

impl PeerIdentity {
    /// SO_PEERCRED plus a pinned `/proc/<pid>` and its executable, right now. None if the socket
    /// has no peer credentials (not a Unix socket).
    pub fn capture(stream: &impl AsRawFd) -> Option<Self> {
        let p = shared::sys::peer(stream)?;
        let dir = (p.pid > 0)
            .then(|| std::fs::File::open(format!("/proc/{}", p.pid)).ok())
            .flatten();
        let exe = dir.as_ref().and_then(exe_of);
        Some(Self {
            uid: p.uid,
            pid: p.pid,
            exe,
            dir,
            showers: &PATH_TRUSTED,
        })
    }

    /// The raw credentials.
    pub fn peer(&self) -> shared::sys::Peer {
        shared::sys::Peer {
            uid: self.uid,
            pid: self.pid,
        }
    }

    /// True while the pinned process still runs the executable it ran at accept.
    pub fn still_same(&self) -> bool {
        match (&self.exe, &self.dir) {
            (Some(was), Some(dir)) => exe_of(dir).as_ref() == Some(was),
            _ => false,
        }
    }

    /// The peer is no longer who it was at accept (audit S4-26): an executable was known then and
    /// now differs or cannot be read (an exec, or the pinned process is gone). An executable that
    /// was unknown on first sight is not a change: no identity, nothing to lose. For calls that
    /// need no grant (`publish`, `subscribe`); grants use [`PeerIdentity::caller`], which also
    /// refuses an unknown executable.
    pub fn changed(&self) -> bool {
        match (&self.exe, &self.dir) {
            (None, _) => false,
            (Some(was), Some(dir)) => exe_of(dir).as_ref() != Some(was),
            (Some(_), None) => true,
        }
    }

    /// The calling app for a grant decision. None (refuse) if the executable is unknown, not an
    /// absolute path, or changed since accept; and, for a program trusted by its path alone (the
    /// shower, audit P6-1; the helper gate and the web host, QA-100 L3), if the process shows signs
    /// of carrying foreign code ([`Self::injected`]).
    pub fn caller(&self) -> Option<Caller> {
        let app = self.exe.clone()?;
        if !(app.starts_with('/') && self.still_same()) {
            return None;
        }
        if self.showers.contains(&app.as_str()) && self.injected() {
            return None;
        }
        // The one program told apart by its scope: the web app host (the scope names the app).
        let scope = (app == crate::webapp::HOST)
            .then(|| self.web_app())
            .flatten();
        Some(Caller {
            uid: self.uid,
            pid: self.pid,
            app,
            scope,
        })
    }

    /// The pinned process carries, or may carry, code its executable did not ship with (audit
    /// P6-1, defence in depth: the path alone is not an identity, ADR 0037). True when its initial
    /// environment names `LD_*`, `GTK_MODULES` and kin, when it has executable code mapped from
    /// outside the sealed system, and whenever either cannot be read: unknown is refused.
    pub fn injected(&self) -> bool {
        let Some(dir) = &self.dir else {
            return true;
        };
        let (Some(env), Some(maps), Some(status)) = (
            proc_file(dir, "environ"),
            proc_file(dir, "maps"),
            proc_file(dir, "status"),
        ) else {
            return true;
        };
        // A tracer already attached (PTRACE_TRACEME before exec, which survives it) can rewrite
        // the process at will; a mount namespace of its own can lay another /usr over the system's.
        if traced(&String::from_utf8_lossy(&status)) || !same_mount_namespace(dir) {
            return true;
        }
        // An empty environment is a process in the middle of an exec (or one that wiped it): no
        // account's shell starts with none.
        env.is_empty() || env_injects(&env) || maps_inject(&String::from_utf8_lossy(&maps))
    }

    /// [`PeerIdentity::capture`] of `stream`, with the executable seen at accept replaced by
    /// `exe` (tests of what a changed or unknown identity may do).
    #[cfg(test)]
    pub(crate) fn captured_as(stream: &impl AsRawFd, exe: Option<&str>) -> Option<Self> {
        let mut id = Self::capture(stream)?;
        id.exe = exe.map(str::to_owned);
        Some(id)
    }

    /// The identity of process `pid` as if it had connected, trusted by path as a shower when its
    /// own executable is in `showers` (tests of a real child exec()ed under a hostile environment).
    #[cfg(test)]
    pub(crate) fn of_process(pid: i32, showers: &'static [&'static str]) -> Self {
        let dir = std::fs::File::open(format!("/proc/{pid}")).ok();
        let exe = dir.as_ref().and_then(exe_of);
        Self {
            uid: 0,
            pid,
            exe,
            dir,
            showers,
        }
    }

    /// The web app this process belongs to (ADR 0048): the app scope its cgroup names for its own
    /// pid (`wol_core::sysapp::app_of_cgroup`), read through the pinned `/proc/<pid>`. None when
    /// the process changed since accept, has no such scope or cannot be read.
    pub fn web_app(&self) -> Option<String> {
        if !self.still_same() {
            return None;
        }
        let text = proc_file(self.dir.as_ref()?, "cgroup")?;
        wol_core::sysapp::app_of_cgroup(&String::from_utf8_lossy(&text), self.pid)
    }

    /// The command line of the pinned process (its arguments, NUL-split, bounded), read now; None
    /// when the process changed since accept or cannot be read. The web host's first argument is
    /// the manifest it runs (QA-100 H9-08).
    pub fn args(&self) -> Option<Vec<String>> {
        if !self.still_same() {
            return None;
        }
        let raw = proc_file(self.dir.as_ref()?, "cmdline")?;
        let raw = raw.strip_suffix(&[0]).unwrap_or(&raw);
        Some(
            raw.split(|b| *b == 0)
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect(),
        )
    }

    /// The value of environment variable `name` in the pinned process's initial environment, read
    /// through the pinned `/proc/<pid>` (never by pid number, QA-A9-SEC-6); None when the process
    /// changed since accept, cannot be read, or has no such variable.
    pub fn env_var(&self, name: &str) -> Option<String> {
        if !self.still_same() {
            return None;
        }
        let env = proc_file(self.dir.as_ref()?, "environ")?;
        let key = format!("{name}=");
        env.split(|b| *b == 0)
            .find_map(|e| e.strip_prefix(key.as_bytes()))
            .and_then(|v| String::from_utf8(v.to_vec()).ok())
    }

    /// Executable bytes as captured at accept (for exact-path checks such as the realtime agent).
    pub fn exe(&self) -> Option<&str> {
        self.exe.as_deref()
    }
}

/// A file of the process a `/proc/<pid>` descriptor pins, read through `/proc/self/fd` (bounded).
fn proc_file(dir: &std::fs::File, name: &str) -> Option<Vec<u8>> {
    let f = std::fs::File::open(format!("/proc/self/fd/{}/{name}", dir.as_raw_fd())).ok()?;
    let mut buf = Vec::new();
    f.take(MAX_PROC_FILE + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= MAX_PROC_FILE).then_some(buf)
}

/// Whether a `status` file names a tracer (`TracerPid:` not 0). No such line counts as traced.
fn traced(status: &str) -> bool {
    status
        .lines()
        .find_map(|l| l.strip_prefix("TracerPid:"))
        .is_none_or(|v| v.trim() != "0")
}

/// Whether the pinned process is in the mount namespace wold is in.
fn same_mount_namespace(dir: &std::fs::File) -> bool {
    let theirs = std::fs::read_link(format!("/proc/self/fd/{}/ns/mnt", dir.as_raw_fd()));
    let ours = std::fs::read_link("/proc/self/ns/mnt");
    matches!((theirs, ours), (Ok(t), Ok(o)) if t == o)
}

/// Whether a NUL-separated environment names a variable that loads foreign code.
fn env_injects(environ: &[u8]) -> bool {
    environ.split(|b| *b == 0).any(|entry| {
        let name = entry.split(|b| *b == b'=').next().unwrap_or_default();
        name.starts_with(b"LD_") || INJECTING_ENV.iter().any(|n| n.as_bytes() == name)
    })
}

/// Whether a `maps` listing has executable code that is not from the sealed system: a path
/// elsewhere or a memfd. Anonymous executable memory is not looked at (JIT).
fn maps_inject(maps: &str) -> bool {
    maps.lines().any(|line| {
        // address perms offset dev inode [path]; the path may hold spaces.
        let mut rest = line;
        let mut perms = "";
        for i in 0..5 {
            rest = rest.trim_start();
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            if i == 1 {
                perms = &rest[..end];
            }
            rest = &rest[end..];
        }
        let path = rest.trim();
        perms.contains('x')
            && !path.is_empty()
            && !path.starts_with('[')
            && !CODE_ROOTS.iter().any(|r| path.starts_with(r))
    })
}

/// `exe` of the process a `/proc/<pid>` descriptor pins, read through `/proc/self/fd` so the pid is
/// never looked up again. An update that replaced the running binary leaves the link reading
/// `<path> (deleted)` (api-security 3): that is still the app at `<path>`, so the suffix goes, but
/// only when the running file really has no name left (link count 0). A program whose own name
/// ends in ` (deleted)` keeps its whole name and is not taken for the one without the suffix.
fn exe_of(dir: &std::fs::File) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let link = PathBuf::from(format!("/proc/self/fd/{}/exe", dir.as_raw_fd()));
    let target = std::fs::read_link(&link).ok()?;
    let mut s = target.into_os_string().into_string().ok()?;
    if let Some(path) = s.strip_suffix(DELETED)
        && std::fs::metadata(&link).is_ok_and(|m| m.nlink() == 0)
    {
        s.truncate(path.len());
    }
    (s.len() < MAX_EXE).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn a_live_peer_is_identified_and_stays_the_same() {
        let (a, _b) = UnixStream::pair().unwrap();
        let id = PeerIdentity::capture(&a).unwrap();
        assert_eq!(id.pid, i32::try_from(std::process::id()).unwrap());
        assert!(id.still_same());
        let me = std::env::current_exe().unwrap();
        assert_eq!(id.caller().unwrap().app, me.to_string_lossy());
    }

    #[test]
    fn a_dead_peer_is_not_the_same() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        let dir = std::fs::File::open(format!("/proc/{pid}")).unwrap();
        let id = PeerIdentity {
            uid: 0,
            pid,
            exe: exe_of(&dir),
            dir: Some(dir),
            showers: &SHOWERS,
        };
        assert!(id.still_same());
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!id.still_same(), "the pinned process is gone");
        assert!(id.caller().is_none());
        assert!(id.changed(), "known at accept, gone now: a change");
    }

    /// Audit S4-26: only a change of a known identity counts; an executable unknown on first
    /// sight is not refused for grant-free calls (it still gets no grant).
    #[test]
    fn only_a_known_identity_can_change() {
        let (a, _b) = UnixStream::pair().unwrap();
        let live = PeerIdentity::capture(&a).unwrap();
        assert!(!live.changed(), "same executable as at accept");
        let unknown = PeerIdentity {
            uid: 1000,
            pid: live.pid,
            exe: None,
            dir: None,
            showers: &SHOWERS,
        };
        assert!(!unknown.changed(), "unknown on first sight is not a change");
        assert!(unknown.caller().is_none(), "and still no grant");
        let execd = PeerIdentity {
            uid: 1000,
            pid: live.pid,
            exe: Some("/usr/bin/not-this-test".into()),
            dir: std::fs::File::open(format!("/proc/{}", live.pid)).ok(),
            showers: &SHOWERS,
        };
        assert!(execd.changed(), "another executable than at accept");
    }

    /// Audit P6-1: names that load foreign code before `main` are refused; others are not.
    #[test]
    fn environment_that_injects_code_is_told_apart() {
        assert!(env_injects(b"HOME=/h\0LD_PRELOAD=/tmp/x.so\0"));
        assert!(env_injects(b"LD_LIBRARY_PATH=/tmp\0"));
        assert!(env_injects(b"LD_AUDIT=/tmp/a.so"));
        assert!(env_injects(b"A=b\0GTK_MODULES=evil\0"));
        assert!(!env_injects(b"HOME=/h\0WAYLAND_DISPLAY=wayland-1\0"));
        assert!(
            !env_injects(b"X=LD_PRELOAD\0"),
            "a value that mentions a name is not the name"
        );
        assert!(!env_injects(b""), "emptiness is `injected`'s to refuse");
    }

    #[test]
    fn code_from_outside_the_sealed_system_is_told_apart() {
        let sys = "7f00-7f10 r-xp 00000000 08:01 12   /usr/lib/libgtk-4.so.1\n\
                   7f20-7f30 r-xp 00000000 08:01 13   /usr/bin/wol-shell\n\
                   7f40-7f50 rw-p 00000000 00:00 0    [heap]\n\
                   7f60-7f70 rwxp 00000000 00:00 0\n\
                   7f80-7f90 r-xp 00000000 08:01 14   /usr/lib/old.so (deleted)\n\
                   7fa0-7fb0 r--p 00000000 08:01 15   /home/ann/data.bin\n";
        assert!(!maps_inject(sys));
        for evil in [
            "7f00-7f10 r-xp 00000000 08:01 12   /home/ann/.cache/evil.so",
            "7f00-7f10 r-xp 00000000 08:01 12   /tmp/e v il.so",
            "7f00-7f10 r-xp 00000000 00:01 12   /memfd:x (deleted)",
            "7f00-7f10 r-xp 00000000 08:01 12   /usrx/lib/e.so",
        ] {
            assert!(maps_inject(&format!("{sys}{evil}\n")), "{evil}");
        }
    }

    #[test]
    fn a_traced_process_is_told_apart() {
        assert!(!traced("Name:\tx\nTracerPid:\t0\nUid:\t1000\n"));
        assert!(traced("Name:\tx\nTracerPid:\t4242\n"));
        assert!(traced("Name:\tx\n"), "no line: unknown is refused");
    }

    /// A child of this test shares its mount namespace; one that unshared its own does not.
    #[test]
    fn a_process_in_another_mount_namespace_is_injected() {
        let me = std::fs::File::open("/proc/self").unwrap();
        assert!(same_mount_namespace(&me));
        // unshare(1) -m needs privilege we may not have; if it works the child must be refused.
        let un = std::process::Command::new("unshare")
            .args(["-m", "-r", "/bin/sleep", "10"])
            .env_clear()
            .env("HOME", "/")
            .stderr(std::process::Stdio::null())
            .spawn();
        let Ok(mut c) = un else {
            eprintln!("skipped: unshare(1) cannot be run here");
            return;
        };
        // unshare execs sleep a moment after it started: wait for that exec, bounded. Where it
        // may not map the account (uid_map EPERM) it exits at once: nothing to judge.
        let start = std::time::Instant::now();
        loop {
            if let Ok(Some(st)) = c.try_wait() {
                eprintln!("skipped: unshare -m -r is not permitted here ({st})");
                return;
            }
            let exe = std::fs::read_link(format!("/proc/{}/exe", c.id()));
            if exe.is_ok_and(|e| e.ends_with("sleep")) {
                break;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "unshare neither ran sleep nor exited"
            );
            std::thread::yield_now();
        }
        let id = PeerIdentity::of_process(i32::try_from(c.id()).unwrap(), &SHOWERS);
        let dir = id.dir.as_ref().expect("the running child's /proc entry");
        assert!(!same_mount_namespace(dir), "a mount namespace of its own");
        assert!(id.injected());
        let _ = c.kill();
        let _ = c.wait();
    }

    /// A child process, started with `env`, still running `sleep`.
    fn child(env: &[(&str, &str)]) -> std::process::Child {
        let mut c = std::process::Command::new("/bin/sleep");
        // Nothing of the test runner's own environment (cargo sets LD_LIBRARY_PATH for tests).
        c.arg("10")
            .env_clear()
            .env("HOME", "/")
            .stderr(std::process::Stdio::null());
        for (k, v) in env {
            c.env(k, v);
        }
        let child = c.spawn().unwrap();
        // The exec of sleep is complete once the executable is no longer this test's and it has an
        // environment to read.
        let me = std::env::current_exe().unwrap();
        for _ in 0..200 {
            let exe = std::fs::read_link(format!("/proc/{}/exe", child.id())).unwrap();
            let env = std::fs::read(format!("/proc/{}/environ", child.id())).unwrap_or_default();
            if exe != me && !env.is_empty() {
                break;
            }
            std::thread::yield_now(); // `spawn` returns after the exec; this is a safety net
        }
        child
    }

    /// The path of a child's executable as a one-entry shower list.
    fn showers_of(pid: u32) -> &'static [&'static str] {
        let exe = std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
        let s: &'static str = Box::leak(exe.to_string_lossy().into_owned().into_boxed_str());
        Box::leak(vec![s].into_boxed_slice())
    }

    /// Audit P6-1: the real path is accepted; the same executable under `LD_PRELOAD` (or any of
    /// the other loaders) is not a shower, and neither is one whose environment cannot be read.
    #[test]
    fn a_shower_binary_under_ld_preload_is_not_a_shower() {
        let mut clean = child(&[]);
        let pid = i32::try_from(clean.id()).unwrap();
        let ok = PeerIdentity::of_process(pid, showers_of(clean.id()));
        assert!(!ok.injected(), "a plain child carries no foreign code");
        assert!(ok.caller().is_some(), "the real path is accepted");
        clean.kill().unwrap();
        clean.wait().unwrap();

        for var in ["LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT", "GTK_MODULES"] {
            let mut bad = child(&[(var, "/nonexistent/evil.so")]);
            let id =
                PeerIdentity::of_process(i32::try_from(bad.id()).unwrap(), showers_of(bad.id()));
            assert!(id.injected(), "{var}");
            assert!(id.caller().is_none(), "{var}: no caller, no shower");
            bad.kill().unwrap();
            bad.wait().unwrap();
        }
    }

    /// Only the path-trusted programs are held to it: another app with `LD_LIBRARY_PATH` (a game
    /// launcher) is still identified and still needs its grant.
    #[test]
    fn other_apps_keep_their_identity_under_ld_library_path() {
        let mut other = child(&[("LD_LIBRARY_PATH", "/opt/lib")]);
        let id = PeerIdentity::of_process(i32::try_from(other.id()).unwrap(), &SHOWERS);
        assert!(id.injected());
        assert!(id.caller().is_some());
        other.kill().unwrap();
        other.wait().unwrap();
    }

    /// QA-100 M3: a binary replaced by an update (here: a copy, run, then unlinked) is still
    /// identified by its path, not by `<path> (deleted)`.
    #[test]
    fn exe_of_strips_deleted() {
        let dir = std::env::temp_dir().join(format!("wold-peer-deleted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("sleepy");
        std::fs::copy("/bin/sleep", &copy).unwrap();
        // ETXTBSY: a test thread that forks while the copy is still open for writing gives its
        // child that descriptor until the child's exec. The race ends with that exec, so the spawn
        // is tried again until the file is no longer open for writing anywhere (bounded).
        let spawn = || {
            std::process::Command::new(&copy)
                .arg("10")
                .env_clear()
                .env("HOME", "/")
                .spawn()
        };
        let mut c = (0..10_000)
            .find_map(|_| match spawn() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::yield_now();
                    None
                }
                other => Some(other),
            })
            .expect("the copy stays busy")
            .unwrap();
        let pid = i32::try_from(c.id()).unwrap();
        let want = copy.to_string_lossy().into_owned();
        for _ in 0..10_000 {
            if std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|e| e == copy) {
                break;
            }
            std::thread::yield_now();
        }
        let id = PeerIdentity::of_process(pid, &SHOWERS);
        std::fs::remove_file(&copy).unwrap();
        let raw = std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
        assert!(raw.to_string_lossy().ends_with(DELETED), "{raw:?}");
        assert!(id.still_same(), "replaced is not changed");
        assert_eq!(id.caller().map(|c| c.app), Some(want));
        assert_eq!(id.args().unwrap(), [copy.to_string_lossy().as_ref(), "10"]);
        let _ = c.kill();
        let _ = c.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// QA-100 L3: the helper gate and the web host are believed by path, so they are held to the
    /// injection check like the shower.
    #[test]
    fn injected_runs_for_helper_gate_and_host() {
        let (a, _b) = UnixStream::pair().unwrap();
        let id = PeerIdentity::capture(&a).unwrap();
        for exe in [SHOWERS[0], shared::portal::HELPER_GATE, crate::webapp::HOST] {
            assert!(id.showers.contains(&exe), "{exe}");
        }
        // `caller()` holds every listed path to `injected()`: the child tests above run it for a
        // listed path (a_shower_binary_under_ld_preload_is_not_a_shower); this is the list.
        assert_eq!(PATH_TRUSTED.len(), 3);
    }

    /// api-security 3: an update replaced the binary an app is running (the old file unlinked).
    /// The app is still identified by its path, before and after; a program whose own name ends
    /// in ` (deleted)` is not taken for the one without the suffix.
    #[test]
    fn an_app_whose_binary_was_replaced_keeps_its_identity() {
        // Next to the test binary: a directory files may be executed from.
        let me = std::env::current_exe().unwrap();
        let dir = me
            .parent()
            .unwrap()
            .join(format!("peer-deleted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let start = |path: &std::path::Path| {
            // Copied by cp(1): this process never holds the new file open for writing, so a
            // child another test thread forks cannot keep it busy (ETXTBSY) at our exec.
            let cp = std::process::Command::new("cp")
                .arg("/bin/sleep")
                .arg(path)
                .status()
                .unwrap();
            assert!(cp.success());
            std::process::Command::new(path)
                .arg("10")
                .env_clear()
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap()
        };
        let app = dir.join("app");
        let mut c = start(&app);
        let pid = i32::try_from(c.id()).unwrap();
        let id = PeerIdentity::of_process(pid, &SHOWERS);
        let path = app.to_string_lossy().into_owned();
        assert_eq!(id.exe(), Some(path.as_str()));
        std::fs::remove_file(&app).unwrap();
        let raw = std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
        assert!(raw.to_string_lossy().ends_with(DELETED), "{raw:?}");
        assert!(id.still_same(), "same process, same binary");
        assert!(!id.changed());
        assert_eq!(id.caller().map(|c| c.app), Some(path.clone()));
        let late = PeerIdentity::of_process(pid, &SHOWERS);
        assert_eq!(
            late.exe(),
            Some(path.as_str()),
            "connected after the update"
        );
        let _ = c.kill();
        let _ = c.wait();

        let odd = dir.join("app (deleted)");
        let mut c = start(&odd);
        let id = PeerIdentity::of_process(i32::try_from(c.id()).unwrap(), &SHOWERS);
        assert_eq!(
            id.exe(),
            Some(odd.to_string_lossy().as_ref()),
            "its own name"
        );
        let _ = c.kill();
        let _ = c.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// QA-A9-SEC-6: a variable of the peer is read through the pinned process, so it is the
    /// process that connected, and it is not read at all once that process has ended (a reused
    /// pid is another process: reading `/proc/<pid>/environ` by number would have named it).
    #[test]
    fn a_variable_is_read_from_the_pinned_process_only() {
        let mut c = child(&[("XDG_RUNTIME_DIR", "/run/user/4242")]);
        let id = PeerIdentity::of_process(i32::try_from(c.id()).unwrap(), &SHOWERS);
        assert_eq!(
            id.env_var("XDG_RUNTIME_DIR").as_deref(),
            Some("/run/user/4242")
        );
        assert_eq!(id.env_var("NOT_THERE"), None);
        let _ = c.kill();
        let _ = c.wait();
        assert_eq!(
            id.env_var("XDG_RUNTIME_DIR"),
            None,
            "the pinned process ended: nothing is read"
        );
    }

    /// QA-A9-SEC-6: the web host's scope comes with the caller (read through the pin), other
    /// programs carry none.
    #[test]
    fn only_the_web_host_carries_a_scope() {
        let (a, _b) = UnixStream::pair().unwrap();
        let id = PeerIdentity::capture(&a).unwrap();
        assert_eq!(
            id.caller().unwrap().scope,
            None,
            "the test binary is no host"
        );
    }

    #[test]
    fn an_unreadable_process_is_injected() {
        let (a, _b) = UnixStream::pair().unwrap();
        let mut id = PeerIdentity::capture(&a).unwrap();
        id.dir = None;
        assert!(id.injected(), "unknown is refused");
    }
}
