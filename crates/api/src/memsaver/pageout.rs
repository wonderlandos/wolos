// SPDX-License-Identifier: GPL-2.0-or-later
//! Page out a background app with process_madvise (MADV_PAGEOUT) over its anonymous and memfd
//! mappings (and writable file data). This works wherever the pages were charged, which memory.reclaim on the app's scope
//! cannot do for pages touched before the app was adopted: charges do not move with a process.
//! Only pages this process alone maps go; anything else stays for memory.reclaim.
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The ranges Memory Saver asks about: `wol_core::jsruntime::pageout_ranges` with the default
/// filter (writable anonymous memory, shared memory, writable file data), the one function
/// `wolos-mm pageout` uses too. Its test table is in `crates/core/src/jsruntime.rs`.
pub fn ranges(maps: &str) -> Vec<(usize, usize)> {
    ranges_of(maps, Pageout::Anon)
}

/// [`ranges`] for either filter: [`Pageout::All`] adds read-only data and code, what the
/// budget's page-out every 30 s behind asks (docs/bench/js-runtime-x.md, run F2).
pub fn ranges_of(maps: &str, what: Pageout) -> Vec<(usize, usize)> {
    wol_core::jsruntime::pageout_ranges(maps, what)
}

pub use wol_core::jsruntime::Pageout;

/// Advise at most this much per call, so `stop` is seen quickly.
const MAX_RANGE: usize = 32 << 20;

fn split(r: &[(usize, usize)]) -> Vec<(usize, usize)> {
    r.iter()
        .flat_map(|&(a, n)| {
            (0..n.div_ceil(MAX_RANGE)).map(move |i| {
                let off = i * MAX_RANGE;
                (a + off, MAX_RANGE.min(n - off))
            })
        })
        .collect()
}

/// A pidfd of `pid` (closed when dropped); None when the process is gone or the kernel has none.
fn pidfd(pid: i32) -> Option<OwnedFd> {
    // SAFETY: pidfd_open takes a pid and flags and returns a new descriptor or -1; a descriptor
    // it returns is owned by nobody else, so the OwnedFd is its one owner.
    unsafe {
        let fd = libc::syscall(libc::SYS_pidfd_open, pid, 0);
        let fd = i32::try_from(fd).ok().filter(|fd| *fd >= 0)?;
        Some(OwnedFd::from_raw_fd(fd))
    }
}

/// Whether the pidfd `fd` still names `pid` (`jsruntime::pidfd_names` over its fdinfo).
fn still_target(fd: &OwnedFd, pid: i32) -> bool {
    u32::try_from(pid).is_ok_and(|p| {
        std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fd.as_raw_fd()))
            .is_ok_and(|s| wol_core::jsruntime::pidfd_names(&s, p))
    })
}

/// Page out the [`Pageout::Anon`] ranges of `pid` until `stop` (Memory Saver's give-back).
pub fn pageout(proc: &Path, pid: i32, stop: &AtomicBool) -> usize {
    pageout_what(proc, pid, stop, Pageout::Anon)
}

/// Page out every range of `pid` the filter `what` picks, until `stop`. Returns the bytes advised
/// (0 when the process is gone or the kernel refuses). One range per call: a call with several
/// stops silently at the first one the kernel refuses (measured: 18 GiB asked, 11 MiB paged out,
/// docs/modules/runtime-tuning.md). Before every call `stop` is read and the pidfd is asked
/// whether it still names the process, so one that exits halfway is left at once. Runs on a
/// worker thread: a large app takes a while.
pub fn pageout_what(proc: &Path, pid: i32, stop: &AtomicBool, what: Pageout) -> usize {
    // The pid is pinned first: the maps below are read after, so a pid that was reused in
    // between is not the one that gets the ranges (the descriptor names the original process).
    let Some(fd) = pidfd(pid) else {
        return 0;
    };
    // A process that vanished since: no ranges.
    let r = std::fs::read_to_string(proc.join(pid.to_string()).join("maps"))
        .map(|maps| split(&ranges_of(&maps, what)))
        .unwrap_or_default();
    let mut done = 0;
    for (a, n) in r {
        if stop.load(Ordering::Relaxed) || !still_target(&fd, pid) {
            break;
        }
        let iov = libc::iovec {
            iov_base: a as *mut libc::c_void,
            iov_len: n,
        };
        // SAFETY: iov is one valid iovec describing a range of the target (never dereferenced
        // here) and lives across the call; advice and flags are plain integers.
        let n = unsafe {
            libc::syscall(
                libc::SYS_process_madvise,
                fd.as_raw_fd(),
                &iov as *const libc::iovec,
                1usize,
                libc::MADV_PAGEOUT,
                0,
            )
        };
        done += usize::try_from(n).unwrap_or(0);
    }
    done
}

/// VmSwap of a process in bytes, 0 when it is gone.
pub fn swapped(proc: &Path, pid: i32) -> u64 {
    std::fs::read_to_string(proc.join(pid.to_string()).join("status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmSwap:"))
                .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
        })
        .map_or(0, |kib| kib << 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_ranges_are_cut_so_a_cancel_is_seen_quickly() {
        let m = MAX_RANGE;
        assert_eq!(split(&[(0, 10)]), [(0, 10)]);
        assert_eq!(split(&[(4096, m)]), [(4096, m)]);
        assert_eq!(
            split(&[(0, 2 * m + 1), (1 << 40, 5)]),
            [(0, m), (m, m), (2 * m, 1), (1 << 40, 5)]
        );
    }

    #[test]
    fn vmswap_is_read_from_status_and_a_gone_process_has_none() {
        let d = std::env::temp_dir().join(format!("wol-swapped-{}", std::process::id()));
        std::fs::create_dir_all(d.join("7")).unwrap();
        std::fs::write(d.join("7/status"), "Name:\tx\nVmSwap:\t    2048 kB\n").unwrap();
        assert_eq!(swapped(&d, 7), 2 << 20);
        assert_eq!(swapped(&d, 8), 0);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn the_ranges_are_the_ones_the_shared_filter_picks() {
        let maps = "\
55d0a0000000-55d0a0100000 r-xp 00000000 08:01 1234 /opt/code/code
7f0000000000-7f0000400000 rw-p 00000000 00:00 0
7f1000000000-7f1000200000 rw-s 00000000 00:01 77 /memfd:shared_mem (deleted)
";
        assert_eq!(
            ranges(maps),
            [(0x7f0000000000, 0x400000), (0x7f1000000000, 0x200000)]
        );
        assert!(ranges("garbage\n").is_empty());
        // The budget's page-out also asks the code.
        assert_eq!(
            ranges_of(maps, Pageout::All),
            [
                (0x55d0a0000000, 0x100000),
                (0x7f0000000000, 0x400000),
                (0x7f1000000000, 0x200000)
            ]
        );
    }

    /// The pidfd of a child names it while it runs and no longer once it is gone: the check made
    /// before every call stops a page-out of a process that exited.
    #[test]
    fn the_pidfd_is_asked_before_every_call() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        let Some(fd) = pidfd(pid) else {
            // A kernel before 5.3: nothing to check.
            child.kill().ok();
            child.wait().ok();
            return;
        };
        assert!(still_target(&fd, pid));
        assert!(!still_target(&fd, pid + 1));
        child.kill().ok();
        child.wait().ok();
        assert!(!still_target(&fd, pid), "gone");
    }

    /// A real child: the call goes through and a missing pid is 0.
    #[test]
    fn pages_out_a_real_child() {
        let stop = AtomicBool::new(false);
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let n = pageout(Path::new("/proc"), child.id() as i32, &stop);
        child.kill().ok();
        child.wait().ok();
        // Kernels before 5.10 lack process_madvise, and it needs privilege: either way, no crash.
        let _ = n;
        assert_eq!(pageout(Path::new("/proc"), i32::MAX, &stop), 0);
    }
}
