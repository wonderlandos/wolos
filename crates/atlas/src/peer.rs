// SPDX-License-Identifier: GPL-2.0-or-later
//! Who is on the other end (STANDARDS 7.1 "Identity", audit C10-6): the uid and pid from
//! SO_PEERCRED, `/proc/<pid>` opened as a directory right at accept so it pins that process, and
//! its executable read through that descriptor. If the process dies and the pid is reused, reads
//! through the descriptor fail instead of describing the newcomer; a peer that exec()s into another
//! program after the hello is caught by [`Identity::still`] before each request.
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Identity {
    pub uid: u32,
    pub pid: i32,
    /// `/proc/<pid>`, opened at accept; None if the kernel would not say.
    dir: Option<std::fs::File>,
    /// The executable at accept.
    exe: Option<PathBuf>,
}

fn exe_of(dir: &std::fs::File) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{}/exe", dir.as_raw_fd())).ok()
}

impl Identity {
    /// SO_PEERCRED and the pinned directory, now. None when the socket has no peer credentials.
    pub fn capture(stream: &impl AsRawFd) -> Option<Self> {
        let p = shared::sys::peer(stream)?;
        let dir = (p.pid > 0)
            .then(|| std::fs::File::open(format!("/proc/{}", p.pid)).ok())
            .flatten();
        let exe = dir.as_ref().and_then(exe_of);
        Some(Self {
            uid: p.uid,
            pid: p.pid,
            dir,
            exe,
        })
    }

    /// True when the peer is this account's process running `browser` (the canonical path), and
    /// still runs the executable it ran at accept.
    pub fn is(&self, browser: &Path) -> bool {
        self.uid == shared::sys::euid() && self.exe.as_deref() == Some(browser) && self.still()
    }

    /// The pinned process still runs the executable it ran at accept.
    pub fn still(&self) -> bool {
        match (&self.dir, &self.exe) {
            (Some(d), Some(was)) => exe_of(d).as_ref() == Some(was),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn a_socket_pair_is_this_process() {
        let (a, _b) = UnixStream::pair().unwrap();
        let id = Identity::capture(&a).unwrap();
        assert_eq!(id.pid, std::process::id() as i32);
        let me = std::env::current_exe().unwrap().canonicalize().unwrap();
        assert!(id.is(&me));
        assert!(!id.is(Path::new(shared::atlas::BROWSER)));
    }
}
