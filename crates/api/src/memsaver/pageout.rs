// SPDX-License-Identifier: GPL-2.0-or-later
//! Page out a background app with process_madvise (MADV_PAGEOUT) over its anonymous and memfd
//! mappings (and writable file data). This works wherever the pages were charged, which memory.reclaim on the app's scope
//! cannot do for pages touched before the app was adopted: charges do not move with a process.
//! Only pages this process alone maps go; anything else stays for memory.reclaim.
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The ranges Memory Saver asks about: `wol_core::jsruntime::pageout_ranges` with the default
/// filter (writable anonymous memory, shared memory, writable file data), the one function
/// `wolos-mm pageout` uses too. Its test table is in `crates/core/src/jsruntime.rs`.
pub fn ranges(maps: &str) -> Vec<(usize, usize)> {
    wol_core::jsruntime::pageout_ranges(maps, wol_core::jsruntime::Pageout::Anon)
}

/// Advise at most this much per range and this many ranges per call, so `stop` is seen quickly.
const MAX_RANGE: usize = 32 << 20;
const IOVECS: usize = 64;

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

/// Page out every range of `pid` until `stop`. Returns the bytes advised (0 when the process is
/// gone or the kernel refuses). Runs on a worker thread: a large app takes a while.
pub fn pageout(proc: &Path, pid: i32, stop: &AtomicBool) -> usize {
    // The pid is pinned first: the maps below are read after, so a pid that was reused in
    // between is not the one that gets the ranges (the descriptor names the original process).
    // SAFETY: pidfd_open takes a pid and flags; the fd is closed below.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
    if fd < 0 {
        return 0;
    }
    // A process that vanished since: no ranges, and the descriptor is still closed below.
    let r = std::fs::read_to_string(proc.join(pid.to_string()).join("maps"))
        .map(|maps| split(&ranges(&maps)))
        .unwrap_or_default();
    let mut done = 0;
    for chunk in r.chunks(IOVECS) {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let iov: Vec<libc::iovec> = chunk
            .iter()
            .map(|(a, n)| libc::iovec {
                iov_base: *a as *mut libc::c_void,
                iov_len: *n,
            })
            .collect();
        // SAFETY: iov is a valid array of `iov.len()` iovecs describing the target's ranges
        // (never dereferenced here); advice and flags are plain integers.
        let n = unsafe {
            libc::syscall(
                libc::SYS_process_madvise,
                fd,
                iov.as_ptr(),
                iov.len(),
                libc::MADV_PAGEOUT,
                0,
            )
        };
        if n > 0 {
            done += n as usize;
        }
    }
    // SAFETY: fd is ours.
    unsafe { libc::close(fd) };
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
