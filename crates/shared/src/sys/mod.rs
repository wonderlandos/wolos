//! Low-level OS access (Linux + FreeBSD). Ported from IaraOS iara-sys.
//! Safe wrappers only; no allocation on hot paths.

use std::os::fd::AsRawFd;

/// Credentials of the process on the other end of a connected Unix socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
}

/// Linux SO_PEERCRED, FreeBSD LOCAL_PEERCRED. None if the OS cannot tell.
pub fn peer(stream: &impl AsRawFd) -> Option<Peer> {
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "linux")]
    // SAFETY: getsockopt writes at most `len` bytes into a zeroed, correctly sized ucred.
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        (libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast(),
            &mut len,
        ) == 0)
            .then_some(Peer {
                pid: cred.pid,
                uid: cred.uid,
            })
    }
    #[cfg(target_os = "freebsd")]
    // SAFETY: as above, with struct xucred.
    unsafe {
        let mut cred: libc::xucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::xucred>() as libc::socklen_t;
        (libc::getsockopt(
            fd,
            0, /* SOL_LOCAL */
            libc::LOCAL_PEERCRED,
            (&raw mut cred).cast(),
            &mut len,
        ) == 0)
            .then_some(Peer {
                pid: cred.cr_pid__c_anonymous_union.cr_pid,
                uid: cred.cr_uid,
            })
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = fd;
        None
    }
}

/// Executable path of `pid` as bytes written into `buf`; returns the length. Does not allocate.
pub fn executable(pid: i32, buf: &mut [u8]) -> Option<usize> {
    if pid <= 0 || buf.is_empty() {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        let mut path = [0u8; 32];
        let mut w = &mut path[..];
        use std::io::Write;
        write!(w, "/proc/{pid}/exe\0").ok()?;
        // SAFETY: path is NUL-terminated; readlink writes at most buf.len() bytes.
        let n = unsafe { libc::readlink(path.as_ptr().cast(), buf.as_mut_ptr().cast(), buf.len()) };
        (n > 0 && (n as usize) < buf.len()).then_some(n as usize)
    }
    #[cfg(target_os = "freebsd")]
    {
        let n = sysctl_proc(libc::KERN_PROC_PATHNAME, pid, buf)?;
        Some(if n > 0 && buf[n - 1] == 0 { n - 1 } else { n })
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        None
    }
}

/// argv of `pid`. Allocates: use once per client, not per frame.
pub fn args(pid: i32) -> Option<Vec<String>> {
    if pid <= 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    #[cfg(target_os = "freebsd")]
    let raw = {
        let mut buf = vec![0u8; 64 * 1024];
        let n = sysctl_proc(libc::KERN_PROC_ARGS, pid, &mut buf)?;
        buf.truncate(n);
        buf
    };
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let raw: Vec<u8> = return None;
    Some(
        raw.split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect(),
    )
}

/// kern.proc.<what>.<pid> into `buf`; returns bytes written.
#[cfg(target_os = "freebsd")]
fn sysctl_proc(what: libc::c_int, pid: i32, buf: &mut [u8]) -> Option<usize> {
    let mib = [libc::CTL_KERN, libc::KERN_PROC, what, pid];
    let mut len: libc::size_t = buf.len();
    // SAFETY: sysctl writes at most `len` bytes into buf.
    let ok = unsafe {
        libc::sysctl(
            mib.as_ptr(),
            4,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null(),
            0,
        )
    } == 0;
    ok.then_some(len)
}

/// CLOCK_MONOTONIC in nanoseconds. Does not allocate.
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid clock id and out pointer.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Canonical path (symlinks and `..` resolved), or the input unchanged if it does not exist.
pub fn canonical(path: &str) -> String {
    std::fs::canonicalize(path)
        .map_or_else(|_| path.to_owned(), |p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn peer_of_socketpair_is_this_process() {
        let (a, _b) = UnixStream::pair().unwrap();
        let p = peer(&a).unwrap();
        assert_eq!(p.pid, std::process::id() as i32);
        assert_eq!(p.uid, unsafe { libc::getuid() });
    }

    #[test]
    fn executable_of_self_matches_current_exe() {
        let mut buf = [0u8; 1024];
        let n = executable(std::process::id() as i32, &mut buf).unwrap();
        let want = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(
            std::str::from_utf8(&buf[..n]).unwrap(),
            want.to_str().unwrap()
        );
    }

    #[test]
    fn missing_or_invalid_pid_has_no_executable() {
        let mut buf = [0u8; 64];
        for pid in [0, -1, i32::MAX] {
            assert_eq!(executable(pid, &mut buf), None, "pid {pid}");
        }
    }

    #[test]
    fn args_of_self_start_with_argv0() {
        let args = args(std::process::id() as i32).unwrap();
        assert_eq!(
            args.first().map(String::as_str),
            std::env::args().next().as_deref()
        );
    }

    #[test]
    fn monotonic_ns_never_goes_back() {
        let a = monotonic_ns();
        let b = monotonic_ns();
        assert!(b >= a && a > 0);
    }

    #[test]
    fn canonical_resolves_dotdot_and_keeps_missing_paths() {
        assert_eq!(canonical("/usr/../usr"), "/usr");
        assert_eq!(canonical("/does/not/exist"), "/does/not/exist");
    }
}
