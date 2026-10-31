//! /proc facts Memory Saver needs: a process's cgroup, owner, parent, argv, and whether it is a
//! game or a session component that must never be touched. `proc` is the /proc root (tests: a
//! temp directory laid out the same way).
use std::path::Path;

/// Compositor, shell, sound server, portals: never moved, never limited.
pub const NEVER: [&str; 12] = [
    "Hyprland",
    "Xwayland",
    "wol-shell",
    "wol-realtime",
    "wol-lock",
    "pipewire",
    "pipewire-pulse",
    "wireplumber",
    "xdg-desktop-por",
    "hypridle",
    "dbus-daemon",
    "systemd",
];

/// Processes that only run games: Wine/Proton, Steam's runtime and launchers, gamescope.
const GAME_COMMS: [&str; 8] = [
    "wine",
    "wine64",
    "wineserver",
    "wine64-preloader",
    "wine-preloader",
    "gamescope",
    "pressure-vessel",
    "reaper",
];

pub fn cgroup(proc: &Path, pid: i32) -> Option<String> {
    let s = std::fs::read_to_string(proc.join(pid.to_string()).join("cgroup")).ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(str::to_owned)
}

pub fn comm(proc: &Path, pid: i32) -> String {
    std::fs::read_to_string(proc.join(pid.to_string()).join("comm"))
        .map(|s| s.trim_end().to_owned())
        .unwrap_or_default()
}

/// argv from /proc/<pid>/cmdline (NUL separated).
pub fn argv(proc: &Path, pid: i32) -> Vec<String> {
    std::fs::read(proc.join(pid.to_string()).join("cmdline"))
        .map(|raw| {
            raw.split(|b| *b == 0)
                .filter(|a| !a.is_empty())
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// (real uid, parent pid) from /proc/<pid>/status.
pub fn owner_and_parent(proc: &Path, pid: i32) -> Option<(u32, i32)> {
    let s = std::fs::read_to_string(proc.join(pid.to_string()).join("status")).ok()?;
    let field = |k: &str| {
        s.lines()
            .find_map(|l| l.strip_prefix(k))
            .and_then(|v| v.split_whitespace().next())
            .map(str::to_owned)
    };
    Some((field("Uid:")?.parse().ok()?, field("PPid:")?.parse().ok()?))
}

/// `/user.slice/user-<uid>.slice/session-<n>.scope` -> (uid, "session-<n>.scope").
pub fn session_scope(cg: &str) -> Option<(u32, &str)> {
    let rest = cg.strip_prefix("/user.slice/user-")?;
    let (uid, rest) = rest.split_once(".slice/")?;
    (rest.starts_with("session-") && rest.ends_with(".scope") && !rest.contains('/'))
        .then_some((uid.parse().ok()?, rest))
}

/// The app scope a cgroup belongs to: the path up to and including the first `app-*.scope`
/// component under a user slice (role children inside it are cut off).
pub fn app_scope(cg: &str) -> Option<&str> {
    if !cg.starts_with("/user.slice/") {
        return None;
    }
    let mut end = 0;
    for part in cg.split('/').skip(1) {
        end += 1 + part.len();
        if part.starts_with("app-") && part.ends_with(".scope") {
            return Some(&cg[..end]);
        }
    }
    None
}

/// A game: Steam marks every process it starts (`SteamGameId`/`SteamAppId`), Wine and gamescope
/// only run games here, and a Windows `.exe` is one too.
pub fn is_game(proc: &Path, pid: i32) -> bool {
    let c = comm(proc, pid);
    if GAME_COMMS.contains(&c.as_str()) || c.to_ascii_lowercase().ends_with(".exe") {
        return true;
    }
    std::fs::read(proc.join(pid.to_string()).join("environ")).is_ok_and(|env| {
        env.split(|b| *b == 0)
            .any(|kv| kv.starts_with(b"SteamGameId=") || kv.starts_with(b"SteamAppId="))
    })
}

/// Every pid under `proc` with its parent (one pass over /proc).
pub fn parents(proc: &Path) -> Vec<(i32, i32)> {
    let Ok(rd) = std::fs::read_dir(proc) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .filter_map(|pid| Some((pid, owner_and_parent(proc, pid)?.1)))
        .collect()
}

/// `root` and all its descendants, from a (pid, parent) table.
pub fn tree(root: i32, table: &[(i32, i32)]) -> Vec<i32> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        out.extend(table.iter().filter(|(_, pp)| *pp == p).map(|(c, _)| *c));
        i += 1;
    }
    out
}

/// systemd unit name for an app's scope: `app-wol-<app id>-<pid>.scope`.
pub fn scope_name(app: &str, pid: i32) -> String {
    format!("app-wol-{}-{pid}.scope", shared::memsaver::app_id(app))
}

/// `busctl` argv that makes the system manager adopt `pids` into a new delegated scope in the
/// user's slice, stopped with the login session (`BindsTo`).
pub fn adopt_args(name: &str, uid: u32, session: &str, pids: &[i32], app: &str) -> Vec<String> {
    let mut a: Vec<String> = [
        "--system",
        "call",
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
        "StartTransientUnit",
        "ssa(sv)a(sa(sv))",
    ]
    .map(String::from)
    .into();
    a.extend([name.into(), "fail".into(), "7".into()]);
    a.extend(["PIDs".into(), "au".into(), pids.len().to_string()]);
    a.extend(pids.iter().map(i32::to_string));
    a.extend(["Slice".into(), "s".into(), format!("user-{uid}.slice")]);
    a.extend(["Delegate".into(), "b".into(), "true".into()]);
    a.extend([
        "Description".into(),
        "s".into(),
        format!("{app} (WolOS app)"),
    ]);
    a.extend([
        "CollectMode".into(),
        "s".into(),
        "inactive-or-failed".into(),
    ]);
    a.extend(["BindsTo".into(), "as".into(), "1".into(), session.into()]);
    a.extend(["After".into(), "as".into(), "1".into(), session.into()]);
    a.push("0".into());
    a
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A fake /proc: (pid, ppid, uid, comm, cgroup, cmdline with spaces, environ).
    pub fn fake(
        name: &str,
        procs: &[(i32, i32, u32, &str, &str, &str, &str)],
    ) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("wol-proc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for (pid, ppid, uid, comm, cg, cmd, env) in procs {
            let p = d.join(pid.to_string());
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(p.join("cgroup"), format!("0::{cg}\n")).unwrap();
            std::fs::write(
                p.join("status"),
                format!("Name:\t{comm}\nPPid:\t{ppid}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .unwrap();
            std::fs::write(p.join("cmdline"), cmd.replace(' ', "\0")).unwrap();
            std::fs::write(p.join("environ"), env.replace(' ', "\0")).unwrap();
        }
        d
    }

    #[test]
    fn cgroup_paths() {
        assert_eq!(
            session_scope("/user.slice/user-1000.slice/session-2.scope"),
            Some((1000, "session-2.scope"))
        );
        for no in [
            "/user.slice/user-1000.slice/app-wol-code-5.scope",
            "/user.slice/user-1000.slice/session-2.scope/x",
            "/system.slice/wold.service",
            "/user.slice/user-x.slice/session-2.scope",
        ] {
            assert_eq!(session_scope(no), None, "{no}");
        }
        let s = "/user.slice/user-1000.slice/app-wol-code-5.scope";
        assert_eq!(app_scope(&format!("{s}/renderer")), Some(s));
        assert_eq!(app_scope(s), Some(s));
        let u = "/user.slice/user-1000.slice/user@1000.service/app.slice/app-glib-foot-9.scope";
        assert_eq!(app_scope(u), Some(u));
        assert_eq!(
            app_scope("/user.slice/user-1000.slice/session-2.scope"),
            None
        );
        assert_eq!(app_scope("/system.slice/app-x.scope"), None);
    }

    #[test]
    fn reads_processes_and_finds_games() {
        let d = fake(
            "read",
            &[
                (10, 1, 1000, "Hyprland", "/s", "Hyprland", ""),
                (
                    20,
                    10,
                    1000,
                    "code",
                    "/s",
                    "/opt/code/code --enable-x",
                    "HOME=/h",
                ),
                (
                    21,
                    20,
                    1000,
                    "code",
                    "/s",
                    "/opt/code/code --type=zygote",
                    "",
                ),
                (
                    22,
                    21,
                    1000,
                    "code",
                    "/s",
                    "/opt/code/code --type=renderer",
                    "",
                ),
                (
                    30,
                    10,
                    1000,
                    "hl2_linux",
                    "/s",
                    "./hl2",
                    "SteamGameId=220 HOME=/h",
                ),
                (31, 10, 1000, "Game.exe", "/s", "Z:\\game.exe", ""),
                (32, 10, 1000, "wineserver", "/s", "wineserver", ""),
            ],
        );
        assert_eq!(owner_and_parent(&d, 22), Some((1000, 21)));
        assert_eq!(comm(&d, 20), "code");
        assert_eq!(argv(&d, 21), vec!["/opt/code/code", "--type=zygote"]);
        assert_eq!(cgroup(&d, 20).as_deref(), Some("/s"));
        assert!(!is_game(&d, 20));
        assert!(is_game(&d, 30) && is_game(&d, 31) && is_game(&d, 32));
        let mut t = tree(20, &parents(&d));
        t.sort();
        assert_eq!(t, vec![20, 21, 22]);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn adoption_asks_systemd_for_a_delegated_scope_bound_to_the_session() {
        let name = scope_name("Code - OSS", 20);
        assert_eq!(name, "app-wol-code_-_oss-20.scope");
        let a = adopt_args(&name, 1000, "session-2.scope", &[20, 21], "code");
        let s = a.join(" ");
        assert!(
            s.starts_with("--system call org.freedesktop.systemd1"),
            "{s}"
        );
        assert!(
            s.contains("StartTransientUnit ssa(sv)a(sa(sv)) app-wol-code_-_oss-20.scope fail 7 PIDs au 2 20 21 Slice s user-1000.slice Delegate b true"),
            "{s}"
        );
        assert!(
            s.contains("BindsTo as 1 session-2.scope After as 1 session-2.scope"),
            "{s}"
        );
        // The property count matches the properties given.
        let props = [
            "PIDs",
            "Slice",
            "Delegate",
            "Description",
            "CollectMode",
            "BindsTo",
            "After",
        ];
        let n: usize = a[9].parse().unwrap();
        assert_eq!(n, props.len(), "{s}");
        assert!(a.ends_with(&["0".to_string()]));
    }
}
