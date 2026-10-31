//! QoS service: applies `shared::qos::QosClass` policy; frontmost app boosting from wol-realtime.
use shared::qos::{ForegroundTracker, QosClass, SchedPolicy, parse_foreground};
use std::io::Read;
use std::path::Path;

/// Sets a process's scheduling class. Implementations must not allocate on success.
pub trait Applier: Send + Sync {
    fn apply(&self, pid: i32, class: QosClass) -> bool;
    /// Tell the GPU driver the frontmost pid (i915 foreground-only waitboost, ADR 0008). Best effort.
    fn gpu_foreground(&self, _pid: i32) {}
}

/// Linux mapping of a class, per thread: (scheduler, nice, I/O class, slice). The FreeBSD realtime class
/// has no safe Linux twin (SCHED_RR can starve the compositor): it becomes the strongest nice
/// boost until a sched_ext scheduler takes QoS over (ADR 0016).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Linux {
    /// SCHED_OTHER or SCHED_IDLE.
    pub idle: bool,
    pub nice: i8,
    /// ioprio_set value: IOPRIO_CLASS_IDLE, or best effort with the level nice implies.
    pub ioprio: i32,
    /// EEVDF request size (`sched_attr.sched_runtime`), 0 = kernel default. Without scx_wolos.
    pub slice_ns: u64,
}

const IOPRIO_CLASS_SHIFT: i32 = 13;
const IOPRIO_CLASS_BE: i32 = 2;
const IOPRIO_CLASS_IDLE: i32 = 3;
/// Strongest boost for UserInteractive (FreeBSD rtprio).
pub const INTERACTIVE_NICE: i8 = -15;
/// Slice of the two boosted classes: a shorter request waits less after every wakeup, and 2 ms
/// still fits a frame's work in one go. Longer slices on nice 0 made heavy frames later.
pub const BOOST_SLICE_NS: u64 = 2_000_000;

pub fn linux(class: QosClass) -> Linux {
    let be = |nice: i8| (IOPRIO_CLASS_BE << IOPRIO_CLASS_SHIFT) | ((i32::from(nice) + 20) / 5);
    match class.policy() {
        SchedPolicy::RtPrio(_) => Linux {
            idle: false,
            nice: INTERACTIVE_NICE,
            ioprio: be(INTERACTIVE_NICE),
            slice_ns: BOOST_SLICE_NS,
        },
        SchedPolicy::Nice(n) => Linux {
            idle: false,
            nice: n,
            ioprio: be(n),
            slice_ns: if n < 0 { BOOST_SLICE_NS } else { 0 },
        },
        SchedPolicy::IdPrio(_) => Linux {
            idle: true,
            nice: 19,
            ioprio: IOPRIO_CLASS_IDLE << IOPRIO_CLASS_SHIFT,
            slice_ns: 0,
        },
    }
}

/// WolKernel hint interface (README.wolos.md in the kernel tree); absent on a stock kernel: no-op.
/// `foreground_pid` feeds the frontmost-app CPU floor, timer slack and the i915 waitboost.
pub const HINTS: &str = "/sys/kernel/wolos";

/// Session policy wold sets at start: the frontmost app runs at least at half capacity with exact
/// timers (Wine/Proton frame limiters wake on time); background threads coalesce wakeups by 50 ms.
pub const HINT_DEFAULTS: [(&str, i64); 3] = [
    ("foreground_uclamp_min", 512),
    ("foreground_timerslack_ns", 0),
    ("background_timerslack_ns", 50_000_000),
];

/// Write one hint (an existing file only; sysfs never creates). True when written.
pub fn hint(dir: &Path, name: &str, value: i64) -> bool {
    use std::io::Write;
    let mut buf = [0u8; 24];
    let mut w = &mut buf[..];
    if write!(w, "{value}").is_err() {
        return false;
    }
    let n = 24 - w.len();
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.join(name))
        .and_then(|mut f| f.write_all(&buf[..n]))
        .is_ok()
}

/// Apply [`HINT_DEFAULTS`]; returns how many the kernel took.
pub fn hint_defaults(dir: &Path) -> usize {
    HINT_DEFAULTS
        .iter()
        .filter(|(f, v)| hint(dir, f, *v))
        .count()
}

/// Real policy: every thread of the process (Linux priorities are per thread).
pub struct Sched;

impl Applier for Sched {
    fn apply(&self, pid: i32, class: QosClass) -> bool {
        if pid <= 0 {
            return false;
        }
        let l = linux(class);
        let mut any = false;
        let ok = for_each_thread(pid, |tid| {
            any = true;
            set_thread(tid, l)
        });
        let oom = oom_adj(class).is_none_or(|adj| set_oom_adj(pid, adj));
        ok && any && oom
    }

    fn gpu_foreground(&self, pid: i32) {
        hint(Path::new(HINTS), "foreground_pid", pid.into());
    }
}

/// Per process (not per thread). Stack buffers only: no allocation.
fn set_oom_adj(pid: i32, adj: i16) -> bool {
    use std::io::Write;
    let (mut path, mut val) = ([0u8; 40], [0u8; 8]);
    let mut p = &mut path[..];
    let mut v = &mut val[..];
    if write!(p, "/proc/{pid}/oom_score_adj").is_err() || write!(v, "{adj}").is_err() {
        return false;
    }
    let (pn, vn) = (40 - p.len(), 8 - v.len());
    let Ok(path) = std::str::from_utf8(&path[..pn]) else {
        return false;
    };
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|mut f| f.write_all(&val[..vn]))
        .is_ok()
}

/// `struct sched_attr`, first version; the kernel leaves the uclamp fields alone.
#[repr(C)]
#[derive(Default)]
struct SchedAttr {
    size: u32,
    policy: u32,
    flags: u64,
    nice: i32,
    priority: u32,
    runtime: u64,
    deadline: u64,
    period: u64,
}

fn set_thread(tid: i32, l: Linux) -> bool {
    let attr = SchedAttr {
        size: size_of::<SchedAttr>() as u32,
        policy: if l.idle {
            libc::SCHED_IDLE
        } else {
            libc::SCHED_OTHER
        } as u32,
        nice: l.nice.into(),
        runtime: l.slice_ns,
        ..SchedAttr::default()
    };
    // SAFETY: plain syscalls with integer arguments and a valid sched_attr. setpriority stays for
    // SCHED_IDLE, which ignores sched_nice.
    unsafe {
        libc::syscall(libc::SYS_sched_setattr, tid, &raw const attr, 0u32) == 0
            && libc::setpriority(libc::PRIO_PROCESS, tid as _, i32::from(l.nice)) == 0
            && libc::syscall(
                libc::SYS_ioprio_set,
                1, /* IOPRIO_WHO_PROCESS */
                tid,
                l.ioprio,
            ) == 0
    }
}

/// Calls `f` for every thread id in /proc/<pid>/task; false when the directory cannot be read or
/// any call failed. Does not allocate (opendir/readdir with a stack path).
fn for_each_thread(pid: i32, mut f: impl FnMut(i32) -> bool) -> bool {
    use std::io::Write;
    let mut path = [0u8; 32];
    let mut w = &mut path[..];
    if write!(w, "/proc/{pid}/task\0").is_err() {
        return false;
    }
    // SAFETY: path is NUL-terminated; the DIR is closed below; dirent names are NUL-terminated.
    unsafe {
        let dir = libc::opendir(path.as_ptr().cast());
        if dir.is_null() {
            return false;
        }
        let mut ok = true;
        loop {
            let e = libc::readdir(dir);
            if e.is_null() {
                break;
            }
            let name = std::ffi::CStr::from_ptr((*e).d_name.as_ptr()).to_bytes();
            if let Some(tid) = std::str::from_utf8(name)
                .ok()
                .and_then(|s| s.parse::<i32>().ok())
            {
                ok &= f(tid);
            }
        }
        libc::closedir(dir);
        ok
    }
}

/// Apply one foreground switch: previous app -> Default, new one -> UserInteractive.
pub fn foreground(tracker: &mut ForegroundTracker, pid: i32, applier: &dyn Applier) {
    let (demoted, promoted) = tracker.switch(pid);
    if demoted > 0 && !applier.apply(demoted, QosClass::Default) {
        eprintln!("wold: qos pid {demoted} -> default failed");
    }
    if promoted > 0 {
        if !applier.apply(promoted, QosClass::UserInteractive) {
            eprintln!("wold: qos pid {promoted} -> user-interactive failed");
        }
        applier.gpu_foreground(promoted);
    }
}

/// Serve one connection: `foreground <pid>` lines until EOF. One fixed buffer; no allocation.
pub fn serve(
    mut conn: impl Read,
    tracker: &std::sync::Mutex<ForegroundTracker>,
    applier: &dyn Applier,
) {
    let mut buf = [0u8; 256];
    let mut filled = 0;
    loop {
        let n = match conn.read(&mut buf[filled..]) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        filled += n;
        while let Some(nl) = buf[..filled].iter().position(|b| *b == b'\n') {
            if let Some(pid) = parse_foreground(&buf[..nl]) {
                let mut t = tracker.lock().unwrap_or_else(|p| p.into_inner());
                foreground(&mut t, pid, applier);
            }
            buf.copy_within(nl + 1..filled, 0);
            filled -= nl + 1;
        }
        if filled == buf.len() {
            filled = 0; // overlong line: drop it
        }
    }
}

/// OOM bias per class (`/proc/<pid>/oom_score_adj`): Background work dies first. Raising needs no
/// privilege. None: leave the process's value alone.
pub fn oom_adj(class: QosClass) -> Option<i16> {
    (class == QosClass::Background).then_some(300)
}

/// The numeric pid directories under `proc_dir` whose `comm` is one of `comms`.
fn procs_named<'a>(
    proc_dir: &Path,
    comms: &'a [&[u8]],
) -> impl Iterator<Item = std::fs::DirEntry> + 'a {
    std::fs::read_dir(proc_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.parse::<u32>().is_ok())
        })
        .filter(move |e| {
            std::fs::read(e.path().join("comm"))
                .is_ok_and(|c| comms.contains(&c.trim_ascii_end()))
        })
}

/// Session processes the OOM killer must spare (a dead compositor ends the whole session).
pub const SESSION_COMMS: [&[u8]; 2] = [b"Hyprland", b"wol-shell"];
pub const SESSION_OOM_ADJ: i16 = -900;

/// Give every [`SESSION_COMMS`] process under `proc_dir` [`SESSION_OOM_ADJ`]; idempotent (only
/// writes values that differ). Returns how many were written.
pub fn protect_session(proc_dir: &Path) -> usize {
    let want = SESSION_OOM_ADJ.to_string();
    procs_named(proc_dir, &SESSION_COMMS)
        .filter(|e| {
            let adj = e.path().join("oom_score_adj");
            std::fs::read_to_string(&adj).is_ok_and(|v| v.trim() != want)
                && std::fs::write(&adj, &want).is_ok()
        })
        .count()
}

/// Shader precompilers, as the kernel truncates them in `comm`.
pub const PRECOMPILE_COMMS: [&[u8]; 1] = [b"fossilize_repla"];

/// Put every [`PRECOMPILE_COMMS`] process under `proc_dir` in [`QosClass::Background`]. Re-applying
/// is harmless and threads inherit it. Returns how many were tagged.
pub fn tag_precompile(proc_dir: &Path, applier: &dyn Applier) -> usize {
    procs_named(proc_dir, &PRECOMPILE_COMMS)
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| applier.apply(*pid, QosClass::Background))
        .count()
}

/// Only the realtime agent may boost (canonical executable path of the peer).
pub const REALTIME_EXE: &str = "/usr/bin/wol-realtime";

pub fn peer_allowed(peer_exe: &[u8], allowed: &str) -> bool {
    peer_exe == allowed.as_bytes()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Recorder(pub Mutex<Vec<(i32, QosClass)>>);

    impl Applier for Recorder {
        fn apply(&self, pid: i32, class: QosClass) -> bool {
            if let Ok(mut v) = self.0.try_lock()
                && v.len() < v.capacity()
            {
                v.push((pid, class));
            }
            true
        }
    }

    #[test]
    fn serve_applies_switches_in_order() {
        let r = Recorder(Mutex::new(Vec::with_capacity(16)));
        let t = Mutex::new(ForegroundTracker::default());
        let input: &[u8] = b"foreground 10\nforeground 10\ngarbage\nforeground 20\nforeground 3";
        serve(input, &t, &r);
        use QosClass::*;
        assert_eq!(
            *r.0.lock().unwrap(),
            vec![(10, UserInteractive), (10, Default), (20, UserInteractive)]
        );
    }

    #[derive(Default)]
    struct GpuRecorder(Mutex<Vec<i32>>);
    impl Applier for GpuRecorder {
        fn apply(&self, _: i32, _: QosClass) -> bool {
            true
        }
        fn gpu_foreground(&self, pid: i32) {
            self.0.lock().unwrap().push(pid);
        }
    }

    #[test]
    fn every_switch_tells_the_gpu_driver_the_frontmost_pid() {
        let r = GpuRecorder::default();
        let t = Mutex::new(ForegroundTracker::default());
        serve(&b"foreground 10\nforeground 20\nforeground 0\n"[..], &t, &r);
        assert_eq!(*r.0.lock().unwrap(), vec![10, 20]);
    }

    #[test]
    fn kernel_hints_name_the_frontmost_app_and_set_the_session_policy() {
        let dir = std::env::temp_dir().join(format!("wol-hints-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (f, _) in HINT_DEFAULTS {
            std::fs::write(dir.join(f), "").unwrap();
        }
        std::fs::write(dir.join("foreground_pid"), "").unwrap();
        assert_eq!(hint_defaults(&dir), HINT_DEFAULTS.len());
        assert_eq!(
            std::fs::read_to_string(dir.join("foreground_uclamp_min")).unwrap(),
            "512"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("foreground_timerslack_ns")).unwrap(),
            "0"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("background_timerslack_ns")).unwrap(),
            "50000000"
        );
        assert!(hint(&dir, "foreground_pid", 4242));
        assert_eq!(
            std::fs::read_to_string(dir.join("foreground_pid")).unwrap(),
            "4242"
        );
        // A stock kernel has no /sys/kernel/wolos: nothing is written, nothing fails.
        let none = dir.join("absent");
        assert_eq!(hint_defaults(&none), 0);
        assert!(!hint(&none, "foreground_pid", 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overlong_lines_are_dropped() {
        let r = Recorder(Mutex::new(Vec::with_capacity(4)));
        let t = Mutex::new(ForegroundTracker::default());
        let mut input = vec![b'x'; 300];
        input.extend_from_slice(b"\nforeground 7\n");
        serve(&input[..], &t, &r);
        assert_eq!(*r.0.lock().unwrap(), vec![(7, QosClass::UserInteractive)]);
    }

    #[test]
    fn linux_class_table() {
        use QosClass::*;
        let be = |level: i32| (2 << 13) | level;
        let ms = 1_000_000;
        for (c, idle, nice, io, slice_ns) in [
            (UserInteractive, false, -15, be(1), 2 * ms),
            (UserInitiated, false, -10, be(2), 2 * ms),
            (Default, false, 0, be(4), 0),
            (Utility, false, 10, be(6), 0),
            (Background, true, 19, 3 << 13, 0),
        ] {
            assert_eq!(
                linux(c),
                Linux {
                    idle,
                    nice,
                    ioprio: io,
                    slice_ns
                },
                "{c:?}"
            );
        }
    }

    /// Lowering a child needs no privilege: every thread gets the nice value, Background is
    /// SCHED_IDLE. A missing pid fails.
    #[test]
    fn sched_lowers_a_real_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let utility = Sched.apply(pid, QosClass::Utility);
        // SAFETY: plain syscalls.
        let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as _) };
        let background = Sched.apply(pid, QosClass::Background);
        let policy = unsafe { libc::sched_getscheduler(pid) };
        let oom = std::fs::read_to_string(format!("/proc/{pid}/oom_score_adj")).unwrap_or_default();
        child.kill().ok();
        child.wait().ok();
        assert!(utility && nice == 10, "utility={utility} nice={nice}");
        assert!(background && policy == libc::SCHED_IDLE, "policy={policy}");
        assert_eq!(oom.trim(), "300", "Background raises oom_score_adj");
        assert!(!Sched.apply(i32::MAX, QosClass::Utility));
        assert!(!Sched.apply(0, QosClass::Utility));
    }

    /// The slice reaches EEVDF and a class without one drops it. A kernel before 6.12 ignores it.
    #[test]
    fn slice_is_set_and_cleared_on_a_real_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let quiet = linux(QosClass::Default);
        let default = runtime_of(pid);
        let set = set_thread(
            pid,
            Linux {
                slice_ns: 2_000_000,
                ..quiet
            },
        );
        let with = runtime_of(pid);
        let cleared = set_thread(pid, quiet);
        let without = runtime_of(pid);
        child.kill().ok();
        child.wait().ok();
        assert!(set && cleared);
        assert_eq!((with, without), (2_000_000, default));
    }

    fn runtime_of(tid: i32) -> u64 {
        let mut a = SchedAttr::default();
        // SAFETY: a is a valid buffer of the size passed.
        let r = unsafe {
            libc::syscall(
                libc::SYS_sched_getattr,
                tid,
                &raw mut a,
                size_of::<SchedAttr>() as u32,
                0u32,
            )
        };
        assert_eq!(r, 0);
        a.runtime
    }

    #[test]
    fn background_is_first_for_the_oom_killer() {
        use QosClass::*;
        assert_eq!(oom_adj(Background), Some(300));
        for c in [UserInteractive, UserInitiated, Default, Utility] {
            assert_eq!(oom_adj(c), None, "{c:?}");
        }
    }

    /// Fake /proc: compositor and shell get -900 once; other processes are untouched.
    #[test]
    fn session_is_protected_from_the_oom_killer() {
        let d = std::env::temp_dir().join(format!("wol-qos-oom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (pid, comm, adj) in [
            ("10", "Hyprland\n", "0\n"),
            ("11", "wol-shell\n", "0\n"),
            ("12", "helium\n", "0\n"),
            ("13", "Hyprland\n", "-900\n"),
            ("self", "Hyprland\n", "0\n"),
        ] {
            std::fs::create_dir_all(d.join(pid)).unwrap();
            std::fs::write(d.join(pid).join("comm"), comm).unwrap();
            std::fs::write(d.join(pid).join("oom_score_adj"), adj).unwrap();
        }
        let adj = |pid: &str| std::fs::read_to_string(d.join(pid).join("oom_score_adj")).unwrap();
        assert_eq!(protect_session(&d), 2);
        assert_eq!(adj("10").trim(), "-900");
        assert_eq!(adj("11").trim(), "-900");
        assert_eq!(adj("12").trim(), "0");
        assert_eq!(adj("self").trim(), "0", "only numeric pid dirs");
        assert_eq!(protect_session(&d), 0, "idempotent");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn shader_precompile_runs_in_the_background() {
        let d = std::env::temp_dir().join(format!("wol-qos-foz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (pid, comm) in [
            ("20", "fossilize_repla\n"),
            ("21", "fossilize_repla\n"),
            ("22", "cs2\n"),
            ("self", "fossilize_repla\n"),
        ] {
            std::fs::create_dir_all(d.join(pid)).unwrap();
            std::fs::write(d.join(pid).join("comm"), comm).unwrap();
        }
        let r = Recorder(Mutex::new(Vec::with_capacity(8)));
        assert_eq!(tag_precompile(&d, &r), 2);
        let mut got = r.0.lock().unwrap().clone();
        got.sort();
        assert_eq!(
            got,
            [(20, QosClass::Background), (21, QosClass::Background)]
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn peer_check() {
        assert!(peer_allowed(b"/usr/bin/wol-realtime", REALTIME_EXE));
        assert!(!peer_allowed(b"/tmp/wol-realtime", REALTIME_EXE));
    }

    #[test]
    fn hot_path_is_fast() {
        let r = Recorder(Mutex::new(Vec::new())); // capacity 0: records nothing, never allocates
        let t = Mutex::new(ForegroundTracker::default());
        let mut input = Vec::new();
        for i in 0..10_000 {
            input.extend_from_slice(format!("foreground {}\n", 100 + i % 2).as_bytes());
        }
        let start = std::time::Instant::now();
        serve(&input[..], &t, &r);
        assert!(
            start.elapsed() < std::time::Duration::from_millis(50),
            "{:?}",
            start.elapsed()
        );
    }
}
