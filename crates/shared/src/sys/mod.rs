// SPDX-License-Identifier: BSD-2-Clause
//! Low-level OS access (Linux). Ported from IaraOS iara-sys.
//! Safe wrappers only; no allocation on hot paths.

use std::os::fd::AsRawFd;

/// Credentials of the process on the other end of a connected Unix socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
}

/// SO_PEERCRED. None if the OS cannot tell.
pub fn peer(stream: &impl AsRawFd) -> Option<Peer> {
    let fd = stream.as_raw_fd();
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
}

/// Open flags for a file a person named: a pipe does not block the open, a terminal is not taken.
pub const OPEN_QUIET: i32 = libc::O_NONBLOCK | libc::O_NOCTTY;

/// Open, for reading, a file a caller named (a package to install, a file to check): with
/// [`OPEN_QUIET`] and `O_NOFOLLOW`, so a FIFO does not block, a final symlink is refused, and a
/// terminal never becomes the opener's controlling terminal. A root daemon without one (wold
/// under systemd) would otherwise take a pty the caller named and die of its hang-up (audit
/// S4-2). Check the opened file (`metadata().is_file()`), not the name. Err as `open(2)`.
pub fn open_named(path: impl AsRef<std::path::Path>) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_QUIET | libc::O_NOFOLLOW)
        .open(path)
}

/// Write `bytes` with one file descriptor attached (SCM_RIGHTS).
pub fn send_with_fd(
    sock: &impl AsRawFd,
    bytes: &[u8],
    fd: std::os::fd::BorrowedFd<'_>,
) -> std::io::Result<()> {
    use std::os::fd::AsRawFd as _;
    let mut control = Control::zeroed();
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr().cast_mut().cast(),
        iov_len: bytes.len(),
    };
    // SAFETY: msghdr is plain data, zero is valid.
    let mut m: libc::msghdr = unsafe { std::mem::zeroed() };
    m.msg_iov = &raw mut iov;
    m.msg_iovlen = 1;
    m.msg_control = control.0.as_mut_ptr().cast();
    // SAFETY: CMSG_SPACE only computes.
    m.msg_controllen = unsafe { libc::CMSG_SPACE(4) } as _;
    // SAFETY: `control` is aligned and CMSG_SPACE(4) bytes long, so the first header and its
    // 4-byte payload fit; `iov` and `control` outlive sendmsg, which reads only those.
    let sent = unsafe {
        let c = libc::CMSG_FIRSTHDR(&raw const m);
        (*c).cmsg_level = libc::SOL_SOCKET;
        (*c).cmsg_type = libc::SCM_RIGHTS;
        (*c).cmsg_len = libc::CMSG_LEN(4) as _;
        libc::CMSG_DATA(c)
            .cast::<i32>()
            .write_unaligned(fd.as_raw_fd());
        libc::sendmsg(sock.as_raw_fd(), &raw const m, libc::MSG_NOSIGNAL)
    };
    if sent < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut rest = &bytes[sent as usize..];
    while !rest.is_empty() {
        // SAFETY: `rest` is valid for its length.
        let n = unsafe {
            libc::send(
                sock.as_raw_fd(),
                rest.as_ptr().cast(),
                rest.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        rest = &rest[n as usize..];
    }
    Ok(())
}

/// Room for one descriptor's control message, aligned for `cmsghdr`.
#[repr(align(8))]
struct Control([u8; 64]);

impl Control {
    fn zeroed() -> Self {
        Self([0; 64])
    }
}

/// One read; a descriptor that came with the bytes is returned, close-on-exec. The buffer has room
/// for one, so the kernel closes any more a peer attached.
pub fn recv_with_fd(
    sock: &impl AsRawFd,
    buf: &mut [u8],
) -> std::io::Result<(usize, Option<std::os::fd::OwnedFd>)> {
    use std::os::fd::FromRawFd;
    let mut control = Control::zeroed();
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    // SAFETY: msghdr is plain data, zero is valid.
    let mut m: libc::msghdr = unsafe { std::mem::zeroed() };
    m.msg_iov = &raw mut iov;
    m.msg_iovlen = 1;
    m.msg_control = control.0.as_mut_ptr().cast();
    // SAFETY: CMSG_SPACE only computes.
    m.msg_controllen = unsafe { libc::CMSG_SPACE(4) } as _;
    let n = loop {
        // SAFETY: `iov` and `control` are live and as long as `m` says.
        let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &raw mut m, libc::MSG_CMSG_CLOEXEC) };
        if n >= 0 {
            break n as usize;
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    // SAFETY: the kernel wrote `msg_controllen` bytes of well-formed headers; a rights message of
    // the expected length carries one descriptor this process now owns.
    let fd = unsafe {
        let c = libc::CMSG_FIRSTHDR(&raw const m);
        (!c.is_null()
            && (*c).cmsg_level == libc::SOL_SOCKET
            && (*c).cmsg_type == libc::SCM_RIGHTS
            && (*c).cmsg_len >= libc::CMSG_LEN(4) as _)
            .then(|| {
                std::os::fd::OwnedFd::from_raw_fd(libc::CMSG_DATA(c).cast::<i32>().read_unaligned())
            })
    };
    Ok((n, fd))
}

/// SO_PASSCRED on or off (ADR 0033). While it is on when a peer writes, the kernel attaches the
/// writing process's credentials to those bytes and [`recv_with_pid`] reports them. Err as setsockopt.
pub fn set_passcred(sock: &impl AsRawFd, on: bool) -> std::io::Result<()> {
    let v: libc::c_int = on.into();
    // SAFETY: setsockopt reads one c_int from a live local of the stated size.
    let r = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            (&raw const v).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// One read on a socket with [`set_passcred`] on: (bytes read, pid of the process that wrote
/// them, as the kernel recorded at write time). The pid is None when no credentials came with the
/// bytes (written before SO_PASSCRED was set). Descriptors a peer attached are closed.
pub fn recv_with_pid(sock: &impl AsRawFd, buf: &mut [u8]) -> std::io::Result<(usize, Option<i32>)> {
    use std::os::fd::FromRawFd;
    let mut control = Control::zeroed();
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    // SAFETY: msghdr is plain data, zero is valid.
    let mut m: libc::msghdr = unsafe { std::mem::zeroed() };
    m.msg_iov = &raw mut iov;
    m.msg_iovlen = 1;
    m.msg_control = control.0.as_mut_ptr().cast();
    m.msg_controllen = control.0.len() as _;
    let n = loop {
        // SAFETY: `iov` and `control` are live and as long as `m` says.
        let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &raw mut m, libc::MSG_CMSG_CLOEXEC) };
        if n >= 0 {
            break n as usize;
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    let mut pid = None;
    // SAFETY: the kernel wrote `msg_controllen` bytes of well-formed headers into `control`, and
    // CMSG_NXTHDR stays inside them. A credentials message of at least CMSG_LEN(ucred) holds one
    // ucred; a rights message holds (len - CMSG_LEN(0)) / 4 descriptors this process now owns.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&raw const m);
        while !c.is_null() {
            let len = (*c).cmsg_len as usize;
            let data = libc::CMSG_DATA(c);
            if (*c).cmsg_level == libc::SOL_SOCKET {
                let ucred = std::mem::size_of::<libc::ucred>() as u32;
                if (*c).cmsg_type == libc::SCM_CREDENTIALS && len >= libc::CMSG_LEN(ucred) as usize
                {
                    pid = Some(data.cast::<libc::ucred>().read_unaligned().pid);
                } else if (*c).cmsg_type == libc::SCM_RIGHTS {
                    let fds = len.saturating_sub(libc::CMSG_LEN(0) as usize) / 4;
                    for i in 0..fds {
                        drop(std::os::fd::OwnedFd::from_raw_fd(
                            data.cast::<i32>().add(i).read_unaligned(),
                        ));
                    }
                }
            }
            c = libc::CMSG_NXTHDR(&raw const m, c);
        }
    }
    Ok((n, pid))
}

/// Reads a socket and keeps the first descriptor that arrives with the bytes.
pub struct FdReader<S> {
    sock: S,
    pub fd: Option<std::os::fd::OwnedFd>,
}

impl<S: AsRawFd> FdReader<S> {
    pub fn new(sock: S) -> Self {
        Self { sock, fd: None }
    }
}

impl<S: AsRawFd> std::io::Read for FdReader<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let (n, fd) = recv_with_fd(&self.sock, buf)?;
        if self.fd.is_none() {
            self.fd = fd;
        }
        Ok(n)
    }
}

/// Effective uid of this process.
pub fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Executable path of `pid` as bytes written into `buf`; returns the length. Does not allocate.
pub fn executable(pid: i32, buf: &mut [u8]) -> Option<usize> {
    if pid <= 0 || buf.is_empty() {
        return None;
    }
    let mut path = [0u8; 32];
    let mut w = &mut path[..];
    use std::io::Write;
    write!(w, "/proc/{pid}/exe\0").ok()?;
    // SAFETY: path is NUL-terminated; readlink writes at most buf.len() bytes.
    let n = unsafe { libc::readlink(path.as_ptr().cast(), buf.as_mut_ptr().cast(), buf.len()) };
    (n > 0 && (n as usize) < buf.len()).then_some(n as usize)
}

/// argv of `pid`. Allocates: use once per client, not per frame.
pub fn args(pid: i32) -> Option<Vec<String>> {
    if pid <= 0 {
        return None;
    }
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(
        raw.split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect(),
    )
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

/// `<runtime_dir>/hypr/<signature>/<name>`: where the compositor puts its IPC socket `name`
/// (`.socket.sock` requests, `.socket2.sock` events). Pure, for tests.
pub fn hypr_socket_in(runtime_dir: &str, signature: &str, name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(runtime_dir)
        .join("hypr")
        .join(signature)
        .join(name)
}

/// The running compositor's socket `name`: [`hypr_socket_in`] with `XDG_RUNTIME_DIR` and
/// `HYPRLAND_INSTANCE_SIGNATURE` as `var` gives them (`|k| std::env::var(k).ok()` in the session);
/// None when either is unset (outside the compositor).
pub fn hypr_socket(var: impl Fn(&str) -> Option<String>, name: &str) -> Option<std::path::PathBuf> {
    let rt = var("XDG_RUNTIME_DIR")?;
    let sig = var("HYPRLAND_INSTANCE_SIGNATURE")?;
    Some(hypr_socket_in(&rt, &sig, name))
}

#[cfg(test)]
mod fd_tests {
    use super::*;
    use std::io::{Read, Seek, Write};
    use std::os::fd::AsFd;
    use std::os::unix::net::UnixStream;

    /// tty_nr of this process (`/proc/self/stat` field 7): 0 = no controlling terminal.
    fn ctty() -> i64 {
        let st = std::fs::read_to_string("/proc/self/stat").unwrap();
        let rest = &st[st.rfind(')').unwrap() + 1..];
        rest.split_whitespace().nth(4).unwrap().parse().unwrap()
    }

    /// Audit S4-2: a session leader without a terminal (wold under systemd) that opens a pty by
    /// name through [`open_named`] does not make it its controlling terminal; a plain open does
    /// (the control). The probe runs as a child under setsid(1) on a pty from python3; skipped
    /// when either is missing.
    #[test]
    fn opening_a_terminal_by_name_never_makes_it_ours() {
        if let Some(tty) = std::env::var_os("WOL_CTTY_PROBE") {
            assert_eq!(ctty(), 0, "setsid left a terminal");
            let plain = std::env::var_os("WOL_CTTY_PLAIN").is_some();
            let f = if plain {
                std::fs::File::open(&tty).unwrap()
            } else {
                open_named(&tty).unwrap()
            };
            assert!(!f.metadata().unwrap().is_file());
            assert_eq!(
                ctty() != 0,
                plain,
                "plain open takes the terminal, open_named never"
            );
            return;
        }
        use std::process::{Command, Stdio};
        let py = Command::new("python3")
            .args([
                "-c",
                "import os,sys; m,s=os.openpty(); print(os.ttyname(s), flush=True); sys.stdin.read()",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn();
        let Ok(mut py) = py else {
            println!("skipped: no python3 to make a pty");
            return;
        };
        let mut line = Vec::new();
        let mut out = py.stdout.take().unwrap();
        let mut b = [0u8; 1];
        while out.read(&mut b).unwrap() == 1 && b[0] != b'\n' {
            line.push(b[0]);
        }
        let tty = String::from_utf8(line).unwrap();
        assert!(tty.starts_with("/dev/pts/"), "{tty}");
        let exe = std::env::current_exe().unwrap();
        let name = "sys::fd_tests::opening_a_terminal_by_name_never_makes_it_ours";
        let probe = |plain: bool| {
            let mut c = Command::new("setsid");
            c.arg("--wait")
                .arg(&exe)
                .args(["--exact", name, "--test-threads=1"])
                .env("WOL_CTTY_PROBE", &tty)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if plain {
                c.env("WOL_CTTY_PLAIN", "1");
            }
            c.status()
        };
        match (probe(false), probe(true)) {
            (Ok(quiet), Ok(plain)) => {
                assert!(quiet.success(), "open_named took the terminal");
                assert!(plain.success(), "the control: a plain open takes it");
            }
            _ => println!("skipped: no setsid"),
        }
        drop(py.stdin.take());
        let _ = py.wait();
    }

    #[test]
    fn a_descriptor_crosses_a_socket_with_its_line() {
        let dir = std::env::temp_dir().join(format!("wol-sys-fd-{}", std::process::id()));
        let mut f = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&dir)
            .unwrap();
        f.write_all(b"inside").unwrap();
        f.rewind().unwrap();
        let (a, b) = UnixStream::pair().unwrap();
        send_with_fd(&a, b"line\n", f.as_fd()).unwrap();
        let mut r = FdReader::new(b.try_clone().unwrap());
        let mut buf = [0u8; 16];
        let n = r.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"line\n");
        let mut got = String::new();
        std::fs::File::from(r.fd.take().unwrap())
            .read_to_string(&mut got)
            .unwrap();
        assert_eq!(got, "inside");
        (&a).write_all(b"plain").unwrap();
        let (n, fd) = recv_with_fd(&b, &mut buf).unwrap();
        assert_eq!((n, fd.is_none()), (5, true));
        let _ = std::fs::remove_file(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn hypr_socket_layout() {
        assert_eq!(
            hypr_socket_in("/run/user/1000", "abc", ".socket2.sock"),
            std::path::PathBuf::from("/run/user/1000/hypr/abc/.socket2.sock")
        );
        let env = |k: &str| (k == "XDG_RUNTIME_DIR").then(|| "/run/user/7".to_string());
        assert_eq!(
            hypr_socket(env, ".socket.sock"),
            None,
            "outside the compositor"
        );
        let env = |k: &str| Some(if k == "XDG_RUNTIME_DIR" { "/r" } else { "s" }.to_string());
        assert_eq!(
            hypr_socket(env, ".socket.sock"),
            Some(std::path::PathBuf::from("/r/hypr/s/.socket.sock"))
        );
    }

    #[test]
    fn peer_of_socketpair_is_this_process() {
        let (a, _b) = UnixStream::pair().unwrap();
        let p = peer(&a).unwrap();
        assert_eq!(p.pid, std::process::id() as i32);
        assert_eq!(p.uid, unsafe { libc::getuid() });
        assert_eq!(euid(), p.uid);
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
