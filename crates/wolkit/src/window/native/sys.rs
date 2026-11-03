// SPDX-License-Identifier: BSD-2-Clause
//! The operating-system calls the native backend needs and std does not wrap: an anonymous
//! shared-memory file for `wl_shm`, a timer descriptor for key repeat and the light/dark switch,
//! inotify for the theme files, and the local hour. Every `unsafe` of the native backend except
//! the keymap's is here, each with its reason (the crate's ratchet row counts them).

use std::ffi::CStr;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::time::Duration;

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// A file descriptor the kernel just returned, owned by nothing else, as a `File`.
fn own(fd: libc::c_int) -> std::io::Result<File> {
    if fd < 0 {
        return Err(last_error());
    }
    // SAFETY: `fd` is a fresh descriptor from the kernel (checked non-negative) that no other
    // value owns, so the `File` may close it.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// An anonymous file in memory (for `wl_shm` pools): only this process and the compositor see it.
pub fn memfd(name: &CStr) -> std::io::Result<File> {
    // SAFETY: `name` is a valid NUL-terminated string for the duration of the call.
    own(unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) })
}

/// A monotonic timer, readable when it fired.
pub struct Timer {
    file: File,
}

impl Timer {
    pub fn new() -> std::io::Result<Timer> {
        let flags = libc::TFD_NONBLOCK | libc::TFD_CLOEXEC;
        // SAFETY: plain integer arguments, no pointers.
        let file = own(unsafe { libc::timerfd_create(libc::CLOCK_MONOTONIC, flags) })?;
        Ok(Timer { file })
    }

    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// Fire after `first`, then every `every` (zero: once). `None` disarms.
    pub fn set(&self, first: Option<Duration>, every: Duration) -> std::io::Result<()> {
        let ts = |d: Duration| libc::timespec {
            tv_sec: libc::time_t::try_from(d.as_secs()).unwrap_or(libc::time_t::MAX),
            tv_nsec: libc::c_long::from(d.subsec_nanos()),
        };
        // A zero expiry disarms, so a requested zero becomes the smallest real one.
        let first = first.map_or(Duration::ZERO, |d| d.max(Duration::from_nanos(1)));
        let spec = libc::itimerspec {
            it_interval: ts(every),
            it_value: ts(first),
        };
        let fd = self.file.as_raw_fd();
        // SAFETY: `spec` is a live itimerspec for the call; NULL is allowed for the old value.
        let r = unsafe { libc::timerfd_settime(fd, 0, &spec, std::ptr::null_mut()) };
        if r < 0 { Err(last_error()) } else { Ok(()) }
    }

    /// How many times it fired since the last call (0 when it did not).
    pub fn take(&self) -> u64 {
        let mut b = [0u8; 8];
        match (&self.file).read(&mut b) {
            Ok(8) => u64::from_ne_bytes(b),
            _ => 0,
        }
    }
}

/// inotify on a few directories (theme files are replaced by rename, so the directory is
/// watched, not the file).
pub struct Inotify {
    file: File,
}

/// Events that mean "a file in the directory changed".
const WATCH: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_TO
    | libc::IN_MOVED_FROM
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

impl Inotify {
    pub fn new() -> std::io::Result<Inotify> {
        let flags = libc::IN_NONBLOCK | libc::IN_CLOEXEC;
        // SAFETY: plain integer argument, no pointers.
        let file = own(unsafe { libc::inotify_init1(flags) })?;
        Ok(Inotify { file })
    }

    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// Watch `dir`; a directory that does not exist is an error the caller may ignore.
    pub fn watch(&self, dir: &std::path::Path) -> std::io::Result<()> {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let fd = self.file.as_raw_fd();
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        let r = unsafe { libc::inotify_add_watch(fd, c.as_ptr(), WATCH) };
        if r < 0 { Err(last_error()) } else { Ok(()) }
    }

    /// Drain pending events; true when there were any.
    pub fn drain(&self) -> bool {
        let mut b = [0u8; 4096];
        let mut any = false;
        // Bounded: at most 64 reads of a full buffer per call.
        for _ in 0..64 {
            match (&self.file).read(&mut b) {
                Ok(n) if n > 0 => any = true,
                _ => break,
            }
        }
        any
    }
}

/// The local wall-clock hour, minute and second (UTC when the time zone cannot be read).
pub fn local_time() -> (u8, u8, u8) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let t: libc::time_t = libc::time_t::try_from(now).unwrap_or(0);
    // SAFETY: an all-zero `tm` is a valid value of the plain C struct (the call overwrites it).
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to live locals for the duration of the call.
    let ok = !unsafe { libc::localtime_r(&t, &mut tm) }.is_null();
    let b = |v: libc::c_int| u8::try_from(v).unwrap_or(0);
    if ok {
        (b(tm.tm_hour), b(tm.tm_min), b(tm.tm_sec))
    } else {
        let s = now % 86_400;
        let b64 = |v: u64| u8::try_from(v).unwrap_or(0);
        (b64(s / 3600), b64(s / 60 % 60), b64(s % 60))
    }
}

/// Seconds-and-nanoseconds of the monotonic clock (the presentation clock's usual id).
pub fn monotonic_ns() -> u64 {
    // std's Instant is monotonic but has no epoch; the presentation clock needs the raw value.
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a live timespec for the call.
    let r = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    if r != 0 {
        return 0;
    }
    let s = u64::try_from(ts.tv_sec).unwrap_or(0);
    let n = u64::try_from(ts.tv_nsec).unwrap_or(0);
    s.saturating_mul(1_000_000_000).saturating_add(n)
}
