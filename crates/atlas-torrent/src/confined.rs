// SPDX-License-Identifier: BSD-2-Clause
//! The helper checks its own view before it serves (DL-SEC-01). `launch` asks the user service
//! manager for a confined unit ([`crate::sandbox`]), but a manager that cannot make the unit's
//! namespaces (no user namespaces for unprivileged accounts, an old systemd) may start it with the
//! mount fences silently absent: the real home readable, `/usr` writable by permission only. The
//! helper parses data from strangers, so `serve` refuses to run unless it sees, from inside:
//! - the home folder on a tmpfs holding only the folders handed to it (`ProtectHome=tmpfs`);
//! - `/usr` read-only (`ProtectSystem=strict`: a create gives EROFS, not EACCES);
//! - `NoNewPrivs: 1` and `Seccomp: 2` in `/proc/self/status` (`NoNewPrivileges`, `SystemCallFilter`);
//! - no block device in `/dev` (`PrivateDevices`).
//!
//! Before it exits it leaves [`REFUSED_FILE`] beside the socket, so `launch` can tell the browser
//! "this computer's service manager cannot confine the helper" instead of a time-out.
use std::ffi::OsString;
use std::os::unix::fs::FileTypeExt;
use std::path::{Component, Path};

/// The file `serve` writes in the socket folder when it refuses (one line: the gaps).
pub const REFUSED_FILE: &str = "torrent.refused";
/// The exit status of a refusing `serve` (and of `launch` when it saw the refusal).
pub const REFUSED_EXIT: u8 = 3;
/// What the browser's row says, printed by `launch` on stderr.
pub const REFUSED_TEXT: &str = "this computer's service manager cannot confine the torrent helper";
/// Linux EROFS.
const EROFS: i32 = 30;

/// One fence the helper does not see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    /// The home folder is not a tmpfs of handed-over folders: the real home is visible.
    HomeVisible,
    /// `/usr` is not read-only.
    SystemWritable,
    /// `NoNewPrivs` is not 1.
    NewPrivileges,
    /// Seccomp is not in filter mode.
    NoSyscallFilter,
    /// A block device is visible in `/dev`.
    Devices,
}

impl Gap {
    pub fn word(self) -> &'static str {
        match self {
            Gap::HomeVisible => "home",
            Gap::SystemWritable => "system",
            Gap::NewPrivileges => "no-new-privileges",
            Gap::NoSyscallFilter => "seccomp",
            Gap::Devices => "devices",
        }
    }
}

/// What the helper saw (plain values: [`gaps`] is tested without a unit).
#[derive(Debug, Clone, Default)]
pub struct View {
    /// `/proc/self/mountinfo`.
    pub mountinfo: String,
    /// `/proc/self/status`.
    pub status: String,
    /// `$HOME`.
    pub home: std::path::PathBuf,
    /// The names in `$HOME`.
    pub home_entries: Vec<OsString>,
    /// The names `$HOME` may hold: the first component of each handed-over folder below it.
    pub allowed_entries: Vec<OsString>,
    /// The error number of creating a file in `/usr` (None: the create succeeded).
    pub usr_create_errno: Option<i32>,
    /// Block devices seen in `/dev`.
    pub block_devices: usize,
}

/// The mount point (unescaped) and the file-system type of each `mountinfo` line.
fn mounts(mountinfo: &str) -> impl Iterator<Item = (String, &str)> {
    mountinfo.lines().filter_map(|l| {
        let point = l.split(' ').nth(4)?;
        let fstype = l.split(" - ").nth(1)?.split(' ').next()?;
        Some((unescape(point), fstype))
    })
}

/// mountinfo escapes space, tab, newline and backslash as `\ooo`.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() {
            let oct = std::str::from_utf8(&b[i + 1..i + 4]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(oct, 8) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The file-system type of the mount that holds `p` (the last listed mount whose point is `p` or
/// an ancestor of it: later lines stack over earlier ones).
pub fn fstype_of<'a>(mountinfo: &'a str, p: &Path) -> Option<&'a str> {
    let mut best: Option<(usize, &str)> = None;
    for (point, fstype) in mounts(mountinfo) {
        let pp = Path::new(&point);
        if p.starts_with(pp) {
            let depth = pp.components().count();
            if best.is_none_or(|(d, _)| depth >= d) {
                best = Some((depth, fstype));
            }
        }
    }
    best.map(|(_, t)| t)
}

fn status_is(status: &str, key: &str, want: &str) -> bool {
    status.lines().any(|l| {
        l.strip_prefix(key)
            .and_then(|r| r.strip_prefix(':'))
            .is_some_and(|v| v.trim() == want)
    })
}

/// The fences missing from `v` (empty: confined).
pub fn gaps(v: &View) -> Vec<Gap> {
    let mut g = Vec::new();
    let home_ok = v.home.is_absolute()
        && fstype_of(&v.mountinfo, &v.home) == Some("tmpfs")
        && v.home_entries.iter().all(|e| v.allowed_entries.contains(e));
    if !home_ok {
        g.push(Gap::HomeVisible);
    }
    if v.usr_create_errno != Some(EROFS) {
        g.push(Gap::SystemWritable);
    }
    if !status_is(&v.status, "NoNewPrivs", "1") {
        g.push(Gap::NewPrivileges);
    }
    if !status_is(&v.status, "Seccomp", "2") {
        g.push(Gap::NoSyscallFilter);
    }
    if v.block_devices > 0 {
        g.push(Gap::Devices);
    }
    g
}

/// The first component below `home` of each folder (None for folders outside home).
pub fn allowed_entries(home: &Path, folders: &[&Path]) -> Vec<OsString> {
    folders
        .iter()
        .filter_map(|f| f.strip_prefix(home).ok())
        .filter_map(|rel| match rel.components().next() {
            Some(Component::Normal(n)) => Some(n.to_os_string()),
            _ => None,
        })
        .collect()
}

/// Look at this process's own view.
pub fn look(home: &Path, folders: &[&Path]) -> View {
    let usr_create_errno = {
        let probe = Path::new("/usr").join(format!(".atlas-torrent-probe-{}", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                None
            }
            Err(e) => Some(e.raw_os_error().unwrap_or(-1)),
        }
    };
    let block_devices = std::fs::read_dir("/dev")
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_block_device()))
                .count()
        })
        .unwrap_or(0);
    View {
        mountinfo: std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default(),
        status: std::fs::read_to_string("/proc/self/status").unwrap_or_default(),
        home: home.to_path_buf(),
        home_entries: match std::fs::read_dir(home) {
            Ok(d) => d.filter_map(Result::ok).map(|e| e.file_name()).collect(),
            // No home at all inside the tmpfs (nothing handed over below it): nothing to see.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            // Any other failure to look is not a confined home.
            Err(_) => vec![OsString::from("?")],
        },
        allowed_entries: allowed_entries(home, folders),
        usr_create_errno,
        block_devices,
    }
}

/// The one line written to [`REFUSED_FILE`] and stderr.
pub fn refusal_line(g: &[Gap]) -> String {
    let words: Vec<&str> = g.iter().map(|x| x.word()).collect();
    format!("{REFUSED_TEXT} (missing: {})", words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFINED_MOUNTS: &str = "\
22 1 259:2 / / ro,relatime shared:1 - ext4 /dev/root ro
30 22 0:40 / /home rw,nosuid,nodev - tmpfs tmpfs rw,mode=755
31 30 259:2 /home/p/Downloads /home/p/Downloads rw,relatime - ext4 /dev/root rw
32 30 259:2 /home/p/.local/state/atlas/torrent /home/p/.local/state/atlas/torrent rw - ext4 /dev/root rw
";
    const HOST_MOUNTS: &str = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/root rw
";
    const STATUS: &str = "Name:\tatlas-torrent\nNoNewPrivs:\t1\nSeccomp:\t2\nSeccomp_filters:\t1\n";

    fn confined() -> View {
        let home = Path::new("/home/p");
        View {
            mountinfo: CONFINED_MOUNTS.into(),
            status: STATUS.into(),
            home: home.into(),
            home_entries: vec!["Downloads".into(), ".local".into()],
            allowed_entries: allowed_entries(
                home,
                &[
                    Path::new("/home/p/Downloads"),
                    Path::new("/home/p/.local/state/atlas/torrent"),
                ],
            ),
            usr_create_errno: Some(EROFS),
            block_devices: 0,
        }
    }

    #[test]
    fn a_confined_view_has_no_gap() {
        assert_eq!(gaps(&confined()), vec![]);
    }

    #[test]
    fn the_real_home_is_a_gap() {
        // the mount fences silently absent (DL-SEC-01, user manager without user namespaces)
        let mut v = confined();
        v.mountinfo = HOST_MOUNTS.into();
        assert_eq!(gaps(&v), vec![Gap::HomeVisible]);
        // a tmpfs home that holds more than the handed-over folders (a live system's home)
        let mut v = confined();
        v.home_entries.push(".ssh".into());
        assert_eq!(gaps(&v), vec![Gap::HomeVisible]);
    }

    #[test]
    fn permission_is_not_read_only() {
        let mut v = confined();
        v.usr_create_errno = Some(13); // EACCES: ProtectSystem absent, /usr only root-owned
        assert_eq!(gaps(&v), vec![Gap::SystemWritable]);
        v.usr_create_errno = None;
        assert_eq!(gaps(&v), vec![Gap::SystemWritable]);
    }

    #[test]
    fn privileges_seccomp_and_devices_are_checked() {
        let mut v = confined();
        v.status = "NoNewPrivs:\t0\nSeccomp:\t0\n".into();
        v.block_devices = 2;
        assert_eq!(
            gaps(&v),
            vec![Gap::NewPrivileges, Gap::NoSyscallFilter, Gap::Devices]
        );
        assert!(refusal_line(&gaps(&v)).contains("seccomp devices"));
    }

    #[test]
    fn the_deepest_mount_holds_the_path() {
        assert_eq!(
            fstype_of(CONFINED_MOUNTS, Path::new("/home/p")),
            Some("tmpfs")
        );
        assert_eq!(
            fstype_of(CONFINED_MOUNTS, Path::new("/home/p/Downloads/x")),
            Some("ext4")
        );
        assert_eq!(fstype_of(HOST_MOUNTS, Path::new("/home/p")), Some("ext4"));
        assert_eq!(unescape("/a\\040b"), "/a b");
    }

    #[test]
    fn this_test_process_is_not_confined() {
        // cargo test runs in the developer's session: the helper would refuse to serve here.
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        assert!(!gaps(&look(&home, &[])).is_empty());
    }
}
