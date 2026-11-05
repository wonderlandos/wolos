// SPDX-License-Identifier: BSD-2-Clause
//! How the browser's start of the helper becomes a confined user service: `atlas-torrent launch`
//! runs `systemd-run --user` with the properties below, so the helper sees an empty home except the
//! downloads folder and its state folder, cannot gain privileges, has bounded memory and tasks, only
//! speaks Unix, IPv4 and IPv6, and makes only the system calls of an ordinary service. When no user
//! service manager answers, the helper does NOT run unconfined: `launch` fails and the browser's
//! row says the torrent helper could not start. When the manager starts the unit but cannot apply
//! its fences (no user namespaces for the account), the helper sees that from inside and refuses to
//! serve ([`crate::confined`]).
//!
//! Not set, because a user manager refuses them (measured, systemd 255 and 262): ProtectKernelModules,
//! ProtectKernelLogs, ProtectClock, CapabilityBoundingSet. `AF_NETLINK` is not allowed: neither the
//! engine (librqbit 9.1.0 with UPnP and the HTTP API off) nor its resolver reads interfaces or routes
//! through netlink (tests/swarm.rs and the e2e run with this list). `IPAddressDeny=` is accepted by a
//! user manager but not enforced, so the address fence is the helper's own ([`crate::trackers`]).
use std::path::Path;

/// The unit's name (one per account; a second launch while it runs is a no-op).
pub const UNIT: &str = "atlas-torrent";
/// Memory the helper may use.
pub const MEMORY_MAX: &str = "512M";
/// Threads and processes the helper may have.
pub const TASKS_MAX: u32 = 64;

/// The fixed properties of the unit (the folders, MemoryMax and TasksMax are added by [`argv`]).
pub const PROPERTIES: &[&str] = &[
    "ProtectSystem=strict",
    "ProtectHome=tmpfs",
    "PrivateTmp=yes",
    "PrivateDevices=yes",
    "PrivateUsers=yes",
    "NoNewPrivileges=yes",
    "ProtectKernelTunables=yes",
    "ProtectControlGroups=yes",
    "ProtectHostname=yes",
    "ProtectProc=invisible",
    "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
    "RestrictNamespaces=yes",
    "RestrictRealtime=yes",
    "RestrictSUIDSGID=yes",
    "LockPersonality=yes",
    "MemoryDenyWriteExecute=yes",
    "SystemCallArchitectures=native",
    "SystemCallFilter=@system-service",
    "SystemCallErrorNumber=EPERM",
    "UMask=0077",
];

/// The `systemd-run` command line for `exe serve ...` confined to `downloads` and `state`, with
/// the socket folder `run` writable.
pub fn argv(
    exe: &Path,
    downloads: &Path,
    state: &Path,
    run: &Path,
    serve_args: &[String],
) -> Vec<String> {
    let d = downloads.display();
    let s = state.display();
    let r = run.display();
    let mut v: Vec<String> = vec![
        "/usr/bin/systemd-run".into(),
        "--user".into(),
        "--quiet".into(),
        "--collect".into(),
        format!("--unit={UNIT}"),
        "--description=Atlas torrent downloads".into(),
    ];
    for p in PROPERTIES {
        v.push("-p".into());
        v.push((*p).into());
    }
    for p in [
        // the socket folder too: ProtectHome=tmpfs also covers /run/user, and only a bind makes
        // its mount point inside that tmpfs (ReadWritePaths= alone fails the unit, 226/NAMESPACE)
        format!("BindPaths={d} {s} {r}"),
        format!("ReadWritePaths={d} {s} {r}"),
        format!("MemoryMax={MEMORY_MAX}"),
        format!("TasksMax={TASKS_MAX}"),
    ] {
        v.push("-p".into());
        v.push(p);
    }
    v.push(exe.display().to_string());
    v.push("serve".into());
    v.extend(serve_args.iter().cloned());
    v
}

/// A folder may be handed to the unit only when it is absolute UTF-8 and has none of the
/// characters a property value cannot carry: systemd splits `BindPaths=` at spaces, reads `:` as
/// `source:destination`, and treats `\\` and quotes as escapes (DL-SEC-14).
pub fn fit(p: &Path) -> bool {
    let Some(s) = p.to_str() else {
        return false;
    };
    p.is_absolute()
        && !s
            .chars()
            .any(|c| c.is_control() || matches!(c, ' ' | ':' | '\\' | '"' | '\'' | '%'))
        && s.len() < 1024
}

/// What the browser's row says when a folder is not [`fit`].
pub const UNFIT_TEXT: &str = "its name has characters the service manager cannot carry (a space, a colon, a backslash, a quote or %)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_is_confined() {
        let a = argv(
            Path::new("/opt/atlas/atlas-torrent"),
            Path::new("/home/p/Downloads"),
            Path::new("/home/p/.local/state/atlas/torrent"),
            Path::new("/run/user/1000/atlas"),
            &[
                "--socket".into(),
                "/run/user/1000/atlas/torrent.sock".into(),
            ],
        );
        let j = a.join(" ");
        for must in [
            "--user",
            "ProtectSystem=strict",
            "ProtectHome=tmpfs",
            "PrivateUsers=yes",
            "BindPaths=/home/p/Downloads /home/p/.local/state/atlas/torrent /run/user/1000/atlas",
            "NoNewPrivileges=yes",
            "ProtectKernelTunables=yes",
            "ProtectControlGroups=yes",
            "ProtectHostname=yes",
            "ProtectProc=invisible",
            "RestrictRealtime=yes",
            "MemoryDenyWriteExecute=yes",
            "SystemCallFilter=@system-service",
            "SystemCallErrorNumber=EPERM",
            "MemoryMax=512M",
            "TasksMax=64",
            "/opt/atlas/atlas-torrent serve --socket",
        ] {
            assert!(j.contains(must), "{must}");
        }
        // DL-SEC-07: the families the lead listed, nothing more
        assert!(a.contains(&"RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6".to_string()));
        assert!(!j.contains("AF_NETLINK"));
        // DL-SEC-15: the service manager's own program, not one found through PATH
        assert_eq!(a[0], "/usr/bin/systemd-run");
        // properties a user manager refuses are not asked for
        for never in [
            "ProtectKernelModules",
            "ProtectKernelLogs",
            "ProtectClock",
            "CapabilityBoundingSet",
        ] {
            assert!(!j.contains(never), "{never}");
        }
    }

    #[test]
    fn folders_systemd_cannot_carry_are_not_handed_over() {
        assert!(fit(Path::new("/home/p/Downloads")));
        assert!(!fit(Path::new("/home/p/My Downloads")));
        assert!(!fit(Path::new("rel/x")));
        // DL-SEC-14: what systemd reads as source:destination or an escape
        assert!(!fit(Path::new("/home/p/a:b")));
        assert!(!fit(Path::new("/home/p/a\\b")));
        assert!(!fit(Path::new("/home/p/it's")));
        assert!(!fit(Path::new("/home/p/\"q\"")));
    }
}
