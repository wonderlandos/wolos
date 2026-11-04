// SPDX-License-Identifier: GPL-2.0-or-later
//! The listening socket: the one systemd passes (`atlas-bridge.socket`), or one bound by hand for
//! development (`atlas-bridge --socket <path>`).
use std::os::unix::net::UnixListener;
use std::path::Path;

/// The first descriptor systemd passes (sd_listen_fds(3)).
const LISTEN_FDS_START: i32 = 3;

/// The socket systemd passed to this process, if it passed exactly one. `listen_pid` and
/// `listen_fds` are the values of `$LISTEN_PID` and `$LISTEN_FDS`, which the binary reads.
pub fn from_systemd(listen_pid: Option<&str>, listen_fds: Option<&str>) -> Option<UnixListener> {
    if listen_pid? != std::process::id().to_string() || listen_fds? != "1" {
        return None;
    }
    // A child that inherits the variables (the xdg-open fallback) fails the LISTEN_PID check.
    adopt().ok()
}

#[allow(unsafe_code)] // the one unsafe block of the crate: adopting an inherited descriptor
fn adopt() -> std::io::Result<UnixListener> {
    use std::os::fd::FromRawFd;
    // SAFETY: systemd handed descriptor 3 to this process (LISTEN_PID and LISTEN_FDS checked by the
    // caller) and nothing else in the process owns it. That it is a listening unix socket is not
    // checked here; if it were not, `accept` would fail on it, the accept thread would back off
    // (`ACCEPT_BACKOFF`) and the bridge would leave after its idle time.
    let inherited = unsafe { UnixListener::from_raw_fd(LISTEN_FDS_START) };
    close_on_exec(inherited)
}

/// A copy of `l` that is closed in every program this process starts, and `l` closed. systemd
/// passes its descriptors without close-on-exec; `try_clone` is `F_DUPFD_CLOEXEC`, so the opener
/// the fallback runs (and the app it starts) never holds the listening socket, which could
/// otherwise take the browser's next connection.
pub fn close_on_exec(l: UnixListener) -> std::io::Result<UnixListener> {
    l.try_clone()
}

/// Bind `path` (0600, its directory 0700) for a development run.
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let _ = std::fs::remove_file(path);
    let l = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    /// O_CLOEXEC as `/proc/<pid>/fdinfo` prints it (octal).
    const O_CLOEXEC: u32 = 0o2_000_000;

    fn closes_on_exec(l: &UnixListener) -> bool {
        let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", l.as_raw_fd())).unwrap();
        let flags = info
            .lines()
            .find_map(|x| x.strip_prefix("flags:"))
            .and_then(|f| u32::from_str_radix(f.trim(), 8).ok())
            .unwrap();
        flags & O_CLOEXEC != 0
    }

    #[test]
    fn the_adopted_listener_is_closed_in_programs_it_starts() {
        let dir = std::env::temp_dir().join(format!("atlas-cloexec-{}", std::process::id()));
        let l = bind(&dir.join("s.sock")).unwrap();
        let old = l.as_raw_fd();
        let l = close_on_exec(l).unwrap();
        assert!(closes_on_exec(&l));
        assert_ne!(
            l.as_raw_fd(),
            old,
            "a new descriptor; the inherited one is closed"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
