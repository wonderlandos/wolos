// SPDX-License-Identifier: GPL-2.0-or-later
//! Group names and the files of a group under `$XDG_RUNTIME_DIR/wol-tor/<group>/` (ADR 0053 D2).
//!
//! A group name ends up in unit names, paths and tor credentials, so it is a short lowercase
//! label and nothing else. Unix socket paths are limited to 107 bytes by the kernel; a group whose
//! paths do not fit is refused here instead of failing inside tor.
use std::fmt;
use std::path::{Path, PathBuf};

/// Longest group name.
pub const MAX_NAME: usize = 32;
/// `sun_path` holds 108 bytes including the terminating zero.
pub const MAX_SOCKET_PATH: usize = 107;

/// A checked group name: `[a-z0-9-]{1,32}`, not starting or ending with `-`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Group(String);

/// Why a group name or its paths were refused.
#[derive(Debug, PartialEq, Eq)]
pub enum GroupError {
    /// Empty, too long, or a character outside `[a-z0-9-]`.
    BadName(String),
    /// The runtime directory is not absolute.
    RelativeRuntime(PathBuf),
    /// A socket path would not fit in `sun_path`.
    SocketTooLong(PathBuf),
}

impl fmt::Display for GroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName(n) => write!(
                f,
                "the Tor group name {n:?} is not valid: use 1 to {MAX_NAME} lowercase letters, digits or dashes"
            ),
            Self::RelativeRuntime(p) => write!(
                f,
                "the runtime directory {} is not an absolute path",
                p.display()
            ),
            Self::SocketTooLong(p) => write!(
                f,
                "the socket path {} is longer than {MAX_SOCKET_PATH} bytes; use a shorter group name",
                p.display()
            ),
        }
    }
}

impl std::error::Error for GroupError {}

impl Group {
    /// Checks a group name.
    pub fn new(name: &str) -> Result<Self, GroupError> {
        let ok = !name.is_empty()
            && name.len() <= MAX_NAME
            && !name.starts_with('-')
            && !name.ends_with('-')
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if ok {
            Ok(Self(name.to_string()))
        } else {
            Err(GroupError::BadName(name.to_string()))
        }
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The files of one group. All live in a directory that only the user can open (mode 0700,
/// created by systemd as the tor unit's `RuntimeDirectory`, removed when the unit stops).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// `$XDG_RUNTIME_DIR/wol-tor/<group>`.
    pub dir: PathBuf,
    /// tor's DataDirectory (tmpfs: the runtime directory).
    pub data: PathBuf,
    /// tor's SOCKS5 Unix socket; the relay is its only client.
    pub socks: PathBuf,
    /// tor's control Unix socket.
    pub control: PathBuf,
    /// tor's control cookie.
    pub cookie: PathBuf,
    /// The browser unit's runtime directory (`RuntimeDirectory=` of `atlas-tor-<g>.service`):
    /// real tmpfs, the only place that unit may write, removed when it stops.
    pub browser: PathBuf,
    /// The host's command socket (open a tab, new circuit for a site), in [`Self::browser`]:
    /// tor's runtime directory is emptied when tor restarts, the host outlives that.
    pub host: PathBuf,
    /// Atlas's `--user-data-dir`, in [`Self::browser`] (tmpfs, never on disk).
    pub atlas_profile: PathBuf,
    /// The Tor instance's policy files and `fonts.conf`, written by the launcher before the
    /// browser unit starts and bound read-only into it (`crate::profile::write_to`).
    pub profile: PathBuf,
}

impl Paths {
    /// The paths of `group` under the runtime directory `runtime` (`$XDG_RUNTIME_DIR`).
    pub fn new(runtime: &Path, group: &Group) -> Result<Self, GroupError> {
        if !runtime.is_absolute() {
            return Err(GroupError::RelativeRuntime(runtime.to_path_buf()));
        }
        let dir = runtime.join("wol-tor").join(group.as_str());
        let browser = runtime.join(Self::browser_runtime_directory(group));
        let p = Self {
            data: dir.join("data"),
            socks: dir.join("socks"),
            control: dir.join("control"),
            cookie: dir.join("control.cookie"),
            host: browser.join("host"),
            atlas_profile: browser.join("profile"),
            profile: runtime.join("wol-tor-profile").join(group.as_str()),
            browser,
            dir,
        };
        for s in [&p.socks, &p.control, &p.host] {
            if s.as_os_str().len() > MAX_SOCKET_PATH {
                return Err(GroupError::SocketTooLong(s.clone()));
            }
        }
        Ok(p)
    }

    /// The runtime directory name systemd creates for the browser unit, relative to
    /// `$XDG_RUNTIME_DIR` (a different tree from tor's, so no group name can collide).
    pub fn browser_runtime_directory(group: &Group) -> String {
        format!("wol-tor-browser/{group}")
    }

    /// The runtime directory name systemd creates for the tor unit (`RuntimeDirectory=`), relative
    /// to `$XDG_RUNTIME_DIR`.
    pub fn runtime_directory(group: &Group) -> String {
        format!("wol-tor/{group}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_names_are_short_lowercase_labels() {
        for ok in ["a", "default", "g-1", "work2"] {
            assert!(Group::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "-a",
            "a-",
            "A",
            "a b",
            "a/b",
            "a.b",
            "a\n",
            &"x".repeat(33),
        ] {
            assert_eq!(
                Group::new(bad),
                Err(GroupError::BadName(bad.to_string())),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn paths_live_under_the_runtime_dir() {
        let g = Group::new("default").unwrap();
        let p = Paths::new(Path::new("/run/u/1000"), &g).unwrap();
        assert_eq!(p.dir, Path::new("/run/u/1000/wol-tor/default"));
        assert_eq!(p.socks, Path::new("/run/u/1000/wol-tor/default/socks"));
        assert_eq!(Paths::runtime_directory(&g), "wol-tor/default");
        assert_eq!(
            p.host,
            Path::new("/run/u/1000/wol-tor-browser/default/host")
        );
        assert_eq!(
            p.atlas_profile,
            Path::new("/run/u/1000/wol-tor-browser/default/profile")
        );
        assert_eq!(p.profile, Path::new("/run/u/1000/wol-tor-profile/default"));
    }

    #[test]
    fn too_long_socket_paths_are_refused() {
        let g = Group::new(&"g".repeat(32)).unwrap();
        let deep = PathBuf::from(format!("/{}", "d".repeat(80)));
        assert!(matches!(
            Paths::new(&deep, &g),
            Err(GroupError::SocketTooLong(_))
        ));
        assert!(matches!(
            Paths::new(Path::new("rel"), &g),
            Err(GroupError::RelativeRuntime(_))
        ));
    }
}
