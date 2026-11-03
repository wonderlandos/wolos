// SPDX-License-Identifier: BSD-2-Clause
//! The wait side of the engine: a self-pipe the engine's own threads write to, and `poll`. The
//! app waits on `wolkit_engine_get_fd`; the native backend adds its Wayland descriptor.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;

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

/// Wait until one of `fds` is readable or `timeout_ms` passes (negative waits forever). The
/// indexes of the readable ones are returned in `ready`.
pub fn wait(fds: &[RawFd], timeout_ms: i32, ready: &mut Vec<usize>) {
    ready.clear();
    let mut set: Vec<libc::pollfd> = fds
        .iter()
        .map(|fd| libc::pollfd { fd: *fd, events: libc::POLLIN, revents: 0 })
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
}
