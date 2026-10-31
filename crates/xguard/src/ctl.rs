// SPDX-License-Identifier: GPL-2.0-or-later
//! The guard's local socket: `check` and `allow`, each with an open file passed alongside, so a
//! caller can only ask about files it can really open, and the guard learns no path from it.
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub const SOCKET: &str = "/run/wolos/xguard.sock";

/// The guard's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    /// `clean`, `found` or `allowed`.
    pub state: String,
    /// What was found.
    #[serde(default)]
    pub name: String,
}

fn cmsg_space() -> usize {
    // SAFETY: arithmetic only.
    unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as usize }
}

/// Send one verb line with `fd` attached.
pub fn send_fd(s: &UnixStream, verb: &str, fd: RawFd) -> std::io::Result<()> {
    let mut line = format!("{verb}\n").into_bytes();
    let mut iov = libc::iovec {
        iov_base: line.as_mut_ptr().cast(),
        iov_len: line.len(),
    };
    let mut ctl = vec![0u8; cmsg_space()];
    // SAFETY: msghdr points at live buffers; CMSG macros stay inside `ctl`, sized by CMSG_SPACE.
    unsafe {
        let mut m: libc::msghdr = std::mem::zeroed();
        m.msg_iov = &mut iov;
        m.msg_iovlen = 1;
        m.msg_control = ctl.as_mut_ptr().cast();
        m.msg_controllen = ctl.len() as _;
        let c = libc::CMSG_FIRSTHDR(&m);
        (*c).cmsg_level = libc::SOL_SOCKET;
        (*c).cmsg_type = libc::SCM_RIGHTS;
        (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(c).cast(), fd);
        if libc::sendmsg(s.as_raw_fd(), &m, 0) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// One verb line and the file that came with it.
pub fn recv_fd(s: &UnixStream) -> std::io::Result<(String, Option<OwnedFd>)> {
    let mut buf = [0u8; 64];
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let mut ctl = vec![0u8; cmsg_space()];
    // SAFETY: as in `send_fd`; a received descriptor is owned by us from here on.
    unsafe {
        let mut m: libc::msghdr = std::mem::zeroed();
        m.msg_iov = &mut iov;
        m.msg_iovlen = 1;
        m.msg_control = ctl.as_mut_ptr().cast();
        m.msg_controllen = ctl.len() as _;
        let n = libc::recvmsg(s.as_raw_fd(), &mut m, libc::MSG_CMSG_CLOEXEC);
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let c = libc::CMSG_FIRSTHDR(&m);
        let fd = (!c.is_null()
            && (*c).cmsg_level == libc::SOL_SOCKET
            && (*c).cmsg_type == libc::SCM_RIGHTS)
            .then(|| {
                OwnedFd::from_raw_fd(std::ptr::read_unaligned(libc::CMSG_DATA(c).cast::<RawFd>()))
            });
        let line = String::from_utf8_lossy(&buf[..n as usize]);
        Ok((line.trim_end().to_owned(), fd))
    }
}

/// Who is on the other end of the socket, from the kernel.
pub fn peer_uid(s: &UnixStream) -> Option<u32> {
    let mut c: libc::ucred = unsafe { std::mem::zeroed() };
    let mut n = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: writes at most n bytes to c.
    let r = unsafe {
        libc::getsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::from_mut(&mut c).cast(),
            &mut n,
        )
    };
    (r == 0).then_some(c.uid)
}

pub fn write_reply(mut s: &UnixStream, r: &Reply) {
    if let Ok(t) = serde_json::to_string(r) {
        let _ = writeln!(s, "{t}");
    }
}

/// Ask the guard about an open file.
pub fn ask(sock: &Path, verb: &str, f: &std::fs::File) -> std::io::Result<Reply> {
    let s = UnixStream::connect(sock)?;
    send_fd(&s, verb, f.as_raw_fd())?;
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_travels_with_its_verb() {
        let (a, b) = UnixStream::pair().unwrap();
        let p = std::env::temp_dir().join(format!("xguard-ctl-{}", std::process::id()));
        std::fs::write(&p, "contents").unwrap();
        let f = std::fs::File::open(&p).unwrap();
        send_fd(&a, "check", f.as_raw_fd()).unwrap();
        let (verb, fd) = recv_fd(&b).unwrap();
        assert_eq!(verb, "check");
        let mut got = String::new();
        std::io::Read::read_to_string(&mut std::fs::File::from(fd.unwrap()), &mut got).unwrap();
        assert_eq!(got, "contents");
        assert_eq!(peer_uid(&b), Some(unsafe { libc::geteuid() }));
        write_reply(
            &b,
            &Reply {
                state: "found".into(),
                name: "T".into(),
            },
        );
        let mut line = String::new();
        BufReader::new(&a).read_line(&mut line).unwrap();
        let r: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(r.state, "found");
        std::fs::remove_file(&p).ok();
    }
}
