//! QoS service: applies `shared::qos::QosClass` policy; frontmost app boosting from wol-realtime.
use shared::qos::{ForegroundTracker, QosClass, SchedPolicy, parse_foreground};
use std::io::Read;

/// Sets a process's scheduling class. Implementations must not allocate on success.
pub trait Applier: Send + Sync {
    fn apply(&self, pid: i32, class: QosClass) -> bool;
    /// Tell the GPU driver the frontmost pid (i915 foreground-only waitboost, ADR 0008). Best effort.
    fn gpu_foreground(&self, _pid: i32) {}
}

/// Real policy: `wolos_qos` kmod (all threads) when loaded, else rtprio(2) / setpriority(2)
/// on FreeBSD; setpriority only elsewhere.
pub struct Sched;

impl Applier for Sched {
    fn apply(&self, pid: i32, class: QosClass) -> bool {
        if pid <= 0 {
            return false;
        }
        #[cfg(target_os = "freebsd")]
        if let Some(ok) = kmod::apply(pid, class) {
            return ok;
        }
        match class.policy() {
            SchedPolicy::Nice(n) => set_nice(pid, n) && set_rt(pid, RtKind::Normal, 0),
            SchedPolicy::RtPrio(p) => set_rt(pid, RtKind::Realtime, p),
            SchedPolicy::IdPrio(p) => set_rt(pid, RtKind::Idle, p),
        }
    }

    #[cfg(target_os = "freebsd")]
    fn gpu_foreground(&self, pid: i32) {
        // Absent unless the WolOS i915kms (kernel/drm) is loaded: then a no-op.
        let v: libc::c_int = pid;
        // SAFETY: sysctlbyname reads size_of::<c_int>() bytes from v; no output buffer.
        unsafe {
            libc::sysctlbyname(
                c"compat.linuxkpi.i915_wol_fg_pid".as_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                (&raw const v).cast(),
                size_of::<libc::c_int>(),
            )
        };
    }
}

#[allow(dead_code)] // variants used on FreeBSD only
enum RtKind {
    Realtime,
    Normal,
    Idle,
}

fn set_nice(pid: i32, nice: i8) -> bool {
    // SAFETY: plain syscall with integer arguments.
    unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as _, i32::from(nice)) == 0 }
}

#[cfg(target_os = "freebsd")]
fn set_rt(pid: i32, kind: RtKind, prio: u8) -> bool {
    let rt = libc::rtprio {
        type_: match kind {
            RtKind::Realtime => libc::RTP_PRIO_REALTIME,
            RtKind::Normal => libc::RTP_PRIO_NORMAL,
            RtKind::Idle => libc::RTP_PRIO_IDLE,
        },
        prio: u16::from(prio),
    };
    let mut rt = rt;
    // SAFETY: rtprio(2) reads a valid struct.
    unsafe { libc::rtprio(libc::RTP_SET, pid, &mut rt) == 0 }
}

/// `wolos_qos_set(tid, class)` syscall (kernel/wolos_qos). Number from `kern.wolos_qos.syscall`.
#[cfg(target_os = "freebsd")]
mod kmod {
    use shared::qos::QosClass;
    use std::sync::OnceLock;

    /// Thread capacity per call; larger processes fall back to the per-process path.
    const MAX_THREADS: usize = 64;

    fn syscall_no() -> Option<libc::c_int> {
        static NO: OnceLock<Option<libc::c_int>> = OnceLock::new();
        *NO.get_or_init(|| {
            let mut v: libc::c_int = -1;
            let mut len = size_of::<libc::c_int>();
            // SAFETY: sysctlbyname writes at most len bytes into v.
            let r = unsafe {
                libc::sysctlbyname(
                    c"kern.wolos_qos.syscall".as_ptr(),
                    (&raw mut v).cast(),
                    &mut len,
                    std::ptr::null(),
                    0,
                )
            };
            (r == 0 && v > 0).then_some(v)
        })
    }

    /// None: kmod not loaded or too many threads (caller falls back). Some(ok) otherwise.
    pub fn apply(pid: i32, class: QosClass) -> Option<bool> {
        let no = syscall_no()?;
        // SAFETY: kinfo_proc is plain old data; zeroed is a valid value.
        let mut kp: [libc::kinfo_proc; MAX_THREADS] = unsafe { std::mem::zeroed() };
        let mut len = size_of_val(&kp);
        let mib = [
            libc::CTL_KERN,
            libc::KERN_PROC,
            libc::KERN_PROC_PID | libc::KERN_PROC_INC_THREAD,
            pid,
        ];
        // SAFETY: sysctl writes at most len bytes into kp.
        let r = unsafe {
            libc::sysctl(
                mib.as_ptr(),
                mib.len() as u32,
                kp.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null(),
                0,
            )
        };
        if r != 0 {
            return None;
        }
        let n = len / size_of::<libc::kinfo_proc>();
        if n == 0 {
            return Some(false);
        }
        let mut ok = true;
        for k in &kp[..n] {
            // SAFETY: plain syscall with integer arguments.
            ok &= unsafe { libc::syscall(no, k.ki_tid, libc::c_int::from(class as u8)) } == 0;
        }
        Some(ok)
    }
}

/// Not FreeBSD: realtime/idle classes have no portable equivalent; normal is a no-op.
#[cfg(not(target_os = "freebsd"))]
fn set_rt(_pid: i32, kind: RtKind, _prio: u8) -> bool {
    matches!(kind, RtKind::Normal)
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

/// Only the realtime agent may boost (canonical executable path of the peer).
pub const REALTIME_EXE: &str = "/usr/local/bin/wol-realtime";

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
    fn overlong_lines_are_dropped() {
        let r = Recorder(Mutex::new(Vec::with_capacity(4)));
        let t = Mutex::new(ForegroundTracker::default());
        let mut input = vec![b'x'; 300];
        input.extend_from_slice(b"\nforeground 7\n");
        serve(&input[..], &t, &r);
        assert_eq!(*r.0.lock().unwrap(), vec![(7, QosClass::UserInteractive)]);
    }

    /// Needs `kldload wolos_qos`; skipped otherwise.
    #[cfg(target_os = "freebsd")]
    #[test]
    fn kmod_sets_child_class() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let r = super::kmod::apply(pid, QosClass::Utility);
        // SAFETY: plain syscall.
        let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as _) };
        child.kill().ok();
        child.wait().ok();
        match r {
            None => eprintln!("wolos_qos not loaded: skipped"),
            Some(ok) => assert!(ok && nice == 10, "ok={ok} nice={nice}"),
        }
    }

    #[test]
    fn peer_check() {
        assert!(peer_allowed(b"/usr/local/bin/wol-realtime", REALTIME_EXE));
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
