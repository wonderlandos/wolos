//! process_madvise over a process's anonymous and memfd mappings. MADV_WILLNEED brings a restored
//! app's compressed pages back before it touches them: the kernel reads every swapped page of those
//! ranges into the swap cache, so the first frames take minor faults instead of one zram decompress
//! per fault. MADV_PAGEOUT sends a background app's pages the other way.
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Writable private anonymous ranges (heap, stacks, `[anon:...]`, JS heaps) and memfd/shm
/// mappings from /proc/<pid>/maps text.
pub fn ranges(maps: &str) -> Vec<(usize, usize)> {
    maps.lines()
        .filter_map(|l| {
            let mut f = l.split_ascii_whitespace();
            let (range, perms) = (f.next()?, f.next()?);
            let (_off, _dev, inode) = (f.next()?, f.next()?, f.next()?);
            let path = f.collect::<Vec<_>>().join(" ");
            let anon = inode == "0"
                && perms.contains('w')
                && (path.is_empty()
                    || path == "[heap]"
                    || path.starts_with("[stack")
                    || path.starts_with("[anon:"));
            let shm = path.starts_with("/memfd:") || path.starts_with("/dev/shm/");
            if !(anon || shm) {
                return None;
            }
            let (a, b) = range.split_once('-')?;
            let (a, b) = (
                usize::from_str_radix(a, 16).ok()?,
                usize::from_str_radix(b, 16).ok()?,
            );
            (b > a).then_some((a, b - a))
        })
        .collect()
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

/// `advice` every range of `pid` until `stop`. Returns the bytes advised (0 when the process is
/// gone or the kernel refuses). Runs on a worker thread: a large app takes a while.
fn advise(proc: &Path, pid: i32, advice: i32, stop: &AtomicBool) -> usize {
    let Ok(maps) = std::fs::read_to_string(proc.join(pid.to_string()).join("maps")) else {
        return 0;
    };
    let r = split(&ranges(&maps));
    // SAFETY: pidfd_open takes a pid and flags; the fd is closed below.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
    if fd < 0 {
        return 0;
    }
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
                advice,
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

/// Bring `pid`'s compressed pages back ahead of use.
pub fn prefetch(proc: &Path, pid: i32) -> usize {
    advise(proc, pid, libc::MADV_WILLNEED, &AtomicBool::new(false))
}

/// Push `pid`'s pages out: this works wherever they were charged, which memory.reclaim on the app's
/// scope cannot do for pages touched before the app was adopted (charges do not move with a
/// process). Only pages this process alone maps go.
pub fn pageout(proc: &Path, pid: i32, stop: &AtomicBool) -> usize {
    advise(proc, pid, libc::MADV_PAGEOUT, stop)
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
    fn picks_anonymous_and_shared_memory_ranges() {
        let maps = "\
55d0a0000000-55d0a0100000 r-xp 00000000 08:01 1234 /opt/code/code
55d0a1000000-55d0a2000000 rw-p 00000000 00:00 0 [heap]
7f0000000000-7f0000400000 rw-p 00000000 00:00 0
7f0000400000-7f0000500000 r--p 00000000 00:00 0
7f1000000000-7f1000200000 rw-s 00000000 00:01 77 /memfd:shared_mem (deleted)
7f2000000000-7f2000001000 rw-p 00000000 00:00 0 [anon:v8]
7ffc00000000-7ffc00021000 rw-p 00000000 00:00 0 [stack]
7ffc10000000-7ffc10002000 r--p 00000000 00:00 0 [vvar]
";
        assert_eq!(
            ranges(maps),
            vec![
                (0x55d0a1000000, 0x1000000),
                (0x7f0000000000, 0x400000),
                (0x7f1000000000, 0x200000),
                (0x7f2000000000, 0x1000),
                (0x7ffc00000000, 0x21000),
            ]
        );
        assert!(ranges("garbage\n").is_empty());
    }

    /// A real child: the call goes through (a fresh process has nothing swapped, so nothing
    /// changes for it) and a missing pid is 0.
    #[test]
    fn advises_a_real_child() {
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let n = prefetch(Path::new("/proc"), child.id() as i32);
        child.kill().ok();
        child.wait().ok();
        // Kernels before 5.10 lack process_madvise; WILLNEED needs 6.0+. Either way: no crash.
        let _ = n;
        assert_eq!(prefetch(Path::new("/proc"), i32::MAX), 0);
    }
}
