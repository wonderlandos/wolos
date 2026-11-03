// SPDX-License-Identifier: GPL-2.0-or-later
//! Who is on the other end of a socket, captured once at accept (ADR 0020, audit B1/A2). The peer
//! uid and pid come from SO_PEERCRED; `/proc/<pid>` is opened as a directory right away, so the
//! descriptor pins that process: if it dies and the pid is reused, reads through it fail instead of
//! describing the newcomer. The executable is read at accept through that descriptor and read again
//! before anything is granted: a peer that sent its request and then exec()ed into another binary
//! is refused, never identified as the binary it became.
use crate::portal::Caller;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

/// Longest executable path we accept (bytes): PATH_MAX.
const MAX_EXE: usize = 4096;

/// The kernel's account of a connected peer.
#[derive(Debug)]
pub struct PeerIdentity {
    pub uid: u32,
    pub pid: i32,
    /// Canonical executable at accept time; None if the kernel would not say.
    exe: Option<String>,
    /// `/proc/<pid>` opened at accept.
    dir: Option<std::fs::File>,
}

impl PeerIdentity {
    /// SO_PEERCRED plus a pinned `/proc/<pid>` and its executable, right now. None if the socket
    /// has no peer credentials (not a Unix socket).
    pub fn capture(stream: &impl AsRawFd) -> Option<Self> {
        let p = shared::sys::peer(stream)?;
        let dir = (p.pid > 0)
            .then(|| std::fs::File::open(format!("/proc/{}", p.pid)).ok())
            .flatten();
        let exe = dir.as_ref().and_then(exe_of);
        Some(Self {
            uid: p.uid,
            pid: p.pid,
            exe,
            dir,
        })
    }

    /// The raw credentials.
    pub fn peer(&self) -> shared::sys::Peer {
        shared::sys::Peer {
            uid: self.uid,
            pid: self.pid,
        }
    }

    /// True while the pinned process still runs the executable it ran at accept.
    pub fn still_same(&self) -> bool {
        match (&self.exe, &self.dir) {
            (Some(was), Some(dir)) => exe_of(dir).as_ref() == Some(was),
            _ => false,
        }
    }

    /// The peer is no longer who it was at accept (audit S4-26): an executable was known then and
    /// now differs or cannot be read (an exec, or the pinned process is gone). An executable that
    /// was unknown on first sight is not a change: no identity, nothing to lose. For calls that
    /// need no grant (`publish`, `subscribe`); grants use [`PeerIdentity::caller`], which also
    /// refuses an unknown executable.
    pub fn changed(&self) -> bool {
        match (&self.exe, &self.dir) {
            (None, _) => false,
            (Some(was), Some(dir)) => exe_of(dir).as_ref() != Some(was),
            (Some(_), None) => true,
        }
    }

    /// The calling app for a grant decision. None (refuse) if the executable is unknown, not an
    /// absolute path, or changed since accept.
    pub fn caller(&self) -> Option<Caller> {
        let app = self.exe.clone()?;
        (app.starts_with('/') && self.still_same()).then_some(Caller {
            uid: self.uid,
            pid: self.pid,
            app,
        })
    }

    /// [`PeerIdentity::capture`] of `stream`, with the executable seen at accept replaced by
    /// `exe` (tests of what a changed or unknown identity may do).
    #[cfg(test)]
    pub(crate) fn captured_as(stream: &impl AsRawFd, exe: Option<&str>) -> Option<Self> {
        let mut id = Self::capture(stream)?;
        id.exe = exe.map(str::to_owned);
        Some(id)
    }

    /// Executable bytes as captured at accept (for exact-path checks such as the realtime agent).
    pub fn exe(&self) -> Option<&str> {
        self.exe.as_deref()
    }
}

/// `exe` of the process a `/proc/<pid>` descriptor pins, read through `/proc/self/fd` so the pid is
/// never looked up again.
fn exe_of(dir: &std::fs::File) -> Option<String> {
    let link = PathBuf::from(format!("/proc/self/fd/{}/exe", dir.as_raw_fd()));
    let target = std::fs::read_link(link).ok()?;
    let s = target.into_os_string().into_string().ok()?;
    (s.len() < MAX_EXE).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn a_live_peer_is_identified_and_stays_the_same() {
        let (a, _b) = UnixStream::pair().unwrap();
        let id = PeerIdentity::capture(&a).unwrap();
        assert_eq!(id.pid, i32::try_from(std::process::id()).unwrap());
        assert!(id.still_same());
        let me = std::env::current_exe().unwrap();
        assert_eq!(id.caller().unwrap().app, me.to_string_lossy());
    }

    #[test]
    fn a_dead_peer_is_not_the_same() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        let dir = std::fs::File::open(format!("/proc/{pid}")).unwrap();
        let id = PeerIdentity {
            uid: 0,
            pid,
            exe: exe_of(&dir),
            dir: Some(dir),
        };
        assert!(id.still_same());
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!id.still_same(), "the pinned process is gone");
        assert!(id.caller().is_none());
        assert!(id.changed(), "known at accept, gone now: a change");
    }

    /// Audit S4-26: only a change of a known identity counts; an executable unknown on first
    /// sight is not refused for grant-free calls (it still gets no grant).
    #[test]
    fn only_a_known_identity_can_change() {
        let (a, _b) = UnixStream::pair().unwrap();
        let live = PeerIdentity::capture(&a).unwrap();
        assert!(!live.changed(), "same executable as at accept");
        let unknown = PeerIdentity {
            uid: 1000,
            pid: live.pid,
            exe: None,
            dir: None,
        };
        assert!(!unknown.changed(), "unknown on first sight is not a change");
        assert!(unknown.caller().is_none(), "and still no grant");
        let execd = PeerIdentity {
            uid: 1000,
            pid: live.pid,
            exe: Some("/usr/bin/not-this-test".into()),
            dir: std::fs::File::open(format!("/proc/{}", live.pid)).ok(),
        };
        assert!(execd.changed(), "another executable than at accept");
    }
}
