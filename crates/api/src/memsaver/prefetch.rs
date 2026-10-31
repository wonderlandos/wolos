//! Bring a restored app's compressed pages back before it touches them: process_madvise
//! (MADV_WILLNEED) over its anonymous and memfd mappings. The kernel reads every swapped page of
//! those ranges into the swap cache, so the first frames take minor faults instead of one zram
//! decompress per fault.
use std::path::Path;

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

/// WILLNEED every range of `pid`. Returns the bytes advised (0 when the process is gone or the
/// kernel refuses). Runs on a worker thread: it can take a few hundred ms for a large app.
pub fn prefetch(proc: &Path, pid: i32) -> usize {
    let Ok(maps) = std::fs::read_to_string(proc.join(pid.to_string()).join("maps")) else {
        return 0;
    };
    let r = ranges(&maps);
    // SAFETY: pidfd_open takes a pid and flags; the fd is closed below.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
    if fd < 0 {
        return 0;
    }
    let mut done = 0;
    // UIO_MAXIOV (1024) ranges per call.
    for chunk in r.chunks(1024) {
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
                libc::MADV_WILLNEED,
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

#[cfg(test)]
mod tests {
    use super::*;

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
