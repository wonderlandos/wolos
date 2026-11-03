// SPDX-License-Identifier: BSD-2-Clause
//! The wait side of the engine: a self-pipe the engine's own threads write to, `poll`, and the
//! readiness set the app waits on (`wolkit_engine_get_fd`): an epoll descriptor holding the
//! self-pipe and every descriptor of every window backend (the native backend's Wayland socket,
//! theme watch, key-repeat timer, clipboard pipes), so it is readable while `dispatch` has work.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

/// A wake-up source: threads of the engine call [`Waker::wake`], the dispatching thread polls
/// [`Waker::fd`] and calls [`Waker::drain`].
pub struct Waker {
    rx: UnixStream,
    tx: UnixStream,
}

impl Waker {
    pub fn new() -> std::io::Result<Waker> {
        let (rx, tx) = UnixStream::pair()?;
        rx.set_nonblocking(true)?;
        tx.set_nonblocking(true)?;
        Ok(Waker { rx, tx })
    }

    /// Readable while a wake-up is pending.
    pub fn fd(&self) -> RawFd {
        self.rx.as_raw_fd()
    }

    /// A handle other threads use to wake the loop.
    pub fn sender(&self) -> std::io::Result<UnixStream> {
        self.tx.try_clone()
    }

    pub fn wake(&self) {
        // A full pipe already means "awake"; nothing to do on an error.
        let _ = (&self.tx).write(&[1]);
    }

    pub fn drain(&self) {
        let mut b = [0u8; 64];
        while matches!((&self.rx).read(&mut b), Ok(n) if n > 0) {}
    }
}

/// A level-triggered epoll set. Polling its descriptor says "readable" while any member is
/// readable; [`Ready::sync`] makes the members exactly a given list.
pub struct Ready {
    ep: OwnedFd,
    members: Mutex<Vec<RawFd>>,
    /// The wanted list, reused so a re-sync allocates nothing once warm (`tests/alloc_free.rs`).
    want: Mutex<Vec<RawFd>>,
}

impl Ready {
    pub fn new() -> std::io::Result<Ready> {
        // SAFETY: epoll_create1 takes no pointers; a non-negative return is a new descriptor
        // nothing else owns, so OwnedFd may take it.
        let ep = unsafe {
            let fd = libc::epoll_create1(libc::EPOLL_CLOEXEC);
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            OwnedFd::from_raw_fd(fd)
        };
        Ok(Ready {
            ep,
            members: Mutex::new(Vec::new()),
            want: Mutex::new(Vec::new()),
        })
    }

    /// The descriptor the app polls.
    pub fn fd(&self) -> RawFd {
        self.ep.as_raw_fd()
    }

    fn ctl(&self, op: libc::c_int, fd: RawFd) -> std::io::Result<()> {
        let mut e = libc::epoll_event {
            events: libc::EPOLLIN.cast_unsigned(),
            u64: u64::try_from(fd).unwrap_or(0),
        };
        // SAFETY: `e` is a live epoll_event for the duration of the call (DEL ignores it).
        if unsafe { libc::epoll_ctl(self.ep.as_raw_fd(), op, fd, &raw mut e) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Make the members exactly `fds`. A member that is still listed is modified rather than
    /// trusted: the kernel drops a closed descriptor by itself and its number can come back as a
    /// new one, which then is added again.
    pub fn sync(&self, fds: &[RawFd]) {
        let mut m = self.members.lock().unwrap_or_else(|e| e.into_inner());
        for old in m.iter().filter(|f| !fds.contains(f)) {
            let _ = self.ctl(libc::EPOLL_CTL_DEL, *old);
        }
        m.clear();
        for fd in fds.iter().copied().filter(|f| *f >= 0) {
            if m.contains(&fd) {
                continue;
            }
            let ok = self.ctl(libc::EPOLL_CTL_MOD, fd).is_ok()
                || self.ctl(libc::EPOLL_CTL_ADD, fd).is_ok();
            if ok {
                m.push(fd);
            }
        }
    }

    /// Fill the wanted list with `fill` (it starts empty; its buffer is reused) and sync to it.
    pub fn sync_with(&self, fill: impl FnOnce(&mut Vec<RawFd>)) {
        let mut want = self.want.lock().unwrap_or_else(|e| e.into_inner());
        want.clear();
        fill(&mut want);
        self.sync(&want);
    }

    /// The current members (tests).
    pub fn members(&self) -> Vec<RawFd> {
        self.members
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Wait until one of `fds` is readable or `timeout_ms` passes (negative waits forever). The
/// indexes of the readable ones are returned in `ready`.
pub fn wait(fds: &[RawFd], timeout_ms: i32, ready: &mut Vec<usize>) {
    ready.clear();
    let mut set: Vec<libc::pollfd> = fds
        .iter()
        .map(|fd| libc::pollfd {
            fd: *fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    // SAFETY: `set` is a live array of `set.len()` pollfd structs for the duration of the call.
    let n = unsafe { libc::poll(set.as_mut_ptr(), set.len() as libc::nfds_t, timeout_ms) };
    if n > 0 {
        ready.extend(
            set.iter()
                .enumerate()
                .filter(|(_, p)| p.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
                .map(|(i, _)| i),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_makes_the_fd_readable_and_drain_clears_it() {
        let w = Waker::new().unwrap();
        let mut ready = Vec::new();
        wait(&[w.fd()], 0, &mut ready);
        assert!(ready.is_empty());
        w.wake();
        wait(&[w.fd()], 1000, &mut ready);
        assert_eq!(ready, vec![0]);
        w.drain();
        wait(&[w.fd()], 0, &mut ready);
        assert!(ready.is_empty());
    }

    #[test]
    fn the_ready_set_is_readable_while_a_member_is() {
        let r = Ready::new().unwrap();
        let a = Waker::new().unwrap();
        let b = Waker::new().unwrap();
        let mut ready = Vec::new();
        r.sync(&[a.fd(), b.fd()]);
        wait(&[r.fd()], 0, &mut ready);
        assert!(ready.is_empty(), "nothing pending");
        b.wake();
        wait(&[r.fd()], 1000, &mut ready);
        assert_eq!(ready, vec![0], "a member became readable");
        wait(&[r.fd()], 0, &mut ready);
        assert_eq!(
            ready,
            vec![0],
            "level-triggered: still readable until drained"
        );
        b.drain();
        wait(&[r.fd()], 0, &mut ready);
        assert!(ready.is_empty(), "drained");
        // a member that leaves the list no longer wakes the set
        r.sync(&[a.fd()]);
        assert_eq!(r.members(), vec![a.fd()]);
        b.wake();
        wait(&[r.fd()], 0, &mut ready);
        assert!(ready.is_empty(), "b is not a member");
        // a closed member whose number comes back is watched again
        let n = b.fd();
        drop(b);
        r.sync(&[a.fd(), n]);
        let c = Waker::new().unwrap();
        let back = c.fd() == n || c.sender().is_ok_and(|s| s.as_raw_fd() == n);
        r.sync(&[a.fd(), c.fd()]);
        c.wake();
        wait(&[r.fd()], 1000, &mut ready);
        assert_eq!(
            ready,
            vec![0],
            "a new descriptor is watched (number reused: {back})"
        );
    }
}
