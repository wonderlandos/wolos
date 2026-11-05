// SPDX-License-Identifier: GPL-2.0-or-later
//! Who is on the other end of the socket (STANDARDS 7.1 "Identity"): the uid and pid from
//! SO_PEERCRED, `/proc/<pid>` opened as a directory at accept so it pins that process, and its
//! executable read through that descriptor. Same scheme as atlas-bridge (crates/atlas/src/peer.rs).
use std::os::fd::AsRawFd;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Identity {
    pub uid: u32,
    pub pid: i32,
    dir: Option<std::fs::File>,
    pub exe: Option<PathBuf>,
}

fn exe_of(dir: &std::fs::File) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{}/exe", dir.as_raw_fd())).ok()
}

impl Identity {
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

    /// This account's own process.
    pub fn is_owner(&self) -> bool {
        self.uid == shared::sys::euid()
    }

    /// The pinned process still runs the executable it ran at accept (it did not exec into another program).
    pub fn still(&self) -> bool {
        match (&self.dir, &self.exe) {
            (Some(d), Some(was)) => exe_of(d).as_ref() == Some(was),
            _ => false,
        }
    }

    /// (parent pid, controlling terminal number) of the pinned process, read through the descriptor.
    pub fn stat(&self) -> Option<(i32, i64)> {
        let d = self.dir.as_ref()?;
        let st = std::fs::read_to_string(format!("/proc/self/fd/{}/stat", d.as_raw_fd())).ok()?;
        let f: Vec<&str> = st.rsplit_once(')')?.1.split_whitespace().collect();
        Some((f.get(1)?.parse().ok()?, f.get(4)?.parse().ok()?))
    }

    /// The executable of the pinned process's parent. The parent is pinned too (its `/proc/<pid>`
    /// opened as a directory) and counts only if the child still names it as its parent afterwards:
    /// a parent that died and whose pid was reused makes the child's parent change, so it is refused.
    pub fn parent_exe(&self) -> Option<String> {
        let (ppid, _) = self.stat()?;
        if ppid <= 1 {
            return None;
        }
        let dir = std::fs::File::open(format!("/proc/{ppid}")).ok()?;
        if self.stat()?.0 != ppid {
            return None;
        }
        exe_of(&dir).map(|p| p.display().to_string())
    }

    pub fn name(&self) -> String {
        self.exe
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| format!("pid {}", self.pid))
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
        assert!(id.is_owner() && id.still());
        assert_eq!(
            id.exe.as_deref(),
            Some(
                std::env::current_exe()
                    .unwrap()
                    .canonicalize()
                    .unwrap()
                    .as_path()
            )
        );
    }

    #[test]
    fn the_parent_is_pinned_through_the_child() {
        let (a, _b) = UnixStream::pair().unwrap();
        let id = Identity::capture(&a).unwrap();
        let (ppid, _tty) = id.stat().unwrap();
        assert_eq!(ppid, std::os::unix::process::parent_id() as i32);
        // the test runner's parent is a real program
        assert!(id.parent_exe().is_some_and(|e| e.starts_with('/')));
    }
}
